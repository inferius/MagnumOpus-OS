//! Tabulka zavadece pro program (`abi::loader::Interface`): stav modulu
//! a TLS, bloky TLS vlaken, finalizatory, `dl_iterate_phdr`, `dladdr`;
//! a `r_debug` pro debuggery (SVR4: adresa v `DT_DEBUG` programu).

use alloc::boxed::Box;
use alloc::format;
use alloc::vec::Vec;
use core::alloc::Layout;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use abi::loader::{Interface, PhdrCallback, PhdrInfo, VERSION};
use abi::vm::{MAP_READ, MAP_WRITE};
use libos::Process;
use libos::sync::Mutex;

use crate::module::{Module, TlsPlace};
use crate::tls::{self, tp};

/// Sablona TLS modulu ve statickem bloku: (cislo modulu, posun od TP,
/// adresa dat, velikost dat).
pub struct TlsTemplate {
    pub module: u64,
    pub off: i64,
    pub src: u64,
    pub len: u64,
}

/// Modul s linym blokem TLS (`dlopen` bez initial-exec).
pub struct DynTls {
    pub module: u64,
    pub src: u64,
    pub len: u64,
    pub layout: Layout,
}

/// TLS procesu.
pub struct TlsState {
    /// Velikost bloku vlakna (staticke TLS a za nim DTV) zaokrouhlena na
    /// stranky; kde v bloku lezi DTV a TP.
    pub len: u64,
    pub dtv: u64,
    pub tp: u64,
    /// Moduly ve statickem TLS (start i `dlopen` v rezerve).
    pub templates: Vec<TlsTemplate>,
    pub dynamic: Vec<DynTls>,
    /// Rezerva pro moduly z `dlopen` s initial-exec.
    pub surplus: dynload::Surplus,
    /// TP zijicich bloku (`dlopen` jim doplni staticke TLS noveho modulu).
    pub threads: Vec<u64>,
    /// Dalsi cislo modulu TLS.
    pub next_id: u64,
}

pub struct State {
    /// Handle vlastniho procesu (mapovani knihoven z `dlopen`).
    pub process: Process,
    pub tls: TlsState,
    /// Program prvni, pak knihovny v poradi nahrani (nic se neodstranuje).
    pub mods: Vec<Module>,
    /// Globalni rozsah symbolu: program, knihovny ze startu, `RTLD_GLOBAL`.
    pub global: Vec<usize>,
}

impl State {
    pub fn find(&self, name: &str) -> Option<usize> {
        self.mods.iter().position(|m| m.is(name))
    }
}

pub static STATE: Mutex<Option<State>> = Mutex::new(None);
static FINI_DONE: AtomicBool = AtomicBool::new(false);

pub fn set_state(s: State) {
    *STATE.lock() = Some(s);
}

/// Novy blok TLS: kopie sablon statickych modulu, nuly jinde (rezerva),
/// DTV (generace a bloky statickych modulu; line bloky prazdne), v TCB
/// ukazatel na DTV a na x86_64 ukazatel sam na sebe. Vraci TP, 0 = malo
/// pameti.
pub fn new_block(t: &TlsState) -> u64 {
    let Ok(block) = libos::vm::alloc(t.len, MAP_READ | MAP_WRITE) else { return 0 };
    let tp = block + t.tp;
    let dtv = block + t.dtv;
    // SAFETY: TCB (16 B u TP), data modulu a DTV lezi v bloku (rozlozeni
    // z `dynload::tls_layout`, DTV za nim); zdroj je sablona modulu v jeho
    // pameti.
    unsafe {
        (dtv as *mut u64).write(tls::GENERATION.load(Ordering::Acquire));
        for m in &t.templates {
            let at = tp.wrapping_add_signed(m.off);
            core::ptr::copy_nonoverlapping(m.src as *const u8, at as *mut u8, m.len as usize);
            ((dtv + m.module * 8) as *mut u64).write(at);
        }
        ((tp + tls::TCB_DTV) as *mut u64).write(dtv);
        if cfg!(target_arch = "x86_64") {
            (tp as *mut u64).write(tp);
        }
    }
    tp
}

