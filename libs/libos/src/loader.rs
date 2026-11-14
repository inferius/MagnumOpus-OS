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

/// Text posledni chyby `dl*` volajiciho vlakna.
fn dl_error(i: &Interface) -> alloc::string::String {
    // SAFETY: funkce zavadece; vraci C retezec platny do dalsiho `dl*`
    // ve vlakne, nebo null.
    let p = unsafe { (i.dlerror)() };
    if p.is_null() {
        return alloc::string::String::from("neznama chyba");
    }
    let mut n = 0;
    // SAFETY: C retezec od zavadece.
    while unsafe { p.add(n).read() } != 0 {
        n += 1;
    }
    // SAFETY: `n` bajtu pred nulou.
    alloc::string::String::from_utf8_lossy(unsafe { core::slice::from_raw_parts(p, n) }).into_owned()
}

fn c_string(s: &str) -> alloc::vec::Vec<u8> {
    let mut v = alloc::vec::Vec::with_capacity(s.len() + 1);
    v.extend_from_slice(s.as_bytes());
    v.push(0);
    v
}

/// Handle knihovny z [`dlopen`] (nezavira se pri drop: [`dlclose`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Library(*mut u8);

// SAFETY: handle je jen cislo modulu u zavadece (ten si stav zamyka sam).
unsafe impl Send for Library {}
unsafe impl Sync for Library {}

/// `dlopen`: knihovna z `/system/lib` (jmeno nebo cela cesta),
/// `flags` = `abi::loader::rtld::*`. Chyba = text od zavadece (`dlerror`);
/// program bez zavadece = "program bez zavadece".
pub fn dlopen(name: &str, flags: i32) -> Result<Library, alloc::string::String> {
    let i = interface().ok_or_else(|| alloc::string::String::from("program bez zavadece"))?;
    let c = c_string(name);
    // SAFETY: funkce zavadece s C retezcem.
    let h = unsafe { (i.dlopen)(c.as_ptr(), flags) };
    if h.is_null() { Err(dl_error(i)) } else { Ok(Library(h)) }
}

/// `dlsym`: adresa symbolu `name` v knihovne a jejich zavislostech
/// (`None` = globalni rozsah); TLS = adresa ve volajicim vlakne.
pub fn dlsym(lib: Option<Library>, name: &str) -> Result<*mut u8, alloc::string::String> {
    let i = interface().ok_or_else(|| alloc::string::String::from("program bez zavadece"))?;
    let c = c_string(name);
    let h = lib.map_or(core::ptr::null_mut(), |l| l.0);
    // SAFETY: funkce zavadece s handlem a C retezcem.
    let p = unsafe { (i.dlsym)(h, c.as_ptr()) };
    if p.is_null() { Err(dl_error(i)) } else { Ok(p) }
}

/// `dlclose`: snizi pocitadlo (knihovna zustava nahrana).
pub fn dlclose(lib: Library) -> Result<(), alloc::string::String> {
    let i = interface().ok_or_else(|| alloc::string::String::from("program bez zavadece"))?;
    // SAFETY: funkce zavadece s handlem.
    if unsafe { (i.dlclose)(lib.0) } == 0 { Ok(()) } else { Err(dl_error(i)) }
}
