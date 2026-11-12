//! Aplikace se `std` (cil magnumopus, docs/cs/rust-std.md): spusteni
//! `hello-std` s blokem parametru, stdout a stderr jako kanaly proudu
//! bajtu a vlastni relaci FS. Overuje std nad libos: argumenty
//! a prostredi, cas, `HashMap`, vlakna s `Mutex`/`Condvar`/TLS, soubory.
//! A primo syscally, na kterych std stoji: futex a registr vlakna (TLS).

use alloc::string::String;
use alloc::vec::Vec;
use alloc::{format, vec};
use core::sync::atomic::{AtomicU32, Ordering};
use core::time::Duration;

use libos::abi::params::tag;
use libos::initfs::Initfs;
use libos::params::Params;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Startup, sys, time};

use super::{Test, TestResult, devices, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("futex: wait, wake, vyprseni", futex),
    ("registr vlakna: vlastni hodnota kazdeho vlakna", thread_pointer),
    ("hello-std: argumenty, vlakna, TLS, soubory", hello_std),
];

/// Hodnota se nezmenila -> spani do terminu; jina hodnota -> hned zpet;
/// FUTEX_WAKE probudi cekajici vlakno.
fn futex(_: &Startup) -> TestResult {
    static WORD: AtomicU32 = AtomicU32::new(0);
    let start = time::now();
    let r = sys::futex_wait(&WORD, 0, time::deadline_after(Duration::from_millis(30)));
    check_eq!(r, Err(Error::TimedOut), "cekani bez probuzeni");
    check!(time::now() - start >= 30_000_000, "vyprseni pred terminem");
    check_eq!(sys::futex_wait(&WORD, 1, u64::MAX), Ok(()), "jina hodnota = hned zpet");
    check_eq!(sys::futex_wake(&WORD, 1), 0, "nikdo neceka");
    let waiter = libos::thread::spawn("futex-test", || {
        while WORD.load(Ordering::Acquire) == 0 {
            let _ = sys::futex_wait(&WORD, 0, time::deadline_after(Duration::from_secs(10)));
        }
    })
    .map_err(|e| fail("vlakno", e))?;
    // Vlakno nejspis uz spi; i kdyby ne, smycka podminku overi.
    time::sleep(Duration::from_millis(20));
    WORD.store(1, Ordering::Release);
    let _ = sys::futex_wake(&WORD, u32::MAX);
    waiter.join(Some(time::deadline_after(Duration::from_secs(5)))).map_err(|e| fail("probuzeni vlakna", e))?;
    check_eq!(sys::futex_wait(&WORD, 0, 0).err(), None, "hodnota 1");
    // Nezarovnana adresa: syrovy syscall (reference na nezarovnany
    // AtomicU32 by byla v Rustu nedefinovana).
    let addr = WORD.as_ptr() as u64 + 1;
    // SAFETY: jadro adresu jen zkontroluje a odmitne, nic necte ani nepise.
    let r = unsafe { sys::raw(libos::abi::syscall::FUTEX_WAIT, addr, 0, 0, 0, 0, 0) };
    check_eq!(libos::abi::error::decode(r).err(), Some(Error::InvalidArgs), "nezarovnana adresa");
    Ok(())
}

/// Precte registr ukazatele vlakna.
fn read_tp() -> u64 {
    let v: u64;
    #[cfg(target_arch = "x86_64")]
    // SAFETY: RDFSBASE neni povolene; TP ukazuje na slovo testu, ve
    // kterem je adresa sama (`fs:0` = TP), cteni platne pameti.
    unsafe {
        core::arch::asm!("mov {}, fs:0", out(reg) v, options(nostack, readonly, preserves_flags));
    }
    #[cfg(target_arch = "aarch64")]
    // SAFETY: cteni registru TPIDR_EL0 z EL0 je povolene.
    unsafe {
        core::arch::asm!("mrs {}, tpidr_el0", out(reg) v, options(nomem, nostack, preserves_flags));
    }
    v
}

