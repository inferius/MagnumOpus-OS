//! Rozhrani zavadece `ld.so` (`abi::loader`, `docs/cs/dynamicke-linkovani.md`):
//! program spusteny pres zavadec (PIE) ho najde v auxv pri startu. Bez
//! zavadece (staticky program) je vse jako drive: [`interface`] = `None`.

use core::sync::atomic::{AtomicPtr, Ordering};

use abi::loader::{AT_MO_LOADER, ENTRY_MAGIC, Interface};

static INTERFACE: AtomicPtr<Interface> = AtomicPtr::new(core::ptr::null_mut());

/// Tabulka zavadece, bezi-li program pod nim.
pub fn interface() -> Option<&'static Interface> {
    let p = INTERFACE.load(Ordering::Acquire);
    // SAFETY: adresu zapsal `init` z auxv od zavadece; tabulka je staticka
    // data zavadece (zije do konce procesu) a ma aspon pole verze 1.
    unsafe { p.as_ref() }
}

/// Precte auxv od zavadece (vstup programu: `sp` = argc a auxv, `magic` =
/// [`ENTRY_MAGIC`]); jiny `magic` = program bez zavadece, nic se nedeje.
///
/// # Safety
///
/// Volat jen ze vstupu programu s registry, jak je predal zavadec nebo
/// jadro (bez zavadece je `magic` jine nez [`ENTRY_MAGIC`]).
pub unsafe fn init(sp: u64, magic: u64) {
    if magic != ENTRY_MAGIC || sp == 0 {
        return;
    }
    // SAFETY: rozlozeni SysV na zasobniku (`abi::auxv`) od spawn a zavadece.
    unsafe {
        let sp = sp as *const u64;
        let argc = sp.read();
        let mut p = sp.add(1 + argc as usize + 1);
        while p.read() != 0 {
            p = p.add(1);
        }
        p = p.add(1);
        loop {
            let (t, v) = (p.read(), p.add(1).read());
            if t == abi::auxv::AT_NULL {
                break;
            }
            if t == AT_MO_LOADER && v != 0 {
                INTERFACE.store(v as *mut Interface, Ordering::Release);
            }
            p = p.add(2);
        }
    }
}

/// Finalizatory modulu (pri normalnim konci programu; podruhe nic).
pub fn run_fini() {
    if let Some(i) = interface() {
        // SAFETY: funkce zavadece bez argumentu.
        unsafe { (i.run_fini)() };
    }
}

/// Novy blok TLS pro vlakno (`None` = program bez zavadece; `Some(0)` =
/// malo pameti).
pub(crate) fn thread_tls_new() -> Option<u64> {
    // SAFETY: funkce zavadece bez argumentu.
    interface().map(|i| unsafe { (i.thread_tls_new)() })
}

/// Uvolni blok TLS vlakna, ktere skoncilo.
pub(crate) fn thread_tls_free(tp: u64) {
    if let Some(i) = interface() {
        // SAFETY: `tp` vratil `thread_tls_new` a vlakno uz nebezi.
        unsafe { (i.thread_tls_free)(tp) };
    }
}
