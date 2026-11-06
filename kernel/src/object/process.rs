//! Proces: adresni prostor a tabulka handlu.
//!
//! Zatim bez vlaken v userspace (milnik 4); vlakna jadra k procesu nepatri.

use alloc::string::String;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};

use super::{Error, HandleTable, KernelObject, ObjectCore, ObjectType, Rights, Signals};
use crate::mm::AddressSpace;
use crate::sync::{SpinLock, SpinLockGuard};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub struct Process {
    core: ObjectCore,
    pub id: u64,
    pub name: String,
    handles: SpinLock<HandleTable>,
    space: AddressSpace,
}

impl Process {
    pub fn new(name: &str) -> Result<Arc<Self>, Error> {
        let space = AddressSpace::new().ok_or(Error::NoMemory)?;
        Ok(Arc::new(Self {
            core: ObjectCore::new(Signals::NONE),
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            name: String::from(name),
            handles: SpinLock::new(HandleTable::new()),
            space,
        }))
    }

    pub fn handles(&self) -> SpinLockGuard<'_, HandleTable> {
        self.handles.lock()
    }

    pub fn address_space(&self) -> &AddressSpace {
        &self.space
    }
}

impl KernelObject for Process {
    fn core(&self) -> &ObjectCore {
        &self.core
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Process
    }

    fn default_rights(&self) -> Rights {
        Rights::BASIC | Rights::MANAGE
    }
}
