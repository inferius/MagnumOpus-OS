//! Tabulka zavadece pro program (`abi::loader::Interface`): bloky TLS
//! novych vlaken, finalizatory, `dl_iterate_phdr`; a `r_debug` pro
//! debuggery (SVR4: adresa v `DT_DEBUG` programu).

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use abi::loader::{Interface, PhdrCallback, PhdrInfo, VERSION};
use abi::vm::{MAP_READ, MAP_WRITE};
use libos::sync::Mutex;

/// Sablona TLS jednoho modulu: (cislo modulu, posun od TP, adresa dat,
/// velikost dat).
pub struct TlsTemplate {
    pub module: u64,
    pub off: i64,
    pub src: u64,
    pub len: u64,
}

/// Staticke TLS (rozlozeni a sablony vsech modulu ze startu).
pub struct TlsState {
    /// Velikost bloku (staticke TLS a za nim DTV) zaokrouhlena na stranky.
    pub len: u64,
    /// Kde v bloku lezi DTV.
    pub dtv: u64,
    /// Kde v bloku lezi TP.
    pub tp: u64,
    pub templates: Vec<TlsTemplate>,
}

/// Modul pro `dl_iterate_phdr` a finalizatory.
pub struct ModInfo {
    pub base: u64,
    /// Rozsah modulu od baze (konec posledniho segmentu).
    pub span: u64,
    /// Jmeno jako C retezec (program = "").
    pub cname: Vec<u8>,
    pub phdr: u64,
    pub phnum: u16,
    pub tls: Option<(u64, i64)>,
    pub fini: Option<u64>,
    /// `DT_FINI_ARRAY`: (adresa uz s bazi, pocet).
    pub fini_array: (u64, u64),
}