/// Linu blok modulu `m` pro vlakno s DTV `dtv` (pomala cesta
/// `__tls_get_addr` a resitele dynamickeho TLSDESC): kopie sablony, nuly
/// do konce, zapis do DTV. Neznamy modul nebo malo pameti = konec procesu
/// (jako glibc: pristup k TLS nema jak vratit chybu).
pub fn dynamic_block(m: u64, dtv: *mut u64) -> u64 {
    let g = STATE.lock();
    let Some(d) = g.as_ref().and_then(|s| s.tls.dynamic.iter().find(|d| d.module == m)) else {
        crate::fail(&format!("TLS neznameho modulu {m}"));
    };
    // SAFETY: layout ma nenulovou velikost (`dl::place_tls`).
    let p = unsafe { alloc::alloc::alloc_zeroed(d.layout) };
    if p.is_null() {
        crate::fail("pamet TLS modulu z dlopen");
    }
    // SAFETY: sablona modulu (`len` <= velikost bloku) do noveho bloku;
    // polozka DTV volajiciho vlakna (cislo modulu < MAX_MODULES).
    unsafe {
        core::ptr::copy_nonoverlapping(d.src as *const u8, p, d.len as usize);
        dtv.add(m as usize).write(p as u64);
    }
    p as u64
}

unsafe extern "C" fn thread_tls_new() -> u64 {
    let mut g = STATE.lock();
    let Some(s) = g.as_mut() else { return 0 };
    let tp = new_block(&s.tls);
    if tp != 0 {
        s.tls.threads.push(tp);
    }
    tp
}

unsafe extern "C" fn thread_tls_free(tp: u64) {
    {
        let mut g = STATE.lock();
        let Some(s) = g.as_mut() else { return };
        s.tls.threads.retain(|&t| t != tp);
        // SAFETY: blok vlakna, ktere skoncilo (od `thread_tls_new`).
        let dtv = unsafe { tls::dtv_of(tp) };
        for d in &s.tls.dynamic {
            // SAFETY: polozka DTV modulu; nenulova = blok z `dynamic_block`.
            unsafe {
                let p = dtv.add(d.module as usize).read();
                if p != 0 {
                    alloc::alloc::dealloc(p as *mut u8, d.layout);
                }
            }
        }
        let _ = libos::vm::release(tp - s.tls.tp, s.tls.len);
    }
    crate::dl::forget_thread(tp);
}

/// Spotreba rezervy statickeho TLS do logu (mereni pro nastaveni
/// `[app] static_tls_reserve`).
pub fn log_surplus(t: &TlsState, what: &str) {
    libos::sys::debug_log(&format!(
        "ld.so: {what}: rezerva statickeho TLS pouzito {} z {} B",
        t.surplus.used(),
        t.surplus.size()
    ));
}

