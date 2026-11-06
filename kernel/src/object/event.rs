//! Event: objekt s jedinym signalem, ktery lze nastavit a shodit.

use alloc::sync::Arc;

use super::{KernelObject, ObjectCore, ObjectType, Rights, Signals};

pub struct Event {
    core: ObjectCore,
}

impl Event {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            core: ObjectCore::new(Signals::NONE),
        })
    }

    pub fn signal(&self) {
        self.core.update(Signals::NONE, Signals::SIGNALED);
    }

    pub fn reset(&self) {
        self.core.update(Signals::SIGNALED, Signals::NONE);
    }
}

impl KernelObject for Event {
    fn core(&self) -> &ObjectCore {
        &self.core
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Event
    }

    fn default_rights(&self) -> Rights {
        Rights::BASIC | Rights::SIGNAL
    }
}
