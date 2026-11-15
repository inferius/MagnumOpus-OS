//! `dlopen`, `dlsym`, `dlclose`, `dlerror` (`abi::loader`,
//! `docs/cs/dynamicke-linkovani.md`).
//!
//! - `dlopen` nahraje knihovnu a jeji chybejici zavislosti od libd (jen
//!   `/system/lib`), rozdeli TLS (initial-exec do rezervy statickeho TLS,
//!   ostatni line pres DTV), relokuje je v rozsahu globalni rozsah +
//!   lokalni skupina knihovny, zapeceti kod a RELRO a zavola inicializatory.
//!   Chyba kdekoli = nic se nezmeni (nove moduly se zahodi) a duvod jde do
//!   `dlerror`.
//! - Moduly se nikdy neodstranuji (`dlclose` jen snizi pocitadlo): kod je
//!   zapeceteny a cisla modulu TLS se neopakuji.
//! - Volani `dl*` jsou serializovana rekurzivnim zamkem (inicializator
//!   smi volat `dlopen`); chyba `dlerror` je pro kazde vlakno zvlast.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::ToString;
use alloc::vec::Vec;
use core::alloc::Layout;
use core::sync::atomic::{AtomicU64, Ordering};

use abi::loader::rtld;
use libos::sync::{Mutex, MutexGuard};

use crate::iface::{self, DynTls, STATE, State, TlsTemplate};
use crate::module::{self, Found, Init, Res, TlsPlace};
use crate::tls::{self, tp};

/// Rekurzivni zamek `dl*` (vlastnik = TP vlakna).
static DL: Mutex<()> = Mutex::new(());
static OWNER: AtomicU64 = AtomicU64::new(0);

