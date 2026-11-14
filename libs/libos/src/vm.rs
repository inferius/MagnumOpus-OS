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

// --- Vlastni adresni prostor: podklad pro `mmap` v C knihovne a Wine ---
//
// Vsechny delky a adresy zarovnane na `page_size()`. Prava `prot` jsou
// `MAP_READ | MAP_WRITE | MAP_EXECUTE` (nikdy W+X; spusteni nezapecetene
// pameti jen s opravnenim JIT jobu). Anonymni pamet je objekt pameti, ktery
// drzi jen mapovani: stranky vznikaji pri prvnim zapisu, cteni vraci nuly.
//
// ```ignore
// let base = vm::reserve(1 << 30, 0)?;              // MEM_RESERVE
// vm::commit(base, 64 << 10, MAP_READ | MAP_WRITE)?; // MEM_COMMIT
// vm::decommit(base, 64 << 10)?;                     // MEM_DECOMMIT
// vm::release(base, 1 << 30)?;                       // MEM_RELEASE / munmap
// ```

/// Ochranna oblast pod zasobniky (spawn i vlakna): 64 KiB = nejvetsi
/// velikost stranky, jen rezervace (nestoji pamet).
pub const STACK_GUARD: u64 = abi::vm::MAX_PAGE_SIZE;

fn me() -> u32 {
    rt::self_process()
}

/// Zarezervuje `len` bajtu adres bez pameti (`abi::vm::RESERVE_GUARD` =
/// ochranna oblast; jinak 0). Vraci zacatek.
pub fn reserve(len: u64, flags: u32) -> abi::Result<u64> {
    sys::vm_reserve(me(), 0, len, flags)
}

/// Rezervace presne na `addr` (Wine: pevne adresy).
pub fn reserve_at(addr: u64, len: u64, flags: u32) -> abi::Result<u64> {
    sys::vm_reserve(me(), addr, len, flags | abi::vm::MAP_FIXED)
}

/// Anonymni pamet `len` bajtu s pravy `prot` na volnem miste
/// (`mmap(MAP_ANONYMOUS)`). Vraci adresu.
pub fn alloc(len: u64, prot: u32) -> abi::Result<u64> {
    let vmo = crate::Vmo::create(len)?;
    sys::vm_map(me(), vmo.handle().raw(), 0, len, prot, 0)
}

/// Anonymni pamet do rezervace na `addr` (`MEM_COMMIT`); rozsah musi
/// lezet cely v rezervacich.
pub fn commit(addr: u64, len: u64, prot: u32) -> abi::Result<()> {
    let vmo = crate::Vmo::create(len)?;
    let flags = prot | abi::vm::MAP_FIXED | abi::vm::MAP_IN_RESERVATION;
    sys::vm_map(me(), vmo.handle().raw(), 0, len, flags, addr).map(|_| ())
}

/// Zmeni prava rozsahu (`mprotect`; 0 = zadny pristup).
pub fn protect(addr: u64, len: u64, prot: u32) -> abi::Result<()> {
    sys::vm_protect(me(), addr, len, prot)
}

/// Pamet rozsahu zpet, adresy zustanou rezervovane (`MEM_DECOMMIT`).
pub fn decommit(addr: u64, len: u64) -> abi::Result<()> {
    sys::vm_unmap_flags(me(), addr, len, abi::vm::UNMAP_DECOMMIT)
}

/// Uvolni rozsah (pamet i rezervace; `munmap`, `MEM_RELEASE`). Smi byt
/// i cast oblasti.
pub fn release(addr: u64, len: u64) -> abi::Result<()> {
    sys::vm_unmap(me(), addr, len)
}

/// Oblast nebo volne misto na adrese `addr` (`VirtualQuery`).
pub fn query(addr: u64) -> abi::Result<abi::vm::RegionInfo> {
    sys::vm_query(me(), addr)
}

/// Zasobnik `len` bajtu z objektu `vmo` v procesu `process` s ochrannou
/// oblasti [`STACK_GUARD`] pod sebou (preteceni = konec procesu s hlaskou
/// misto prepsani cizi pameti). Vraci (zacatek rezervace, celkova delka,
/// vrchol zasobniku); celek se uvolni jednim `unmap(zacatek, delka)`.
pub fn map_stack(process: &crate::Process, vmo: &crate::Vmo, len: u64) -> abi::Result<(u64, u64, u64)> {
    let guard = STACK_GUARD.next_multiple_of(page_size_of(process)?);
    let total = guard + len;
    let base = process.reserve(0, total, abi::vm::RESERVE_GUARD)?;
    let flags = abi::vm::MAP_READ | abi::vm::MAP_WRITE | abi::vm::MAP_FIXED | abi::vm::MAP_IN_RESERVATION;
    if let Err(e) = process.map(vmo, 0, len, flags, base + guard) {
        let _ = process.unmap(base, total);
        return Err(e);
    }
    Ok((base, total, base + total))
}
