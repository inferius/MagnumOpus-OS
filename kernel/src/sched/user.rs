//! Vlakna procesu v userspace: vytvoreni, spusteni, ukonceni, chyby.

use abi::vm::{USER_MAX, USER_MIN};
use alloc::boxed::Box;
use alloc::sync::Arc;
use core::sync::atomic::Ordering;

use super::thread::Qos;
use super::{Thread, enqueue, exit, percpu, thread_start, wake};
use crate::arch::context::{Context, ThreadContext};
use crate::arch::{self, interrupts};
use crate::error::Error;
use crate::log;
use crate::mm::KernelStack;
use crate::object::process::Process;

/// Vytvori (nespustene) vlakno procesu.
pub fn create_user_thread(process: &Arc<Process>, name: &str) -> Result<Arc<Thread>, Error> {
    // Zasobnik jadra vlakna je pamet jadra, ale vlakna vytvari program:
    // nesmi sahnout do rezervy.
    if !crate::mm::phys::user_room(crate::mm::KERNEL_STACK_SIZE / crate::arch::paging::PAGE_SIZE) {
        return Err(Error::NoMemory);
    }
    // Vlakno a jeho zasobnik jadra plati job procesu, zvlast: zasobnik se
    // vraci uz pri konci vlakna, objekt az se zahodi posledni handle.
    let job = process.job();
    let charge = job.charge(crate::object::job::Pool::Kernel, crate::object::job::object_cost::<Thread>())?;
    let stack_charge = job.charge(crate::object::job::Pool::Kernel, crate::mm::KERNEL_STACK_SIZE)?;
    let stack = KernelStack::new().ok_or(Error::NoMemory)?;
    // SAFETY: zasobnik je novy a patri jen tomuto vlaknu.
    let ctx = unsafe { Context::new(stack.top(), thread_start) };
    let cpu = percpu::try_this_cpu().map_or(0, |c| c.index);
    let (class, rt) = process.default_qos();
    let t = Arc::new(Thread::new(
        name,
        Qos::Normal,
        Some(stack),
        ctx,
        None,
        cpu,
        Some(process.clone()),
    ));
    crate::object::KernelObject::core(&*t).set_charge(charge);
    t.set_stack_charge(stack_charge);
    process.add_thread(&t)?;
    // Trida z `PROCESS_SET`. Realtime nad kapacitu: vlakno pobezi jako
    // Interactive (proces se nezastavi kvuli vycerpanemu Realtime).
    if class != Qos::Normal
        && let Err(e) = super::set_thread_qos(&t, class, rt)
    {
        log::warn!("proces {} ({}): vlakno {name} nedostalo tridu {class:?} ({e}), bezi jako Interactive", process.id, process.name);
        let _ = super::set_thread_qos(&t, class.min(Qos::Interactive), None);
    }
    Ok(t)
}

/// Spusti vlakno procesu: v userspace zacne na `entry` se zasobnikem
/// `stack` a argumenty `arg0` (rdi), `arg1` (rsi). `first` = prvni vlakno
/// procesu (PROCESS_START), jinak musi proces uz bezet (THREAD_START).
pub fn start_user_thread(
    t: &Arc<Thread>,
    first: bool,
    entry: u64,
    stack: u64,
    arg0: u64,
    arg1: u64,
) -> Result<(), Error> {
    let process = t.process.as_ref().ok_or(Error::WrongType)?;
    // Navrat do userspace (`arch::enter_user`) na adresu mimo pamet procesu by vyvolal
    // vyjimku jeste v jadre: vstupni bod i zasobnik musi lezet v ni.
    if !(USER_MIN..USER_MAX).contains(&entry) || !(USER_MIN..=USER_MAX).contains(&stack) {
        return Err(Error::InvalidArgs);
    }
    if t.started
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(Error::BadState);
    }
    if let Err(e) = process.thread_starting(first) {
        t.started.store(false, Ordering::Release);
        return Err(e);
    }
    *t.entry.lock() = Some(Box::new(move || {
        // Vlakno pozastavene jeste pred prvnim vstupem stoji uz tady
        // (prvni vstup nejde pres `on_return_to_user`). Kontext pred
        // prvnim vstupem jeste neni (ramec vznikne az pri vstupu do
        // jadra), cekajici zapis se tu proto nepouzije.
        interrupts::disable();
        super::suspend::park_before_start();
        interrupts::enable();
        if super::current().kill_requested() {
            exit();
        }
        // SAFETY: planovac pred spustenim vlakna prepnul na adresni prostor
        // jeho procesu a nastavil zasobnik jadra pro navrat z userspace.
        unsafe { arch::enter_user(entry, stack, arg0, arg1) }
    }));
    enqueue(t.clone(), super::Enq::New);
    Ok(())
}

/// Spusti vlakno procesu s funkci jadra `f` misto vstupu do userspace
/// (testy jadra: zivot a konec vlakna procesu bez programu). Po `f` vlakno
/// skonci beznym `exit`. `first` jako u [`start_user_thread`].
pub fn start_user_thread_in_kernel(
    t: &Arc<Thread>,
    first: bool,
    f: impl FnOnce() + Send + 'static,
) -> Result<(), Error> {
    let process = t.process.as_ref().ok_or(Error::WrongType)?;
    if t.started
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(Error::BadState);
    }
    if let Err(e) = process.thread_starting(first) {
        t.started.store(false, Ordering::Release);
        return Err(e);
    }
    *t.entry.lock() = Some(Box::new(f));
    enqueue(t.clone(), super::Enq::New);
    Ok(())
}

