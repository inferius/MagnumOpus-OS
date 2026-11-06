//! Namapovany objekt pameti ve vlastnim adresnim prostoru.

use abi::Result;

use crate::{Vmo, rt, sys};

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
        let len = len.next_multiple_of(abi::vm::PAGE_SIZE);
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

    /// Precte hodnotu na offsetu (volatile - vhodne i pro registry).
    pub fn read<T: Copy>(&self, offset: u64) -> T {
        assert!(
            offset + core::mem::size_of::<T>() as u64 <= self.len,
            "cteni mimo mapovani"
        );
        // SAFETY: rozsah overen vyse a je namapovany.
        unsafe { ((self.addr + offset) as *const T).read_volatile() }
    }

    /// Zapise hodnotu na offset (volatile).
    pub fn write<T: Copy>(&self, offset: u64, value: T) {
        assert!(
            offset + core::mem::size_of::<T>() as u64 <= self.len,
            "zapis mimo mapovani"
        );
        // SAFETY: rozsah overen vyse; zapis do mapovani jen pro cteni by
        // vyvolal vyjimku procesoru, ne nedefinovane chovani.
        unsafe { ((self.addr + offset) as *mut T).write_volatile(value) }
    }

    /// Zkopiruje bajty do mapovani.
    pub fn write_bytes(&self, offset: u64, data: &[u8]) {
        assert!(
            offset + data.len() as u64 <= self.len,
            "zapis mimo mapovani"
        );
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
        assert!(offset + buf.len() as u64 <= self.len, "cteni mimo mapovani");
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
