//! Kanal: obousmerna fronta zprav mezi dvema konci.
//!
//! Zprava = bajty (do 64 KiB) + handly (do 64). Handly se zpravou
//! presouvaji: odesilatel o ne prijde, prijemce je dostane do sve tabulky.
//! Kazdy konec ma vlastni frontu prichozich zprav.
//!
//! Signaly konce: `READABLE` (fronta neni prazdna), `PEER_CLOSED` (druhy
//! konec uz neexistuje; zpravy, ktere uz ve fronte jsou, jde dal cist).
//!
//! Poradi zamku: fronta -> signaly objektu -> planovac.

use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use abi::syscall::{CHANNEL_MAX_DATA, CHANNEL_MAX_HANDLES, CHANNEL_MAX_QUEUED};

use super::{Error, KernelObject, ObjectCore, ObjectType, Rights, Signals};
use crate::sync::SpinLock;

/// Zprava v kanalu.
pub struct Message {
    pub data: Vec<u8>,
    pub handles: Vec<(Arc<dyn KernelObject>, Rights)>,
}

pub struct Channel {
    core: ObjectCore,
    peer: SpinLock<Weak<Channel>>,
    queue: SpinLock<VecDeque<Message>>,
}

impl Channel {
    /// Vytvori dvojici propojenych koncu.
    pub fn create() -> (Arc<Self>, Arc<Self>) {
        let new = || {
            Arc::new(Self {
                core: ObjectCore::new(Signals::NONE),
                peer: SpinLock::new(Weak::new()),
                queue: SpinLock::new(VecDeque::new()),
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
        // Limit poctu zprav i bajtu: fronta jinak muze drzet az
        // 1024 x 64 KiB pameti jadra.
        let queued: usize = queue.iter().map(|m| m.data.len()).sum();
        if queue.len() >= CHANNEL_MAX_QUEUED
            || queued + msg.data.len() > abi::limits::CHANNEL_MAX_QUEUED_BYTES
        {
            return Err(Error::ShouldWait);
        }
        queue.push_back(msg);
        peer.core.update(Signals::NONE, Signals::READABLE);
        Ok(())
    }

    /// Velikost dalsi zpravy ve fronte (bajty, handly).
    pub fn peek_size(&self) -> Option<(usize, usize)> {
        self.queue
            .lock()
            .front()
            .map(|m| (m.data.len(), m.handles.len()))
    }

    /// Vyjme dalsi zpravu, pokud se vejde do zadanych limitu. Jinak vrati
    /// `BufferTooSmall` a zprava zustane ve fronte.
    pub fn read(&self, data_cap: usize, handles_cap: usize) -> Result<Message, Error> {
        let mut queue = self.queue.lock();
        let Some(front) = queue.front() else {
            return Err(if self.core.signals().intersects(Signals::PEER_CLOSED) {
                Error::PeerClosed
            } else {
                Error::ShouldWait
            });
        };
        if front.data.len() > data_cap || front.handles.len() > handles_cap {
            return Err(Error::BufferTooSmall);
        }
        let msg = queue.pop_front().unwrap();
        if queue.is_empty() {
            self.core.update(Signals::READABLE, Signals::NONE);
        }
        Ok(msg)
    }

    /// Je `other` tento konec? (Konec kanalu nelze poslat sam sebou.)
    pub fn is_same(&self, other: &Arc<dyn KernelObject>) -> bool {
        core::ptr::addr_eq(self as *const Self, Arc::as_ptr(other))
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