/// Pozada vlakno o ukonceni: probudi ho z cekani a pokud prave bezi na
/// jinem CPU, prerusi ho, aby pri navratu do userspace skoncilo.
pub fn request_kill(t: &Arc<Thread>) {
    t.set_kill();
    wake(t, t.wait_seq());
    // Zaparkovane pozastavene vlakno: probuzeni pod zamkem `stop`.
    super::suspend::kill_wake(t);
    if t.on_cpu_after_update()
        && let Some(cpu) = percpu::cpu(t.last_cpu.load(Ordering::Relaxed))
    {
        arch::send_reschedule(cpu.hw_id);
    }
}

/// Volano pred navratem do userspace (konec syscallu nebo preruseni
/// z userspace) se zakazanymi prerusenimi: ukoncovane vlakno skonci.
pub fn on_return_to_user() {
    let cpu = percpu::this_cpu();
    // Pozastavene vlakno tu stoji (a pouzije cekajici zapis kontextu).
    super::suspend::park();
    if cpu.current().kill_requested() {
        interrupts::enable();
        exit();
    }
}

/// Nejvic ramcu zasobniku procesu ve vypisu pady.
const USER_FRAMES: usize = 16;

/// Vyjimka CPU v userspace: zaloguje ji (s ukazatelem zasobniku a zpetnou
/// stopou podle ukazatelu ramcu - programy se prekladaji s
/// `force-frame-pointers`) a ukonci proces.
/// `code` = cislo chyby podle ABI (viz `trap::user_fault`), `pc` = adresa
/// instrukce (v logu pod jmenem registru architektury, `arch::PC_NAME`).
/// Vyjimka v userspace pro kanaly vyjimek (viz `trap::user_exception`).
pub fn user_exception(code: u8, error_code: u64, pc: u64, addr: u64, access: u32) -> bool {
    // Duvod nevyreseneho vypadku (napr. za koncem objektu pameti).
    let reason = super::current().fault_reason.swap(0, core::sync::atomic::Ordering::Relaxed);
    let report = abi::exception::Report {
        kind: code as u32,
        access,
        reason,
        pc,
        addr,
        code: error_code,
        ..Default::default()
    };
    // Obsluha muze cekat dlouho (debugger): s prerusenimi jako syscall.
    interrupts::enable();
    let outcome = crate::object::exception::deliver(report);
    interrupts::disable();
    outcome == crate::object::exception::Outcome::Resume
}

/// Vypadek stranky v userspace (viz `trap::user_page_fault`).
pub fn user_page_fault(addr: u64, access: crate::mm::Access) -> bool {
    let Some(process) = super::current().process.clone() else {
        return false;
    };
    // Reseni muze alokovat a cekat na shootdown: s prerusenimi jako syscall.
    interrupts::enable();
    let result = process.address_space().resolve_fault(addr, access);
    interrupts::disable();
    // Pristup za konec zvetsitelneho objektu pameti: duvod pro zpravu
    // o vyjimce (C `SIGBUS`, ne `SIGSEGV`).
    let reason = match result {
        Err(crate::error::Error::OutOfRange) => abi::exception::reason::BEYOND_END,
        _ => abi::exception::reason::NONE,
    };
    super::current().fault_reason.store(reason, core::sync::atomic::Ordering::Relaxed);
    match result {
        Ok(()) => true,
        Err(_) if process.address_space().is_guard(addr) => {
            log::error!(
                "proces {} ({}): pristup do ochranne oblasti na {addr:#x} (preteceni zasobniku?)",
                process.id,
                process.name
            );
            false
        }
        Err(crate::error::Error::NoMemory) => {
            log::error!(
                "proces {} ({}): vypadek na {addr:#x}: nedostatek fyzicke pameti (jadro vycerpalo rezervu)",
                process.id,
                process.name
            );
            false
        }
        Err(_) => false,
    }
}

pub fn user_fault(code: u8, error_code: u64, pc: u64, addr: u64, sp: u64, fp: u64) -> ! {
    let me = super::current();
    let process = me
        .process
        .clone()
        .expect("vyjimka z userspace ve vlakne jadra");
    log::error!(
        "proces {} ({}), vlakno {}: vyjimka CPU {code}, kod {error_code:#x}, {} {pc:#x}, adresa {addr:#x}",
        process.id,
        process.name,
        me.name,
        arch::PC_NAME
    );
    // Zpetna stopa: zaznam ramce = (predchozi ramec, navratova adresa) na
    // obou architekturach. Cteni pres kontrolovane kopirovani z pameti
    // procesu (vadny ukazatel jen ukonci vypis).
    log::error!("  proces {} ({}): sp {sp:#x} fp {fp:#x}, zpetna stopa:", process.id, process.name);
    let mut frame = fp;
    for _ in 0..USER_FRAMES {
        if frame == 0 || frame % 8 != 0 {
            break;
        }
        let Ok(rec) = crate::syscall::read_user(&process, frame, 16) else {
            break;
        };
        let next = u64::from_le_bytes(rec[..8].try_into().unwrap());
        let ret = u64::from_le_bytes(rec[8..].try_into().unwrap());
        if ret == 0 {
            break;
        }
        log::error!("  proces {} ({}): ramec {ret:#018x}", process.id, process.name);
        if next <= frame {
            break;
        }
        frame = next;
    }
    drop(me);
    process.terminate(abi::process::EXIT_FAULT_BASE - code as i64);
    drop(process);
    interrupts::enable();
    exit()
}
