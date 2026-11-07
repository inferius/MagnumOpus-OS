//! Preruseni zarizeni pro ovladac v userspace.
//!
//! Objekt dostane volne cislo preruseni (x86: vektor CPU, aarch64: LPI)
//! a vrati zpravu MSI (adresa + data), kterou
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

/// Rozsah cisel preruseni pro zarizeni (urcuje architektura).
const FIRST: u32 = arch::DEVICE_IRQ_FIRST;
const LAST: u32 = arch::DEVICE_IRQ_LAST;
const COUNT: usize = (LAST - FIRST + 1) as usize;

/// Objekty podle cisla preruseni (index = cislo - FIRST).
static TABLE: SpinLock<[Option<Weak<Interrupt>>; COUNT]> = SpinLock::new([const { None }; COUNT]);

pub struct Interrupt {
    core: ObjectCore,
    irq: u32,
    /// Zarizeni (segment, sbernice, zarizeni << 3 | funkce), pokud je zname.
    device: Option<(u16, u8, u8)>,
}

impl Interrupt {
    /// Novy objekt s volnym cislem preruseni pro zarizeni `device`.
    pub fn new(device: Option<(u16, u8, u8)>) -> Result<Arc<Self>, Error> {
        let mut table = TABLE.lock();
        let slot = (0..COUNT)
            .find(|&i| table[i].as_ref().is_none_or(|w| w.strong_count() == 0))
            .ok_or(Error::NoResources)?;
        let irq = Arc::new(Self {
            core: ObjectCore::new(Signals::NONE),
            irq: FIRST + slot as u32,
            device,
        });
        table[slot] = Some(Arc::downgrade(&irq));
        Ok(irq)
    }

    /// Zprava MSI pro zarizeni: (adresa, data). Chyba = architektura
    /// zpravu bez znalosti zarizeni vytvorit neumi (GICv3 ITS).
    pub fn msi_message(&self) -> Result<(u64, u32), Error> {
        arch::msi_message(self.irq, self.device)
    }

    /// Potvrzeni: smaze SIGNALED (dalsi preruseni ho znovu nastavi).
    pub fn ack(&self) {
        self.core.update(Signals::SIGNALED, Signals::NONE);
    }
}

impl Drop for Interrupt {
    fn drop(&mut self) {
        TABLE.lock()[(self.irq - FIRST) as usize] = None;
    }
}

/// Volano z obsluhy preruseni (`trap::device_irq`) pro cislo z rozsahu
/// zarizeni.
pub fn dispatch(irq: u32) {
    let obj = irq
        .checked_sub(FIRST)
        .and_then(|i| TABLE.lock().get(i as usize)?.as_ref()?.upgrade());
    match obj {
        Some(obj) => obj.core.update(Signals::NONE, Signals::SIGNALED),
        None => log::warn!("preruseni na vektoru {irq}, ktery nikdo nepouziva"),
    }
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
