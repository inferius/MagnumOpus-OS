//! Tabulka handlu procesu.
//!
//! Handle je 32bitova hodnota: dolnich 20 bitu = index + 1, hornich 12 bitu
//! = generace slotu. Generace se pri zavreni zvysi, takze stary handle po
//! znovupouziti slotu neukazuje na cizi objekt. Slot, ktery vycerpal vsechny
//! generace, se uz nikdy nepouzije (generace by pretekla a stary handle by
//! zacal znovu platit). Hodnota 0 neni nikdy platna.

use alloc::sync::Arc;
use alloc::vec::Vec;

use super::{Error, KernelObject, Rights};

const INDEX_BITS: u32 = 20;
const GENERATION_BITS: u32 = 32 - INDEX_BITS;
const MAX_GENERATION: u16 = (1 << GENERATION_BITS) - 1;
const MAX_HANDLES: usize = (1 << INDEX_BITS) - 1;

pub use abi::Handle;

fn make_handle(index: usize, generation: u16) -> Handle {
    Handle(((generation as u32) << INDEX_BITS) | (index as u32 + 1))
}

fn index_of(h: Handle) -> Option<usize> {
    (h.0 & ((1 << INDEX_BITS) - 1))
        .checked_sub(1)
        .map(|i| i as usize)
}

fn generation_of(h: Handle) -> u16 {
    (h.0 >> INDEX_BITS) as u16
}

struct Entry {
    object: Arc<dyn KernelObject>,
    rights: Rights,
}

struct Slot {
    generation: u16,
    entry: Option<Entry>,
}

#[derive(Default)]
pub struct HandleTable {
    slots: Vec<Slot>,
    free: Vec<usize>,
}

impl HandleTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(
        &mut self,
        object: Arc<dyn KernelObject>,
        rights: Rights,
    ) -> Result<Handle, Error> {
        // Objekty jadra (a jejich data) vznikaji na pozadani programu: kdyz
        // by volne pameti zbyla jen rezerva jadra, dalsi uz ne.
        if !crate::mm::phys::user_room(1) {
            return Err(Error::NoMemory);
        }
        let index = match self.free.pop() {
            Some(i) => i,
            None if self.slots.len() < MAX_HANDLES.min(abi::limits::PROCESS_MAX_HANDLES) => {
                self.slots.push(Slot {
                    generation: 0,
                    entry: None,
                });
                self.slots.len() - 1
            }
            None => return Err(Error::NoResources),
        };
        let slot = &mut self.slots[index];
        slot.entry = Some(Entry { object, rights });
        Ok(make_handle(index, slot.generation))
    }

    fn entry(&self, handle: Handle) -> Result<&Entry, Error> {
        let slot = index_of(handle)
            .and_then(|i| self.slots.get(i))
            .ok_or(Error::BadHandle)?;
        if slot.generation != generation_of(handle) {
            return Err(Error::BadHandle);
        }
        slot.entry.as_ref().ok_or(Error::BadHandle)
    }

    #[allow(dead_code, reason = "API pro syscally (milnik 4)")]
    pub fn rights(&self, handle: Handle) -> Result<Rights, Error> {
        self.entry(handle).map(|e| e.rights)
    }

    /// Objekt za handlem, pokud handle ma vsechna prava `required`.
    pub fn get(&self, handle: Handle, required: Rights) -> Result<Arc<dyn KernelObject>, Error> {
        let entry = self.entry(handle)?;
        if !entry.rights.contains(required) {
            return Err(Error::AccessDenied);
        }
        Ok(entry.object.clone())
    }

    /// Jako `get`, ale navic overi typ objektu.
    pub fn get_as<T: KernelObject>(
        &self,
        handle: Handle,
        required: Rights,
    ) -> Result<Arc<T>, Error> {
        let object: Arc<dyn core::any::Any + Send + Sync> = self.get(handle, required)?;
        object.downcast::<T>().map_err(|_| Error::WrongType)
    }

    /// Novy handle na stejny objekt s pravy `rights`, ktera musi byt
    /// podmnozinou prav puvodniho handlu. Vyzaduje pravo DUPLICATE.
    pub fn duplicate(&mut self, handle: Handle, rights: Rights) -> Result<Handle, Error> {
        let entry = self.entry(handle)?;
        if !entry.rights.contains(Rights::DUPLICATE) || !entry.rights.contains(rights) {
            return Err(Error::AccessDenied);
        }
        let object = entry.object.clone();
        self.insert(object, rights)
    }

    pub fn close(&mut self, handle: Handle) -> Result<(), Error> {
        self.take(handle).map(|_| ())
    }

    /// Odebere handle z tabulky a vrati objekt s pravy (presun handlu
    /// jinam, napr. do zpravy kanalu).
    pub fn take(&mut self, handle: Handle) -> Result<(Arc<dyn KernelObject>, Rights), Error> {
        self.entry(handle)?;
        let index = index_of(handle).unwrap();
        let slot = &mut self.slots[index];
        let entry = slot.entry.take().unwrap();
        if slot.generation < MAX_GENERATION {
            slot.generation += 1;
            self.free.push(index);
        }
        // Jinak slot vyradime: zustane prazdny a uz se nepouzije.
        Ok((entry.object, entry.rights))
    }

    /// Overi handle a jeho prava bez odebrani.
    pub fn check(&self, handle: Handle, required: Rights) -> Result<(), Error> {
        self.get(handle, required).map(|_| ())
    }
}
