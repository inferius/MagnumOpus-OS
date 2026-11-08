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
//!   (i z obsluhy preruseni) nikdy nealokuje a nikdy se neztrati. Kazda
//!   registrace a nevyzvednuty paket se strhava z rozpoctu jadra jobu
//!   procesu, ktery port vytvoril (zadny pevny pocet); vycerpany rozpocet =
//!   `NO_RESOURCES`. Pakety lezi v blocich po 4 KiB (zadna velka souvisla
//!   alokace), prazdny port vrati prebytecne bloky.
//! - Objekt drzi na port jen slabou referenci: zavreny port registrace
//!   neudrzi a pakety pro nej se zahodi.
//! - Na port samotny registrovat nejde (`NOT_SUPPORTED`): doruceni by
//!   zamykalo tentyz port dvakrat.
//!
//! Poradi zamku: zamek objektu -> fronta portu -> signaly portu.

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;

use super::job::Job;
use super::job::Pool;
use super::{Error, KernelObject, ObjectCore, ObjectType, Rights, Signals};
use crate::sync::SpinLock;

/// Paketu v jednom bloku fronty (blok = 4 KiB).
const PER_CHUNK: usize = 4096 / core::mem::size_of::<Packet>();
/// Prazdna fronta si necha bloky aspon pro tolik paketu (a pro vsechny
/// rezervace), zbytek vrati alokatoru.
const KEEP_PACKETS: usize = 64;
/// Pamet jadra za jednu registraci: misto pro paket a zaznam cekani
/// v objektu.
pub const REGISTRATION_COST: u64 = 64;

#[derive(Debug, Clone, Copy)]
pub struct Packet {
    pub key: u64,
    pub signals: Signals,
    /// Trida odesilatele zpravy, ktera paket zpusobila (dedeni QoS).
    pub boost: Option<crate::sched::Qos>,
}

/// Fronta paketu v blocich. Bloky `chunks[..used]` drzi pakety (posledni
/// z nich je konec fronty), dalsi jsou prazdne pripravene bloky. Vlozeni
/// do pripraveneho mista nikdy nealokuje.
struct PacketQueue {
    chunks: VecDeque<Vec<Packet>>,
    used: usize,
    /// Index prvniho paketu v `chunks[0]`.
    head: usize,
    len: usize,
}

impl PacketQueue {
    const fn new() -> Self {
        Self {
            chunks: VecDeque::new(),
            used: 0,
            head: 0,
            len: 0,
        }
    }

    /// Kolik paketu se jeste vejde bez alokace.
    fn free_slots(&self) -> usize {
        let tail_free = match self.used {
            0 => 0,
            n => PER_CHUNK - self.chunks[n - 1].len(),
        };
        tail_free + (self.chunks.len() - self.used) * PER_CHUNK
    }

    /// Zajisti misto pro `n` dalsich paketu.
    fn reserve(&mut self, n: usize) -> Result<(), Error> {
        while self.free_slots() < n {
            self.chunks.try_reserve(1).map_err(|_| Error::NoMemory)?;
            let mut c = Vec::new();
            c.try_reserve_exact(PER_CHUNK).map_err(|_| Error::NoMemory)?;
            self.chunks.push_back(c);
        }
        Ok(())
    }

    /// Vlozi paket do pripraveneho mista (`reserve` predem).
    fn push(&mut self, p: Packet) {
        if self.used == 0 || self.chunks[self.used - 1].len() == PER_CHUNK {
            self.used += 1;
        }
        let c = &mut self.chunks[self.used - 1];
        debug_assert!(c.len() < c.capacity(), "paket bez rezervace");
        c.push(p);
        self.len += 1;
    }

