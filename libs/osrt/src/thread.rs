//! Vlakna pro `std::thread`.
//!
//! Zasobnik a start vlakna dela `libos::thread`; osrt prida blok TLS
//! ([`crate::tls`]) a nastavi ho jako prvni vec v novem vlakne, pred
//! jakymkoli kodem `std`.

use alloc::boxed::Box;
use core::time::Duration;

use crate::tls::Block;
use crate::{Error, Result};

/// Bezici vlakno. `join` pocka na konec a uvolni zasobnik a TLS; zahozeni
/// bez `join` vlakno odpoji (zasobnik a blok TLS pak zustanou do konce
/// procesu - vlakno je jeste muze pouzivat).
pub struct Thread {
    handle: Option<libos::thread::JoinHandle>,
    tls: Option<Block>,
}

/// Spusti `f` v novem vlakne se zasobnikem aspon `stack` bajtu.
pub fn spawn(stack: usize, f: Box<dyn FnOnce() + Send>) -> Result<Thread> {
    // Pod zavadecem ld.so dostane vlakno blok TLS vsech modulu od libos
    // (`abi::loader`); vlastni blok jednoho modulu by ho prepsal.
    if libos::loader::interface().is_some() {
        let handle = libos::thread::spawn_sized("std", stack as u64, f)?;
        return Ok(Thread { handle: Some(handle), tls: None });
    }
    let tls = Block::new().ok_or(Error::NoMemory)?;
    let tp = tls.thread_pointer();
    let handle = libos::thread::spawn_sized("std", stack as u64, move || {
        if libos::sys::thread_set_pointer(tp).is_err() {
            crate::debug_log("osrt: nelze nastavit ukazatel vlakna (TLS)");
            crate::abort();
        }
        f();
    })?;
    Ok(Thread { handle: Some(handle), tls: Some(tls) })
}

impl Thread {
    /// Pocka na konec vlakna.
    pub fn join(mut self) -> Result<()> {
        let handle = self.handle.take().ok_or(Error::BadState)?;
        handle.join(None)?;
        // Vlakno skoncilo: blok TLS uz nikdo nepouziva.
        drop(self.tls.take());
        Ok(())
    }
}

impl Drop for Thread {
    fn drop(&mut self) {
        // Odpojene vlakno muze dal bezet a pouzivat svuj blok TLS.
        if let Some(tls) = self.tls.take() {
            core::mem::forget(tls);
        }
    }
}

pub fn yield_now() {
    libos::sys::thread_yield();
}

pub fn sleep(d: Duration) {
    libos::time::sleep(d);
}

/// Usne do monotonniho casu `deadline_ns` (ns od startu).
pub fn sleep_until(deadline_ns: u64) {
    let _ = libos::sys::object_wait_many(&mut [], deadline_ns);
}

/// Pocet CPU (pro `available_parallelism`).
pub fn cpu_count() -> Option<usize> {
    libos::sched::cpu_info(0).ok().map(|(_, n)| n).filter(|&n| n > 0)
}
