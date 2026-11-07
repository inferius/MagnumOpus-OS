//! Port: fronta udalosti pro cekani na libovolne mnozstvi objektu.
//!
//! Program zaregistruje u objektu jednorazove cekani (`OBJECT_WAIT_ASYNC`:
//! handle, port, klic, maska). Jakmile signaly objektu masku protnou, objekt
//! vlozi do portu paket (klic + signaly) a registrace zanikne. Program pak
//! vybira pakety (`PORT_WAIT`) a po obsluze objekt zaregistruje znovu.
//! Na rozdil od `OBJECT_WAIT_MANY` neni limit poctu objektu a cena udalosti
//! nezavisi na tom, kolik objektu se hlida.
//!
//! - Port ma signal `READABLE`, dokud v nem je paket (lze na nej cekat
//!   i pres `OBJECT_WAIT_ONE` / `OBJECT_WAIT_MANY`).
//! - Misto pro paket se rezervuje uz pri registraci, takze doruceni
//!   (i z obsluhy preruseni) nikdy nealokuje a nikdy se neztrati. Registraci
//!   a paketu muze byt nejvys `PORT_MAX_PENDING` v jednom portu a
//!   `PROCESS_MAX_PORT_PENDING` ve vsech portech procesu dohromady (pamet
//!   jadra); dalsi = `NO_RESOURCES`. Prazdna fronta vrati prebytecnou
//!   kapacitu.
//! - Objekt drzi na port jen slabou referenci: zavreny port registrace
//!   neudrzi a pakety pro nej se zahodi.
//! - Na port samotny registrovat nejde (`NOT_SUPPORTED`): doruceni by
//!   zamykalo tentyz port dvakrat.
//!
//! Poradi zamku: zamek objektu -> fronta portu -> signaly portu.

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};

use abi::limits::PROCESS_MAX_PORT_PENDING;
use abi::syscall::PORT_MAX_PENDING;

use super::{Error, KernelObject, ObjectCore, ObjectType, Rights, Signals};
use crate::sync::SpinLock;

/// Prazdna fronta si necha jen tolik mista, kolik potrebuji rezervace
/// (a aspon tolik), zbytek vrati alokatoru.
const KEEP_CAPACITY: usize = 16;

#[derive(Debug, Clone, Copy)]
pub struct Packet {
    pub key: u64,
    pub signals: Signals,
}

struct Queue {
    packets: VecDeque<Packet>,
    /// Zaregistrovana, jeste nedorucena cekani (maji rezervovane misto).
    armed: usize,
}

pub struct Port {
    core: ObjectCore,
    queue: SpinLock<Queue>,
    /// Kvota procesu, ktery port vytvoril (sdili ji vsechny jeho porty):
    /// pocet registraci + nevyzvednutych paketu.
    budget: Arc<AtomicUsize>,
}

impl Port {
    pub fn new(budget: Arc<AtomicUsize>) -> Arc<Self> {
        Arc::new(Self {
            core: ObjectCore::new(Signals::NONE),
            queue: SpinLock::new(Queue {
                packets: VecDeque::new(),
                armed: 0,
            }),
            budget,
        })
    }

    /// Vezme jedno misto z kvoty procesu.
    fn take_budget(&self) -> Result<(), Error> {
        let prev = self.budget.fetch_add(1, Ordering::Relaxed);
        if prev >= PROCESS_MAX_PORT_PENDING {
            self.budget.fetch_sub(1, Ordering::Relaxed);
            return Err(Error::NoResources);
        }
        Ok(())
    }

    fn release_budget(&self, n: usize) {
        if n > 0 {
            self.budget.fetch_sub(n, Ordering::Relaxed);
        }
    }

    /// Rezervuje misto pro jeden paket (pred registraci cekani).
    pub(super) fn reserve(&self) -> Result<(), Error> {
        let mut q = self.queue.lock();
        let total = q.armed + q.packets.len();
        if total >= PORT_MAX_PENDING {
            return Err(Error::NoResources);
        }
        self.take_budget()?;
        // Kapacita pro vsechny cekajici + tuto registraci (len + armed + 1).
        let extra = q.armed + 1;
        if q.packets.try_reserve(extra).is_err() {
            self.release_budget(1);
            return Err(Error::NoMemory);
        }
        q.armed += 1;
        Ok(())
    }

    /// Doruci paket do rezervovaneho mista (vola objekt pod svym zamkem).
    /// Misto v kvote zustava obsazene, dokud program paket nevyzvedne.
    pub(super) fn deliver(&self, key: u64, signals: Signals) {
        let mut q = self.queue.lock();
        q.armed = q.armed.saturating_sub(1);
        q.packets.push_back(Packet { key, signals });
        self.core.update(Signals::NONE, Signals::READABLE);
    }

    /// Zrusi rezervaci, ktera se nakonec nepouzila.
    pub(super) fn unreserve(&self) {
        let mut q = self.queue.lock();
        if q.armed > 0 {
            q.armed -= 1;
            self.release_budget(1);
        }
    }

    /// Vyjme nejstarsi paket (a vrati jeho misto kvote procesu).
    pub fn pop(&self) -> Option<Packet> {
        let mut q = self.queue.lock();
        let p = q.packets.pop_front();
        if p.is_some() {
            self.release_budget(1);
        }
        if q.packets.is_empty() {
            let keep = q.armed.max(KEEP_CAPACITY);
            if q.packets.capacity() > keep * 2 {
                q.packets.shrink_to(keep);
            }
            self.core.update(Signals::READABLE, Signals::NONE);
        }
        p
    }
}

impl Drop for Port {
    /// Zavreny port vrati kvote vsechny nevyzvednute pakety a rezervace
    /// (registrace v objektech uz ho nenajdou - drzi jen slaby odkaz).
    fn drop(&mut self) {
        let q = self.queue.get_mut();
        let n = q.armed + q.packets.len();
        self.release_budget(n);
    }
}

impl KernelObject for Port {
    fn core(&self) -> &ObjectCore {
        &self.core
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Port
    }

    fn default_rights(&self) -> Rights {
        Rights::BASIC | Rights::READ | Rights::WRITE
    }
}
