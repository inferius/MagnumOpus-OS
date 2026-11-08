//! Namapovany objekt pameti ve vlastnim adresnim prostoru.

use abi::Result;

use crate::{Vmo, rt, sys};

mod sealed {
    pub trait Sealed {}
}

/// Typy, ktere lze bezpecne cist a zapisovat jako surove bajty pameti
/// (registry zarizeni, sdilena pamet): kazdy bitovy vzor je platna hodnota.
/// Jen cela cisla - `bool`, odkazy nebo vyctove typy by z libovolnych bajtu
/// vytvorily neplatnou hodnotu (nedefinovane chovani).
pub trait Pod: Copy + sealed::Sealed {}

macro_rules! pod {
    ($($t:ty),*) => {$(
        impl sealed::Sealed for $t {}
        impl Pod for $t {}
    )*};
}
pod!(u8, u16, u32, u64, i8, i16, i32, i64);

/// Rozsah vlastni pameti namapovany z objektu pameti; pri zruseni se
/// odmapuje (mapovani drzi objekt v jadre, handle uz neni potreba).
#[derive(Debug)]
pub struct Mapping {
    addr: u64,
    len: u64,
}

impl Mapping {
    /// Namapuje `len` bajtu `vmo` od `offset` s pravy `flags` (`abi::vm::MAP_*`).
    pub fn new(vmo: &Vmo, offset: u64, len: u64, flags: u32) -> Result<Self> {
        let len = len.next_multiple_of(crate::vm::page_size());
        let addr = sys::vm_map(
            rt::self_process(),
            vmo.handle().raw(),
            offset,
            len,
            flags,
            0,
        )?;
        Ok(Self { addr, len })
    }

    /// Zrusi mapovani a vrati vysledek (Drop chybu zahodi). Po odvolani
    /// objektu (`Resource::revoke`) uz mapovani neexistuje: `NOT_FOUND`.
    pub fn unmap(self) -> Result<()> {
        let r = sys::vm_unmap(rt::self_process(), self.addr, self.len);
        core::mem::forget(self);
        r
    }

    pub fn addr(&self) -> u64 {
        self.addr
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Obsah jako bajty.
    ///
    /// # Safety
    ///
    /// Mapovani nesmi byt registry zarizeni (cteni ma vedlejsi efekty) a nikdo
    /// jiny nesmi pamet soucasne menit.
    pub unsafe fn as_slice(&self) -> &[u8] {
        // SAFETY: predano volajicim; rozsah je namapovany po celou dobu zivota.
        unsafe { core::slice::from_raw_parts(self.addr as *const u8, self.len as usize) }
    }

    /// Overi, ze `len` bajtu od `offset` lezi v mapovani (bez preteceni).
    fn check(&self, offset: u64, len: u64, what: &str) {
        let end = offset.checked_add(len);
        assert!(end.is_some_and(|e| e <= self.len), "{what} mimo mapovani");
    }

    /// Precte hodnotu na offsetu (volatile - vhodne i pro registry).
    /// Offset musi byt zarovnany na velikost typu.
    pub fn read<T: Pod>(&self, offset: u64) -> T {
        self.check(offset, core::mem::size_of::<T>() as u64, "cteni");
        assert!(
            offset.is_multiple_of(core::mem::align_of::<T>() as u64),
            "nezarovnane cteni"
        );
        // SAFETY: rozsah a zarovnani overeny vyse (mapovani zacina na
        // strance); `T: Pod` = kazdy bitovy vzor je platna hodnota.
        unsafe { ((self.addr + offset) as *const T).read_volatile() }
    }

    /// Zapise hodnotu na offset (volatile). Offset musi byt zarovnany.
    pub fn write<T: Pod>(&self, offset: u64, value: T) {
        self.check(offset, core::mem::size_of::<T>() as u64, "zapis");
        assert!(
            offset.is_multiple_of(core::mem::align_of::<T>() as u64),
            "nezarovnany zapis"
        );
        // SAFETY: rozsah a zarovnani overeny vyse; zapis do mapovani jen pro
        // cteni by vyvolal vyjimku procesoru, ne nedefinovane chovani.
        unsafe { ((self.addr + offset) as *mut T).write_volatile(value) }
    }

    /// Zkopiruje bajty do mapovani.
    pub fn write_bytes(&self, offset: u64, data: &[u8]) {
        self.check(offset, data.len() as u64, "zapis");
        // SAFETY: rozsah overen vyse, zdroj je mimo mapovani.
        unsafe {
            core::ptr::copy_nonoverlapping(
                data.as_ptr(),
                (self.addr + offset) as *mut u8,
                data.len(),
            )
        }
    }

    /// Zkopiruje bajty z mapovani.
    pub fn read_bytes(&self, offset: u64, buf: &mut [u8]) {
        self.check(offset, buf.len() as u64, "cteni");
        // SAFETY: rozsah overen vyse, cil je mimo mapovani.
        unsafe {
            core::ptr::copy_nonoverlapping(
                (self.addr + offset) as *const u8,
                buf.as_mut_ptr(),
                buf.len(),
            )
        }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        let _ = sys::vm_unmap(rt::self_process(), self.addr, self.len);
    }
}