struct DlGuard(Option<MutexGuard<'static, ()>>);

fn dl_lock() -> DlGuard {
    let me = tp();
    if OWNER.load(Ordering::Acquire) == me {
        return DlGuard(None);
    }
    let g = DL.lock();
    OWNER.store(me, Ordering::Release);
    DlGuard(Some(g))
}

impl Drop for DlGuard {
    fn drop(&mut self) {
        if self.0.is_some() {
            OWNER.store(0, Ordering::Release);
        }
    }
}

/// Chyby vlaken: TP -> (cekajici chyba, posledni vracena). Vraceny retezec
/// musi platit do dalsiho volani `dl*` ve vlakne.
type Errors = BTreeMap<u64, (Option<Vec<u8>>, Vec<u8>)>;
static ERRORS: Mutex<Errors> = Mutex::new(BTreeMap::new());

fn set_error(msg: &str) {
    let mut v = msg.as_bytes().to_vec();
    v.push(0);
    ERRORS.lock().entry(tp()).or_default().0 = Some(v);
}

/// Vlakno skoncilo: jeho chyba pryc.
pub fn forget_thread(tp: u64) {
    ERRORS.lock().remove(&tp);
}

pub unsafe extern "C" fn dlerror() -> *const u8 {
    let mut g = ERRORS.lock();
    let Some(e) = g.get_mut(&tp()) else { return core::ptr::null() };
    match e.0.take() {
        Some(v) => {
            e.1 = v;
            e.1.as_ptr()
        }
        None => core::ptr::null(),
    }
}

/// C retezec jako `&str` (nejvys 4 KiB).
///
/// # Safety
///
/// `p` je platny C retezec od volajiciho.
unsafe fn c_str<'a>(p: *const u8) -> Res<&'a str> {
    let mut n = 0;
    // SAFETY: predano volajicim (konci nulou).
    while unsafe { p.add(n).read() } != 0 {
        n += 1;
        if n > 4096 {
            return Err("prilis dlouhy retezec".to_string());
        }
    }
    // SAFETY: `n` bajtu pred nulou.
    core::str::from_utf8(unsafe { core::slice::from_raw_parts(p, n) }).map_err(|_| "retezec neni UTF-8".to_string())
}

/// Handle modulu `i` (nasobek 16, nikdy 0 ani `RTLD_NEXT`).
fn handle(i: usize) -> *mut u8 {
    ((i as u64 + 1) << 4) as *mut u8
}

fn index(s: &State, h: *mut u8) -> Res<usize> {
    let h = h as u64;
    let i = (h >> 4).wrapping_sub(1) as usize;
    if h & 15 != 0 || h == 0 || i >= s.mods.len() {
        return Err(format!("neplatny handle {h:#x}"));
    }
    Ok(i)
}

pub unsafe extern "C" fn dlopen(path: *const u8, flags: i32) -> *mut u8 {
    let _l = dl_lock();
    // SAFETY: cesta od volajiciho (C retezec) nebo null.
    let r = if path.is_null() { Ok(handle(0)) } else { unsafe { c_str(path) }.and_then(|p| open(p, flags)) };
    r.unwrap_or_else(|e| {
        set_error(&format!("dlopen: {e}"));
        core::ptr::null_mut()
    })
}

/// Nahraje knihovnu `path` (a zavislosti), vraci handle.
fn open(path: &str, flags: i32) -> Res<*mut u8> {
    // Hole jmeno = /system/lib, absolutni cesta = /system/lib nebo disk
    // (relace FS procesu); nahrany modul se hleda podle normalizovaneho
    // jmena.
    let key = match dynload::search::candidates(path, None, None).map_err(|e| format!("{path}: {e}"))?.into_iter().next() {
        Some(dynload::search::Source::System(n) | dynload::search::Source::Disk(n)) => n,
        None => return Err(format!("{path}: neplatna cesta")),
    };
    let name = key.as_str();
    let page = libos::vm::page_size();
    let (root, inits) = {
        let mut g = STATE.lock();
        let s = g.as_mut().ok_or("zavadec neni pripraven")?;
        if let Some(i) = s.find(name) {
            s.mods[i].refs += 1;
            if flags & rtld::GLOBAL != 0 {
                make_global(s, i)?;
            }
            return Ok(handle(i));
        }
        if flags & rtld::NOLOAD != 0 {
            return Ok(core::ptr::null_mut());
        }
        let first = s.mods.len();
        let saved = (s.tls.surplus.clone(), s.tls.next_id);
        match load_new(s, name, first, flags, page) {
            Ok(inits) => (first, inits),
            Err(e) => {
                for m in s.mods.drain(first..) {
                    module::discard(&m);
                }
                (s.tls.surplus, s.tls.next_id) = saved;
                return Err(e);
            }
        }
    };
    // Inicializatory bez zamku stavu (smi volat `dlopen`, pristupovat k TLS).
    for i in inits {
        i.run();
    }
    Ok(handle(root))
}

/// Pridat skupinu modulu `i` do globalniho rozsahu.
fn make_global(s: &mut State, i: usize) -> Res<()> {
    for j in module::group(&s.mods, i)? {
        if !s.global.contains(&j) {
            s.global.push(j);
        }
    }
    Ok(())
}

/// Nahraje `name` a chybejici zavislosti na indexy od `first`, rozdeli
/// TLS, relokuje, zapeceti a zaregistruje. Vraci inicializatory (zavislosti
/// napred). Pri chybe zahodi volajici moduly od `first` a vrati rezervu.
fn load_new(s: &mut State, name: &str, first: usize, flags: i32, page: u64) -> Res<Vec<Init>> {
    match module::resolve(&s.process, &s.mods, None, name)? {
        Err(m) => s.mods.push(m),
        Ok(_) => return Err(format!("{name}: uz nahrany")),
    }
    let mut k = first;
    while k < s.mods.len() {
        for n in s.mods[k].needed()? {
            if let Err(m) = module::resolve(&s.process, &s.mods, Some(k), &n)? {
                s.mods.push(m);
            }
        }
        k += 1;
    }
    for k in first..s.mods.len() {
        place_tls(s, k)?;
    }
    let grp = module::group(&s.mods, first)?;
    let mut scope = s.global.clone();
    for &j in &grp {
        if !scope.contains(&j) {
            scope.push(j);
        }
    }
    for k in (first..s.mods.len()).rev() {
        module::relocate(&s.mods, &scope, k)?;
    }
    for k in first..s.mods.len() {
        module::seal(&mut s.mods[k], page)?;
    }
    // Od ted se nic nevraci: TLS modulu do vlaken, rozsah, debugger.
    commit_tls(s, first);
    s.mods[first].refs = 1;
    if flags & rtld::GLOBAL != 0 {
        for j in grp {
            if !s.global.contains(&j) {
                s.global.push(j);
            }
        }
    }
    let list: Vec<(u64, *const u8, u64)> =
        s.mods[first..].iter().map(|m| (m.base, m.cname.as_ptr(), m.dynamic_addr())).collect();
    iface::r_debug_add(&list, 0);
    Ok(s.mods[first..].iter().rev().map(Init::of).collect())
}

/// Cislo a misto TLS modulu `k` (zatim jen v modulu; do vlaken az
/// [`commit_tls`]). Initial-exec = kus rezervy statickeho TLS, jinak liny
/// blok pres DTV.
fn place_tls(s: &mut State, k: usize) -> Res<()> {
    let m = &s.mods[k];
    let Some(t) = m.tls else { return Ok(()) };
    if s.tls.next_id as usize > tls::MAX_MODULES {
        return Err(format!("{}: prilis mnoho modulu s TLS (nejvys {})", m.name, tls::MAX_MODULES));
    }
    let place = if m.needs_static_tls()? {
        let off = s.tls.surplus.alloc(t.mem_size, t.align).ok_or_else(|| {
            format!(
                "{}: staticke TLS (initial-exec) potrebuje {} B (zarovnani {}), v rezerve zbyva {} z {} B; \
                 zvyste [app] static_tls_reserve v manifestu aplikace (nejvys {} KiB)",
                m.name,
                t.mem_size,
                t.align,
                s.tls.surplus.size() - s.tls.surplus.used(),
                s.tls.surplus.size(),
                dynload::MAX_STATIC_TLS_RESERVE / 1024
            )
        })?;
        TlsPlace::Static(off)
    } else {
        TlsPlace::Dynamic
    };
    let id = s.tls.next_id;
    s.tls.next_id += 1;
    s.mods[k].tls_id = Some((id, place));
    Ok(())
}

/// TLS novych modulu do stavu: staticke do sablon a do bloku vsech
/// zijicich vlaken, line do seznamu (bloky az pri pristupu).
fn commit_tls(s: &mut State, first: usize) {
    let mut any = false;
    for m in &s.mods[first..] {
        let (Some(t), Some((id, place))) = (m.tls, m.tls_id) else { continue };
        any = true;
        let data = m.elf.tls_data(&t).unwrap_or(&[]);
        match place {
            TlsPlace::Static(off) => {
                for &tp in &s.tls.threads {
                    // SAFETY: kus rezervy (nikdo ho jeste nepouzil) v bloku
                    // zijiciho vlakna; polozka DTV modulu.
                    unsafe {
                        let at = tp.wrapping_add_signed(off);
                        core::ptr::copy_nonoverlapping(data.as_ptr(), at as *mut u8, data.len());
                        tls::dtv_of(tp).add(id as usize).write(at);
                    }
                }
                s.tls.templates.push(TlsTemplate { module: id, off, src: data.as_ptr() as u64, len: data.len() as u64 });
                iface::log_surplus(&s.tls, &format!("dlopen {} (staticke TLS {} B)", m.name, t.mem_size));
            }
            TlsPlace::Dynamic => {
                let layout = Layout::from_size_align(t.mem_size.max(1) as usize, t.align.max(1) as usize)
                    .unwrap_or(Layout::new::<u64>());
                s.tls.dynamic.push(DynTls { module: id, src: data.as_ptr() as u64, len: data.len() as u64, layout });
            }
        }
    }
    if any {
        tls::GENERATION.fetch_add(1, Ordering::AcqRel);
    }
}

pub unsafe extern "C" fn dlsym(h: *mut u8, name: *const u8) -> *mut u8 {
    let _l = dl_lock();
    // SAFETY: jmeno od volajiciho (C retezec).
    let r = unsafe { c_str(name) }.and_then(|n| sym(h, n));
    r.unwrap_or_else(|e| {
        set_error(&format!("dlsym: {e}"));
        core::ptr::null_mut()
    })
}

fn sym(h: *mut u8, name: &str) -> Res<*mut u8> {
    let found = {
        let g = STATE.lock();
        let s = g.as_ref().ok_or("zavadec neni pripraven")?;
        let scope = if h as usize == rtld::DEFAULT || h == handle(0) {
            s.global.clone()
        } else if h as usize == rtld::NEXT {
            return Err("RTLD_NEXT zatim neni".to_string());
        } else {
            module::group(&s.mods, index(s, h)?)?
        };
        module::find_symbol(&s.mods, &scope, name)?.ok_or_else(|| format!("{name}: symbol nenalezen"))?
    };
    // Bez zamku stavu: blok TLS a resitel IFUNC ho smi potrebovat.
    Ok(match found {
        Found::Addr(a) => a as *mut u8,
        Found::Tls(_, value, TlsPlace::Static(off)) => tp().wrapping_add_signed(off).wrapping_add(value) as *mut u8,
        Found::Tls(id, value, TlsPlace::Dynamic) => tls::tls_get_addr(&[id, value]),
        Found::Ifunc(f) => {
            // SAFETY: resitel IFUNC relokovaneho modulu.
            let f: extern "C" fn() -> u64 = unsafe { core::mem::transmute(f as usize) };
            f() as *mut u8
        }
    })
}

pub unsafe extern "C" fn dlclose(h: *mut u8) -> i32 {
    let _l = dl_lock();
    let mut g = STATE.lock();
    let Some(s) = g.as_mut() else { return -1 };
    match index(s, h) {
        Ok(i) => {
            let m = &mut s.mods[i];
            m.refs = m.refs.saturating_sub(1);
            0
        }
        Err(e) => {
            drop(g);
            set_error(&format!("dlclose: {e}"));
            -1
        }
    }
}

/// Vestavene funkce zavadece pro kod v C (jmeno -> adresa).
pub fn builtin(name: &str) -> Option<u64> {
    Some(match name {
        "dl_iterate_phdr" => iface::dl_iterate_phdr as *const () as u64,
        "dlopen" => dlopen as *const () as u64,
        "dlsym" => dlsym as *const () as u64,
        "dlclose" => dlclose as *const () as u64,
        "dlerror" => dlerror as *const () as u64,
        "dladdr" => iface::dladdr as *const () as u64,
        _ => return None,
    })
}