/// Finalizatory: program prvni, zavislosti posledni (opak init);
/// `DT_FINI_ARRAY` pozpatku, pak `DT_FINI`. Moduly z `dlopen` jsou na konci
/// seznamu, takze prijdou na radu posledni (glibc: pred svymi zavislostmi;
/// tady jsou zavislosti nahrane drive, a tedy uz bez finalizatoru - jen
/// kdyz nove moduly zavisi na starsich).
unsafe extern "C" fn run_fini() {
    if FINI_DONE.swap(true, Ordering::AcqRel) {
        return;
    }
    // Seznam se zkopiruje: finalizator smi volat funkce zavadece.
    let list: Vec<(Option<u64>, (u64, u64))> = match STATE.lock().as_ref() {
        Some(s) => {
            if s.tls.surplus.used() > 0 {
                log_surplus(&s.tls, "konec procesu");
            }
            s.mods
                .iter()
                .map(|m| {
                    let fa = m.dynamic.fini_array;
                    (m.dynamic.fini.map(|f| m.base + f), (m.base + fa.addr, fa.size / 8))
                })
                .collect()
        }
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

/// `dladdr`: modul, v jehoz rozsahu adresa lezi (jmeno a baze); symbol
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
    // Kazdy zaznam se zkopiruje pod zamkem, callback bezi bez zamku (smi
    // volat dalsi funkce zavadece, i `dlopen`).
    let mut i = 0;
    loop {
        let info = {
            let g = STATE.lock();
            let Some(s) = g.as_ref() else { return 0 };
            let Some(m) = s.mods.get(i) else { return 0 };
            let tls_data = match m.tls_id {
                Some((_, TlsPlace::Static(off))) => tp().wrapping_add_signed(off),
                // SAFETY: DTV volajiciho vlakna; 0 = blok jeste neni.
                Some((id, TlsPlace::Dynamic)) => unsafe { tls::dtv_of(tp()).add(id as usize).read() },
                None => 0,
            };
            PhdrInfo {
                addr: m.base,
                name: m.cname.as_ptr(),
                phdr: (m.base + m.elf.phdr_vaddr().unwrap_or(0)) as *const u8,
                phnum: m.elf.phnum(),
                adds: s.mods.len() as u64,
                subs: 0,
                tls_modid: m.tls_id.map_or(0, |t| t.0),
                tls_data: tls_data as *mut u8,
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
    dlopen: crate::dl::dlopen,
    dlsym: crate::dl::dlsym,
    dlclose: crate::dl::dlclose,
    dlerror: crate::dl::dlerror,
    dladdr,
    libd_session,
};

/// Relace s libd a listek pro behove prostredi programu (`NARROW` na
/// listku zavadece).
unsafe extern "C" fn libd_session(session: *mut u32, lease: *mut u32) -> i32 {
    match libos::libd::child() {
        Ok((s, l)) => {
            // SAFETY: ukazatele od volajiciho (libos).
            unsafe {
                session.write(s.into_handle().into_raw());
                lease.write(l.into_handle().into_raw());
            }
            0
        }
        Err(e) => e.code(),
    }
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

/// `r_debug.state`: seznam se meni / je hotovy.
const RT_ADD: i32 = 1;
const RT_CONSISTENT: i32 = 0;

/// Debugger si da breakpoint sem; zavadec ji vola pri zmene seznamu
/// modulu (start, `dlopen`).
#[inline(never)]
extern "C" fn dl_debug_state() {
    // SAFETY: prazdna instrukce (funkce nesmi zmizet ani splynout s jinou).
    unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
}

static R_DEBUG: AtomicU64 = AtomicU64::new(0);
/// Posledni uzel seznamu (pripojovani pri `dlopen`).
static R_TAIL: AtomicU64 = AtomicU64::new(0);

/// Pripoji moduly (jmeno, baze, `_DYNAMIC`) k seznamu `r_debug` (prvni
/// volani ho zalozi) a vrati jeho adresu (pro `DT_DEBUG` programu). Uzly
/// se nikdy neuvolni (moduly se neodstranuji).
pub fn r_debug_add(mods: &[(u64, *const u8, u64)], ldbase: u64) -> u64 {
    let mut r = R_DEBUG.load(Ordering::Acquire) as *mut RDebug;
    if r.is_null() {
        r = Box::into_raw(Box::new(RDebug {
            version: 1,
            map: core::ptr::null_mut(),
            brk: dl_debug_state as *const () as u64,
            state: RT_CONSISTENT,
            ldbase,
        }));
        R_DEBUG.store(r as u64, Ordering::Release);
    }
    // SAFETY: `r` a uzly vznikly z Box::into_raw a nikdy se neuvolni;
    // zmeny jen pod zamkem `dlopen` (nebo pri startu v jednom vlakne).
    unsafe {
        (*r).state = RT_ADD;
        dl_debug_state();
        let mut prev = R_TAIL.load(Ordering::Acquire) as *mut LinkMap;
        for &(addr, name, ld) in mods {
            let node = Box::into_raw(Box::new(LinkMap { addr, name, ld, next: core::ptr::null_mut(), prev }));
            if prev.is_null() {
                (*r).map = node;
            } else {
                (*prev).next = node;
            }
            prev = node;
        }
        R_TAIL.store(prev as u64, Ordering::Release);
        (*r).state = RT_CONSISTENT;
    }
    dl_debug_state();
    r as u64
}
