//! Vlastnosti adresniho prostoru procesu.
//!
//! Velikost stranky neni konstanta: je to vlastnost adresniho prostoru
//! (dnes vsude 4 KiB, na aarch64 muze byt 16 KiB). Zarovnani adres a delek
//! pro mapovani a rozsahy prostredku se ridi touto hodnotou. Mez je dana
//! v `abi::vm::MIN_PAGE_SIZE` a `abi::vm::MAX_PAGE_SIZE`.
//!
//! ```ignore
//! let page = libos::vm::page_size();
//! let len = data_len.next_multiple_of(page);
//! ```

use core::sync::atomic::{AtomicU64, Ordering};

use abi::vm::MIN_PAGE_SIZE;

use crate::{rt, sys};

/// Zjistena velikost stranky vlastniho procesu (0 = jeste nezjistena).
static PAGE_SIZE: AtomicU64 = AtomicU64::new(0);

/// Velikost stranky vlastniho adresniho prostoru v bajtech. Prvni volani se
/// zepta jadra (`VM_PAGE_SIZE`), dalsi vraci ulozenou hodnotu.
pub fn page_size() -> u64 {
    match PAGE_SIZE.load(Ordering::Relaxed) {
        0 => match sys::vm_page_size(rt::self_process()) {
            Ok(size) => {
                PAGE_SIZE.store(size, Ordering::Relaxed);
                size
            }
            // Bez handlu na sebe (pred startem behoveho prostredi) plati
            // nejmensi velikost a neuklada se; pri spatnem odhadu jadro
            // mapovani odmitne chybou zarovnani, pamet se neposkodi.
            Err(_) => MIN_PAGE_SIZE,
        },
        size => size,
    }
}

/// Velikost stranky adresniho prostoru jineho procesu (napr. potomka pri
/// nahravani programu - muze mit jinou nez rodic).
pub fn page_size_of(process: &crate::Process) -> abi::Result<u64> {
    sys::vm_page_size(process.handle().raw())
}