    fn pop(&mut self) -> Option<Packet> {
        if self.len == 0 {
            return None;
        }
        let p = self.chunks[0][self.head];
        self.head += 1;
        self.len -= 1;
        if self.head == self.chunks[0].len() {
            if self.used == 1 {
                self.chunks[0].clear();
            } else {
                // Vycerpany plny blok jde na konec mezi pripravene (bez alokace).
                let mut c = self.chunks.pop_front().expect("blok fronty");
                c.clear();
                self.chunks.push_back(c);
                self.used -= 1;
            }
            self.head = 0;
        }
        Some(p)
    }

    /// Vrati pripravene bloky nad `keep` paketu.
    fn shrink(&mut self, keep: usize) {
        let keep_chunks = keep.div_ceil(PER_CHUNK).max(self.used);
        while self.chunks.len() > keep_chunks {
            self.chunks.pop_back();
        }
        self.chunks.shrink_to_fit();
    }
}

struct Queue {
    packets: PacketQueue,
    /// Zaregistrovana, jeste nedorucena cekani (maji rezervovane misto).
    armed: usize,
}

pub struct Port {
    core: ObjectCore,
    queue: SpinLock<Queue>,
    /// Job procesu, ktery port vytvoril: z jeho rozpoctu se strhava kazda
    /// registrace a nevyzvednuty paket.
    job: Option<Arc<Job>>,
}

impl Port {
    pub fn new(job: Option<Arc<Job>>) -> Arc<Self> {
        Arc::new(Self {
            core: ObjectCore::new(Signals::NONE),
            queue: SpinLock::new(Queue {
                packets: PacketQueue::new(),
                armed: 0,
            }),
            job,
        })
    }

    fn take_budget(&self) -> Result<(), Error> {
        match &self.job {
            Some(j) => j.try_charge(Pool::Kernel, REGISTRATION_COST),
            None => Ok(()),
        }
    }

    fn release_budget(&self, n: usize) {
        if let Some(j) = &self.job {
            j.uncharge(Pool::Kernel, n as u64 * REGISTRATION_COST);
        }
    }

    /// Rezervuje misto pro jeden paket (pred registraci cekani).
    pub(super) fn reserve(&self) -> Result<(), Error> {
        let mut q = self.queue.lock();
        self.take_budget()?;
        // Misto pro vsechny cekajici registrace + tuto.
        let extra = q.armed + 1;
        if q.packets.reserve(extra).is_err() {
            self.release_budget(1);
            return Err(Error::NoMemory);
        }
        q.armed += 1;
        Ok(())
    }

    /// Doruci paket do rezervovaneho mista (vola objekt pod svym zamkem).
    /// Strzeni zustava, dokud program paket nevyzvedne.
    pub(super) fn deliver(&self, key: u64, signals: Signals, boost: Option<crate::sched::Qos>) {
        let mut q = self.queue.lock();
        q.armed = q.armed.saturating_sub(1);
        q.packets.push(Packet { key, signals, boost });
        self.core.update_boost(Signals::NONE, Signals::READABLE, boost);
    }

    /// Zrusi rezervaci, ktera se nakonec nepouzila.
    pub(super) fn unreserve(&self) {
        let mut q = self.queue.lock();
        if q.armed > 0 {
            q.armed -= 1;
            self.release_budget(1);
        }
    }

    /// Vyjme nejstarsi paket (a vrati jeho strzeni).
    pub fn pop(&self) -> Option<Packet> {
        let mut q = self.queue.lock();
        let p = q.packets.pop();
        if p.is_some() {
            self.release_budget(1);
        }
        if q.packets.len == 0 {
            let keep = q.armed.max(KEEP_PACKETS);
            q.packets.shrink(keep);
            self.core.update(Signals::READABLE, Signals::NONE);
        }
        p
    }
}

impl Drop for Port {
    /// Zavreny port vrati vsechny nevyzvednute pakety a rezervace
    /// (registrace v objektech uz ho nenajdou - drzi jen slaby odkaz).
    fn drop(&mut self) {
        let q = self.queue.get_mut();
        let n = q.armed + q.packets.len;
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
