//! Vlastneny handle: pri zruseni se zavre.

use crate::sys;
use abi::{Result, Rights};

#[derive(Debug, PartialEq, Eq)]
pub struct Handle(u32);

impl Handle {
    /// Prevezme syrovou hodnotu handlu (od te chvile ho zavre Drop).
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn raw(&self) -> u32 {
        self.0
    }

    /// Preda vlastnictvi syrove hodnote (Drop handle nezavre).
    pub fn into_raw(self) -> u32 {
        let raw = self.0;
        core::mem::forget(self);
        raw
    }

    /// Novy handle na stejny objekt; `None` = stejna prava.
    pub fn duplicate(&self, rights: Option<Rights>) -> Result<Handle> {
        sys::handle_duplicate(self.0, rights.map(|r| r.0)).map(Handle)
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = sys::handle_close(self.0);
    }
}

/// Spolecne metody typu objektu (obalka nad Handle).
macro_rules! object_type {
    ($name:ident) => {
        impl $name {
            pub fn from_handle(h: $crate::Handle) -> Self {
                Self(h)
            }

            pub fn handle(&self) -> &$crate::Handle {
                &self.0
            }

            pub fn into_handle(self) -> $crate::Handle {
                self.0
            }
        }
    };
}
pub(crate) use object_type;
