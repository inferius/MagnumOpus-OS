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
    if !crate::mm::phys::user_room(crate::mm::KERNEL_STACK_SIZE / abi::vm::PAGE_SIZE) {
        return Err(Error::NoMemory);
    }
    let stack = KernelStack::new().ok_or(Error::NoMemory)?;
    // SAFETY: zasobnik je novy a patri jen tomuto vlaknu.
    let ctx = unsafe { Context::new(stack.top(), thread_start) };
    let cpu = percpu::try_this_cpu().map_or(0, |c| c.index);
    let t = Arc::new(Thread::new(
        name,
        Qos::Normal,
        Some(stack),
        ctx,
        None,
        cpu,
        Some(process.clone()),
    ));
    process.add_thread(&t)?;
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
        // SAFETY: planovac pred spustenim vlakna prepnul na adresni prostor
        // jeho procesu a nastavil zasobnik jadra pro navrat z userspace.
        unsafe { arch::enter_user(entry, stack, arg0, arg1) }
    }));
    enqueue(t.clone(), true);
    Ok(())
}

/// Pozada vlakno o ukonceni: probudi ho z cekani a pokud prave bezi na
/// jinem CPU, prerusi ho, aby pri navratu do userspace skoncilo.
pub fn request_kill(t: &Arc<Thread>) {
    t.set_kill();
    wake(t, t.wait_seq());
    if t.on_cpu.load(Ordering::Acquire)
        && let Some(cpu) = percpu::cpu(t.last_cpu.load(Ordering::Relaxed))
    {
        arch::send_reschedule(cpu.hw_id);
    }
}

/// Volano pred navratem do userspace (konec syscallu nebo preruseni
/// z userspace) se zakazanymi prerusenimi: ukoncovane vlakno skonci.
pub fn on_return_to_user() {
    let cpu = percpu::this_cpu();
    if cpu.current().kill_requested() {
        interrupts::enable();
        exit();
    }
}

/// Vyjimka CPU v userspace: zaloguje ji a ukonci proces.
/// `code` = cislo chyby podle ABI (viz `trap::user_fault`), `pc` = adresa
/// instrukce. Text "rip 0x..." v logu cte `cargo xtask` pri symbolizaci,
/// proto zustava stejny na vsech architekturach.
pub fn user_fault(code: u8, error_code: u64, pc: u64, addr: u64) -> ! {
    let me = super::current();
    let process = me
        .process
        .clone()
        .expect("vyjimka z userspace ve vlakne jadra");
    log::error!(
        "proces {} ({}), vlakno {}: vyjimka CPU {code}, kod {error_code:#x}, rip {pc:#x}, adresa {addr:#x}",
        process.id,
        process.name,
        me.name
    );
    drop(me);
    process.terminate(abi::process::EXIT_FAULT_BASE - code as i64);
    drop(process);
    interrupts::enable();
    exit()
}