/// Dve vlakna nastavi ruzne TP a po prepinani (yield, spani) kazde cte
/// svoje. Na x86_64 TP ukazuje na slovo s vlastni adresou (`fs:0`).
fn thread_pointer(_: &Startup) -> TestResult {
    fn run(slot: &'static mut [u64; 2]) -> Result<(), String> {
        let tp = slot.as_ptr() as u64;
        slot[0] = tp;
        sys::thread_set_pointer(tp).map_err(|e| fail("THREAD_SET_POINTER", e))?;
        for _ in 0..50 {
            sys::thread_yield();
            time::sleep(Duration::from_micros(200));
            let got = read_tp();
            if got != tp {
                return Err(format!("TP {got:#x} misto {tp:#x}"));
            }
        }
        // Init (no_std) TP jinak nepouziva: vratit 0.
        let _ = sys::thread_set_pointer(0);
        Ok(())
    }
    static mut SLOTS: [[u64; 2]; 2] = [[0; 2]; 2];
    // SAFETY: kazde vlakno dostane jiny prvek; po join uz je nikdo nepouziva.
    let (a, b) = unsafe { (&mut *(&raw mut SLOTS[0]), &mut *(&raw mut SLOTS[1])) };
    let other = libos::thread::spawn("tp-test", move || {
        if let Err(e) = run(b) {
            libos::println!("init: registr vlakna (druhe vlakno): {e}");
            libos::exit(98);
        }
    })
    .map_err(|e| fail("vlakno", e))?;
    let mine = run(a);
    other.join(Some(time::deadline_after(Duration::from_secs(10)))).map_err(|e| fail("join", e))?;
    mine?;
    check_eq!(sys::thread_set_pointer(u64::MAX).err(), Some(Error::InvalidArgs), "adresa mimo userspace");
    Ok(())
}

/// Vsechny zpravy z kanalu proudu az do jeho zavreni.
fn read_stream(ch: &Channel, deadline: u64) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        match ch.read(Some(deadline)) {
            Ok(m) => out.extend_from_slice(&m.data),
            Err(Error::PeerClosed) => return Ok(out),
            Err(e) => return Err(format!("cteni proudu: {e}; zatim: {}", String::from_utf8_lossy(&out))),
        }
    }
}

fn hello_std(s: &Startup) -> TestResult {
    let fs = devices().and_then(|d| d.fs.as_ref()).ok_or("sluzba fsd nebezi")?;
    let initfs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = initfs.find("hello-std").ok_or("hello-std neni v initfs")?;
    match fs.create("/std-test", fsproto::kind::DIR) {
        Ok(()) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(fail("adresar /std-test", e)),
    }
    let own = fs.connect().map_err(|e| fail("relace fsd", e))?;
    let params = Params {
        cwd: b"/".to_vec(),
        args: vec![b"hello-std".to_vec(), b"alfa".to_vec(), b"beta".to_vec()],
        env: vec![b"HELLO=svet".to_vec(), b"HELLO_DIR=/std-test".to_vec()],
    };
    let (out, out_child) = Channel::create().map_err(|e| fail("kanal stdout", e))?;
    let (err, err_child) = Channel::create().map_err(|e| fail("kanal stderr", e))?;
    let handles = vec![
        params.start_handle().map_err(|e| fail("blok parametru", e))?,
        StartHandle::tagged(tag::STDOUT, out_child.into_handle()),
        StartHandle::tagged(tag::STDERR, err_child.into_handle()),
        StartHandle::tagged(tag::FS, own.into_channel().into_handle()),
    ];
    let (process, _bootstrap) = libos::spawn("hello-std", image, handles).map_err(|e| fail("spusteni", e))?;
    let deadline = time::deadline_after(Duration::from_secs(120));
    let stdout = read_stream(&out, deadline)?;
    let code = process.wait_exit(Some(deadline)).map_err(|e| fail("konec procesu", e))?;
    let stderr = read_stream(&err, time::deadline_after(Duration::from_secs(1))).unwrap_or_default();
    let stdout = String::from_utf8_lossy(&stdout);
    let stderr = String::from_utf8_lossy(&stderr);
    for line in stdout.lines() {
        libos::println!("init: hello-std: {line}");
    }
    if code != 0 || !stdout.contains("hello-std: hotovo") {
        return Err(format!("kod {code}; stderr: {}; stdout: {}", stderr.trim(), stdout.trim()));
    }
    for name in ["args_env", "time", "hashmap", "threads", "files"] {
        check!(stdout.contains(&format!("ok {name}\n")), format!("chybi ok {name}"));
    }
    check!(stdout.contains("argumenty: alfa beta"), "argumenty ve vystupu");
    Ok(())
}