pub struct State {
    pub tls: Option<TlsState>,
    /// Program prvni, pak knihovny v poradi nahrani.
    pub mods: Vec<ModInfo>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static FINI_DONE: AtomicBool = AtomicBool::new(false);

pub fn set_state(s: State) {
    *STATE.lock() = Some(s);
}

/// Novy blok TLS: kopie sablon, nuly jinde (rezerva), DTV (generace
/// a bloky modulu ze startu), v TCB ukazatel na DTV a na x86_64 ukazatel
/// sam na sebe. Vraci TP, 0 = malo pameti.
pub fn new_block(t: &TlsState) -> u64 {
    let Ok(block) = libos::vm::alloc(t.len, MAP_READ | MAP_WRITE) else { return 0 };
    let tp = block + t.tp;
    let dtv = block + t.dtv;
    // SAFETY: TCB (16 B u TP), data modulu a DTV lezi v bloku (rozlozeni
    // z `dynload::tls_layout`, DTV za nim); zdroj je sablona modulu v jeho
    // pameti.
    unsafe {
        (dtv as *mut u64).write(crate::tls::GENERATION.load(Ordering::Acquire));
        for m in &t.templates {
            let at = tp.wrapping_add_signed(m.off);
            core::ptr::copy_nonoverlapping(m.src as *const u8, at as *mut u8, m.len as usize);
            ((dtv + m.module * 8) as *mut u64).write(at);
        }
        ((tp + crate::tls::TCB_DTV) as *mut u64).write(dtv);
        if cfg!(target_arch = "x86_64") {
            (tp as *mut u64).write(tp);
        }
    }
    tp
}

/// Stav TLS bez modulu s TLS: blok jen s TCB a DTV (TP musi platit vzdy).
fn empty_tls() -> TlsState {
    TlsState { len: (64 + crate::tls::DTV_BYTES).next_multiple_of(libos::vm::page_size()), dtv: 64, tp: 32, templates: Vec::new() }
}

unsafe extern "C" fn thread_tls_new() -> u64 {
    match STATE.lock().as_ref().and_then(|s| s.tls.as_ref()) {
        Some(t) => new_block(t),
        None => new_block(&empty_tls()),
    }
}

unsafe extern "C" fn thread_tls_free(tp: u64) {
    let (len, off) = match STATE.lock().as_ref().and_then(|s| s.tls.as_ref()) {
        Some(t) => (t.len, t.tp),
        None => {
            let e = empty_tls();
            (e.len, e.tp)
        }
    };
    let _ = libos::vm::release(tp - off, len);
}

/// Finalizatory: program prvni, zavislosti posledni (opak init);
/// `DT_FINI_ARRAY` pozpatku, pak `DT_FINI`.
unsafe extern "C" fn run_fini() {
    if FINI_DONE.swap(true, Ordering::AcqRel) {
        return;
    }
    // Seznam se zkopiruje: finalizator smi volat `dl_iterate_phdr`.
    let list: Vec<(Option<u64>, (u64, u64))> = match STATE.lock().as_ref() {
        Some(s) => s.mods.iter().map(|m| (m.fini, m.fini_array)).collect(),
        None => return,
    };
    type Fini = extern "C" fn();
    for (fini, (arr, n)) in list {
        for i in (0..n).rev() {
            // SAFETY: pole ukazatelu modulu (relokovane zavadecem).
            let f = unsafe { ((arr + i * 8) as *const u64).read_unaligned() };
            if f != 0 && f != u64::MAX {
                // SAFETY: finalizator modulu.
                let f: Fini = unsafe { core::mem::transmute(f as usize) };
                f();
            }
        }
        if let Some(f) = fini {
            // SAFETY: `DT_FINI` modulu (adresa uz s bazi).
            let f: Fini = unsafe { core::mem::transmute(f as usize) };
            f();
        }
    }
}

use crate::tls::tp;

/// Duvod posledni chyby `dl*` (C retezec) nebo null.
static DL_ERROR: AtomicU64 = AtomicU64::new(0);
static NOT_SUPPORTED: &[u8] = b"dlopen: zatim nepodporovano (sluzba knihoven libd v DL-4)\0";

unsafe extern "C" fn dlopen(_path: *const u8, _flags: i32) -> *mut u8 {
    DL_ERROR.store(NOT_SUPPORTED.as_ptr() as u64, Ordering::Release);
    core::ptr::null_mut()
}

unsafe extern "C" fn dlsym(_handle: *mut u8, _name: *const u8) -> *mut u8 {
    DL_ERROR.store(NOT_SUPPORTED.as_ptr() as u64, Ordering::Release);
    core::ptr::null_mut()
}

unsafe extern "C" fn dlclose(_handle: *mut u8) -> i32 {
    DL_ERROR.store(NOT_SUPPORTED.as_ptr() as u64, Ordering::Release);
    -1
}

pub unsafe extern "C" fn dlerror() -> *const u8 {
    DL_ERROR.swap(0, Ordering::AcqRel) as *const u8
}

/// `dladdr`: modul, v jehoz segmentech adresa lezi (jmeno a baze); symbol
/// zatim ne (`sname` null).
pub unsafe extern "C" fn dladdr(addr: *const u8, info: *mut abi::loader::DlInfo) -> i32 {
    let a = addr as u64;
    let g = STATE.lock();
    let Some(s) = g.as_ref() else { return 0 };
    for m in &s.mods {
        if a >= m.base && a < m.base + m.span {
            // SAFETY: `info` od volajiciho (Dl_info).
            unsafe {
                info.write(abi::loader::DlInfo { fname: m.cname.as_ptr(), fbase: m.base, sname: core::ptr::null(), saddr: 0 });
            }
            return 1;
        }
    }
    0
}

pub unsafe extern "C" fn dl_iterate_phdr(cb: PhdrCallback, data: *mut u8) -> i32 {
    // Bez alokace (alokator zavadece neuvolnuje; unwinder vola casto):
    // kazdy zaznam se zkopiruje pod zamkem, callback bezi bez zamku (smi
    // volat dalsi funkce zavadece).
    let mut i = 0;
    loop {
        let info = {
            let g = STATE.lock();
            let Some(s) = g.as_ref() else { return 0 };
            let Some(m) = s.mods.get(i) else { return 0 };
            PhdrInfo {
                addr: m.base,
                name: m.cname.as_ptr(),
                phdr: m.phdr as *const u8,
                phnum: m.phnum,
                adds: s.mods.len() as u64,
                subs: 0,
                tls_modid: m.tls.map_or(0, |t| t.0),
                tls_data: m.tls.map_or(core::ptr::null_mut(), |t| tp().wrapping_add_signed(t.1) as *mut u8),
            }
        };
        let mut info = info;
        // SAFETY: callback od programu s jeho `data`.
        let r = unsafe { cb(&mut info, size_of::<PhdrInfo>(), data) };
        if r != 0 {
            return r;
        }
        i += 1;
    }
}

pub static INTERFACE: Interface = Interface {
    version: VERSION,
    size: size_of::<Interface>() as u32,
    thread_tls_new,
    thread_tls_free,
    run_fini,
    dl_iterate_phdr,
    dlopen,
    dlsym,
    dlclose,
    dlerror,
    dladdr,
};

/// Vestavene funkce zavadece pro kod v C (jmeno -> adresa).
pub fn builtin(name: &str) -> Option<u64> {
    Some(match name {
        "dl_iterate_phdr" => dl_iterate_phdr as *const () as u64,
        "dlopen" => dlopen as *const () as u64,
        "dlsym" => dlsym as *const () as u64,
        "dlclose" => dlclose as *const () as u64,
        "dlerror" => dlerror as *const () as u64,
        "dladdr" => dladdr as *const () as u64,
        _ => return None,
    })
}

/// `struct link_map` (SVR4).
#[repr(C)]
struct LinkMap {
    addr: u64,
    name: *const u8,
    ld: u64,
    next: *mut LinkMap,
    prev: *mut LinkMap,
}

/// `struct r_debug` (SVR4).
#[repr(C)]
struct RDebug {
    version: i32,
    map: *mut LinkMap,
    brk: u64,
    state: i32,
    ldbase: u64,
}

/// Debugger si da breakpoint sem; zavadec ji vola pri zmene seznamu
/// modulu (`dlopen`).
#[inline(never)]
extern "C" fn dl_debug_state() {
    // SAFETY: prazdna instrukce (funkce nesmi zmizet ani splynout s jinou).
    unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
}

static R_DEBUG: AtomicU64 = AtomicU64::new(0);

/// Sestavi `r_debug` se seznamem modulu (jmeno, baze, `_DYNAMIC`) a vrati
/// jeho adresu (pro `DT_DEBUG` programu).
pub fn build_r_debug(mods: &[(u64, *const u8, u64)], ldbase: u64) -> u64 {
    let mut prev: *mut LinkMap = core::ptr::null_mut();
    let mut first: *mut LinkMap = core::ptr::null_mut();
    for &(addr, name, ld) in mods {
        let node = Box::into_raw(Box::new(LinkMap { addr, name, ld, next: core::ptr::null_mut(), prev }));
        if prev.is_null() {
            first = node;
        } else {
            // SAFETY: `prev` je predchozi uzel z Box::into_raw (nikdy se neuvolni).
            unsafe { (*prev).next = node };
        }
        prev = node;
    }
    let r = Box::into_raw(Box::new(RDebug { version: 1, map: first, brk: dl_debug_state as *const () as u64, state: 0, ldbase }));
    R_DEBUG.store(r as u64, Ordering::Release);
    dl_debug_state();
    r as u64
}
