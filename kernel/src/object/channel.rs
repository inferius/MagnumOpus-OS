//! Kanal: obousmerna fronta zprav mezi dvema konci.
//!
//! Zprava = bajty (do 64 KiB) + handly (do 64). Handly se zpravou
//! presouvaji: odesilatel o ne prijde, prijemce je dostane do sve tabulky.
//! Kazdy konec ma vlastni frontu prichozich zprav.
//!
//! Signaly konce: `READABLE` (fronta neni prazdna), `PEER_CLOSED` (druhy
//! konec uz neexistuje; zpravy, ktere uz ve fronte jsou, jde dal cist).
//!
//! Pamet zpravy ve fronte se strhava z rozpoctu jadra jobu odesilatele
//! a vrati se, kdyz si ji prijemce precte (nebo s kanalem zanikne).
//! Limity poctu zprav a bajtu ve fronte jsou rizeni toku (`SHOULD_WAIT`).
//!
//! Poradi zamku: fronta -> signaly objektu -> planovac.

use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use abi::syscall::{CHANNEL_MAX_DATA, CHANNEL_MAX_HANDLES, CHANNEL_MAX_QUEUED};

use super::job::Charge;
use super::{Error, KernelObject, ObjectCore, ObjectType, Rights, Signals};
use crate::sync::SpinLock;

/// Zprava v kanalu.
pub struct Message {
    pub data: Vec<u8>,
    pub handles: Vec<(Arc<dyn KernelObject>, Rights)>,
    /// Trida odesilatele (`None` = jadro): prijemce se na ni docasne
    /// zvedne (dedeni QoS pres IPC).
    pub qos: Option<crate::sched::QosLoan>,
    /// Strzeni za zpravu z rozpoctu odesilatele (vrati se se zanikem zpravy).
    #[expect(dead_code, reason = "drzi strzeni, cte se jen v Drop")]
    pub charge: Charge,
}

impl Message {
    /// Zprava bez strzeni (posila jadro).
    pub fn new(data: Vec<u8>, handles: Vec<(Arc<dyn KernelObject>, Rights)>) -> Self {
        Self {
            data,
            handles,
            qos: None,
            charge: Charge::none(),
        }
    }

    /// Pamet jadra, kterou zprava ve fronte zabira.
    pub fn cost(data_len: usize, handles: usize) -> u64 {
        (data_len + handles * 24 + 64) as u64
    }
}

/// Fronta prichozich zprav jednoho konce.
#[derive(Default)]
struct Queue {
    msgs: VecDeque<Message>,
    /// Soucet bajtu dat ve fronte (rizeni toku bez prochazeni fronty).
    bytes: usize,
}

impl Queue {
    fn pop(&mut self) -> Option<Message> {
        let m = self.msgs.pop_front()?;
        self.bytes -= m.data.len();
        Some(m)
    }
}

pub struct Channel {
    core: ObjectCore,
    peer: SpinLock<Weak<Channel>>,
    queue: SpinLock<Queue>,
}

impl Channel {
    /// Vytvori dvojici propojenych koncu.
    pub fn create() -> (Arc<Self>, Arc<Self>) {
        let new = || {
            Arc::new(Self {
                core: ObjectCore::new(Signals::NONE),
                peer: SpinLock::new(Weak::new()),
                queue: SpinLock::new(Queue::default()),
            })
        };
        let (a, b) = (new(), new());
        *a.peer.lock() = Arc::downgrade(&b);
        *b.peer.lock() = Arc::downgrade(&a);
        (a, b)
    }

    /// Posle zpravu druhemu konci. Pri chybe se zprava (i s handly) zahodi.
    pub fn write(&self, msg: Message) -> Result<(), Error> {
        if msg.data.len() > CHANNEL_MAX_DATA || msg.handles.len() > CHANNEL_MAX_HANDLES {
            return Err(Error::InvalidArgs);
        }
        let peer = self.peer.lock().upgrade().ok_or(Error::PeerClosed)?;
        let mut queue = peer.queue.lock();
        // Rizeni toku: odesilatel nepredbehne prijemce neomezene.
        if queue.msgs.len() >= CHANNEL_MAX_QUEUED
            || queue.bytes + msg.data.len() > abi::limits::CHANNEL_MAX_QUEUED_BYTES
        {
            return Err(Error::ShouldWait);
        }
        queue.bytes += msg.data.len();
        let qos = msg.qos;
        queue.msgs.push_back(msg);
        peer.core.update_boost(Signals::NONE, Signals::READABLE, qos);
        Ok(())
    }

    /// Velikost dalsi zpravy ve fronte (bajty, handly).
    pub fn peek_size(&self) -> Option<(usize, usize)> {
        self.queue
            .lock()
            .msgs
            .front()
            .map(|m| (m.data.len(), m.handles.len()))
    }

    /// Vyjme dalsi zpravu, pokud se vejde do zadanych limitu. Jinak vrati
    /// `BufferTooSmall` a zprava zustane ve fronte.
    pub fn read(&self, data_cap: usize, handles_cap: usize) -> Result<Message, Error> {
        let mut queue = self.queue.lock();
        let Some(front) = queue.msgs.front() else {
            return Err(if self.core.signals().intersects(Signals::PEER_CLOSED) {
                Error::PeerClosed
            } else {
                Error::ShouldWait
            });
        };
        if front.data.len() > data_cap || front.handles.len() > handles_cap {
            return Err(Error::BufferTooSmall);
        }
        let msg = queue.pop().unwrap();
        if queue.msgs.is_empty() {
            self.core.update(Signals::READABLE, Signals::NONE);
        }
        Ok(msg)
    }

    /// Je `other` tento konec? (Konec kanalu nelze poslat sam sebou.)
    pub fn is_same(&self, other: &Arc<dyn KernelObject>) -> bool {
        core::ptr::addr_eq(self as *const Self, Arc::as_ptr(other))
    }

    /// Lezi ve fronte tohoto konce nejaky konec kanalu? Takovy konec se
    /// nesmi poslat dal (viz `syscall::channel::write` - zabrana cyklum).
    pub fn holds_channels(&self) -> bool {
        self.queue.lock().msgs.iter().any(|m| {
            m.handles
                .iter()
                .any(|(o, _)| o.object_type() == ObjectType::Channel)
        })
    }

    /// Je `other` druhy konec tohoto kanalu? Zprava by skoncila v jeho
    /// vlastni fronte: konec by drzel sam sebe a nikdy by se neuvolnil.
    pub fn is_peer(&self, other: &Arc<dyn KernelObject>) -> bool {
        self.peer
            .lock()
            .upgrade()
            .is_some_and(|p| core::ptr::addr_eq(Arc::as_ptr(&p), Arc::as_ptr(other)))
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        if let Some(peer) = self.peer.get_mut().upgrade() {
            peer.core.update(Signals::NONE, Signals::PEER_CLOSED);
        }
    }
}

impl KernelObject for Channel {
    fn core(&self) -> &ObjectCore {
        &self.core
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Channel
    }

    fn default_rights(&self) -> Rights {
        Rights::BASIC | Rights::READ | Rights::WRITE
    }
}
