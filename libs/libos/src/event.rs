//! Event.

use crate::handle::object_type;
use crate::{Handle, sys};
use abi::{Result, Signals};

#[derive(Debug)]
pub struct Event(Handle);
object_type!(Event);

impl Event {
    pub fn create() -> Result<Self> {
        sys::event_create().map(|h| Self(Handle::from_raw(h)))
    }

    pub fn signal(&self) -> Result<()> {
        sys::object_signal(self.0.raw(), 0, Signals::SIGNALED.0)
    }

    pub fn reset(&self) -> Result<()> {
        sys::object_signal(self.0.raw(), Signals::SIGNALED.0, 0)
    }
}
