//! Preruseni zarizeni pro ovladac v userspace.
//!
//! Objekt dostane volny vektor CPU a vrati zpravu MSI (adresa + data), kterou
//! ovladac zapise do zarizeni (MSI nebo MSI-X). Kdyz zarizeni preruseni
//! vyvola, jadro jen nastavi signal SIGNALED; ovladac na nej ceka, potvrdi
//! ho (`ack` = smaze signal) a obslouzi zarizeni. Potvrzeni pred obsluhou
//! zaruci, ze se neztrati preruseni, ktere prijde behem obsluhy.
//!
//! Starsi linky preruseni (INTx pres IOAPIC) zatim nejsou potreba - vsechna
//! moderni zarizeni PCIe umi MSI.

use alloc::sync::{Arc, Weak};

use super::{Error, KernelObject, ObjectCore, ObjectType, Rights, Signals};
use crate::sync::SpinLock;
use crate::{arch, log};

/// Rozsah vektoru pro zarizeni (nize jsou vyjimky a vektory jadra, nahore
/// spurious).
const FIRST: usize = 48;
const LAST: usize = 239;

static TABLE: SpinLock<[Option<Weak<Interrupt>>; 256]> = SpinLock::new([const { None }; 256]);

pub struct Interrupt {
    core: ObjectCore,
    vector: u8,
}

impl Interrupt {
    /// Novy objekt s volnym vektorem.
    pub fn new() -> Result<Arc<Self>, Error> {
        let mut table = TABLE.lock();
        let vector = (FIRST..=LAST)
            .find(|&v| table[v].as_ref().is_none_or(|w| w.strong_count() == 0))
            .ok_or(Error::NoResources)?;
        let irq = Arc::new(Self {
            core: ObjectCore::new(Signals::NONE),
            vector: vector as u8,
        });
        table[vector] = Some(Arc::downgrade(&irq));
        Ok(irq)
    }

    /// Zprava MSI pro zarizeni: (adresa, data).
    pub fn msi_message(&self) -> (u64, u32) {
        arch::msi_message(self.vector)
    }

    /// Potvrzeni: smaze SIGNALED (dalsi preruseni ho znovu nastavi).
    pub fn ack(&self) {
        self.core.update(Signals::SIGNALED, Signals::NONE);
    }
}

impl Drop for Interrupt {
    fn drop(&mut self) {
        TABLE.lock()[self.vector as usize] = None;
    }
}

/// Volano z obsluhy preruseni pro vektor zarizeni.
pub fn dispatch(vector: u8) {
    let irq = TABLE.lock()[vector as usize]
        .as_ref()
        .and_then(Weak::upgrade);
    match irq {
        Some(irq) => irq.core.update(Signals::NONE, Signals::SIGNALED),
        None => log::warn!("preruseni na vektoru {vector}, ktery nikdo nepouziva"),
    }
}

/// Je vektor z rozsahu zarizeni?
pub fn is_device_vector(vector: u64) -> bool {
    (FIRST as u64..=LAST as u64).contains(&vector)
}

impl KernelObject for Interrupt {
    fn core(&self) -> &ObjectCore {
        &self.core
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Interrupt
    }

    fn default_rights(&self) -> Rights {
        Rights::BASIC | Rights::WRITE
    }
}
