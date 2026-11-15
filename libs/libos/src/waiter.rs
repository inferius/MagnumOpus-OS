//! Cekani na promenny pocet objektu (klienti sluzby) pres port, bez limitu
//! `OBJECT_WAIT_MANY` (`WAIT_MANY_MAX` = 64).
//!
//! Pravidlo (docs/cs/libos.md): sluzba s promennym poctem klientu neceka
//! pres `wait_many` nad seznamem klientu, ale pres [`Waiter`]:
//!
//! ```ignore
//! let w = Waiter::new()?;
//! let key = w.key();                      // jedinecny klic objektu
//! w.arm(chan.handle(), key, Signals::READABLE | Signals::PEER_CLOSED)?;
//! for (k, signals) in w.wait(deadline)? {  // pripravene objekty
//!     // ... obslouzit objekt s klicem k, pak ho znovu zaregistrovat:
//!     w.arm(chan.handle(), k, Signals::READABLE | Signals::PEER_CLOSED)?;
//! }
//! ```
//!
//! Registrace je jednorazova (paket = obnovit registraci po obsluze).
//! Klic se nikdy nepouzije znovu ([`Waiter::key`]): paket uz zavreneho
//! objektu (registrace mohla zustat cekat) tak nikdy nepatri jinemu.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use abi::{Error, Result, Signals};

use crate::{Handle, Port};

pub struct Waiter {
    port: Port,
    next: AtomicU64,
}

impl Waiter {
    pub fn new() -> Result<Self> {
        Ok(Waiter { port: Port::create()?, next: AtomicU64::new(1) })
    }

    /// Novy jedinecny klic (nikdy 0, nikdy znovu).
    pub fn key(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    /// Jednorazova registrace: az signaly objektu protnou `mask`, [`wait`]
    /// vrati `key`.
    ///
    /// [`wait`]: Waiter::wait
    pub fn arm(&self, h: &Handle, key: u64, mask: Signals) -> Result<()> {
        self.port.wait_async(h, key, mask)
    }

    /// Pripravene objekty: prvni paket ceka do `deadline` (ns od startu,
    /// `None` = bez limitu), dalsi uz pripravene se vyzvednou bez cekani.
    /// Prazdny vysledek = termin uplynul.
    pub fn wait(&self, deadline: Option<u64>) -> Result<Vec<(u64, Signals)>> {
        let mut out = Vec::new();
        match self.port.wait(deadline) {
            Ok(p) => out.push((p.key, Signals(p.signals))),
            Err(Error::TimedOut) => return Ok(out),
            Err(e) => return Err(e),
        }
        loop {
            match self.port.wait(Some(0)) {
                Ok(p) => out.push((p.key, Signals(p.signals))),
                Err(Error::TimedOut) => return Ok(out),
                Err(e) => return Err(e),
            }
        }
    }
}
