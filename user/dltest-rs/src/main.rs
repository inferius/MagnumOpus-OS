//! Test DL-3 (`docs/cs/dynamicke-linkovani.md`): program v Rustu nad libos
//! prelozeny jako PIE se zavadecem `ld.so` a knihovnami `libdltest.so.1`
//! a `libdltest2.so.1` (`user/dltest`). Bezne programy v Rustu zustavaji
//! staticke - tenhle existuje jen kvuli testu vlaken libos pod zavadecem.
//!
//! Navratovy kod: bitova maska chyb (0 = vse sedi), viz konstanty `E_*`.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use abi::loader::PhdrInfo;
use libos::Startup;

unsafe extern "C" {
    fn dl_fn(x: i32) -> i32;
    fn dl_tls_get() -> i32;
    fn dl_fini_get() -> i32;
}

/// Hlavni vlakno: volani a TLS knihovny.
const E_MAIN: i64 = 1;
/// Nove vlakno nema cerstvou kopii TLS knihovny (nebo neni oddelena).
const E_THREAD_TLS: i64 = 2;
/// Tabulka zavadece chybi (program nevi, ze bezi pod zavadecem).
const E_NO_LOADER: i64 = 4;
/// `dl_iterate_phdr`: spatny pocet modulu, jmena nebo TLS.
const E_PHDR: i64 = 8;
/// `r_debug` v `DT_DEBUG` programu chybi nebo nesedi.
const E_RDEBUG: i64 = 16;
/// Finalizator knihovny se nezavolal.
const E_FINI: i64 = 32;
/// Vlakno nejde spustit.
const E_SPAWN: i64 = 64;
/// `dlopen`/`dlsym`/`dlclose` zakladni chovani nebo `dladdr` nesedi.
const E_DL: i64 = 128;
/// Kod nebo RELRO knihovny neni zapeceteny (jde zmenit).
const E_SEAL: i64 = 256;
/// Knihovna z `dlopen` (dynamicke TLS, inicializator, initial-exec
/// v rezerve) v hlavnim vlakne nesedi.
const E_DLOPEN: i64 = 512;
/// Line TLS knihovny z `dlopen` ve vlaknech (zalozenem pred i po `dlopen`)
/// nesedi.
const E_DLTLS: i64 = 1024;
/// Prilis velke initial-exec TLS: `dlopen` nesmi uspet a `dlerror` musi
/// radit `static_tls_reserve`.
const E_RESERVE: i64 = 2048;

libos::entry!(main);

fn main(mut s: Startup) -> i64 {
    // Cekaci rezim (test poradi obnovy libd, `user/dlspawn`).
    if !s.handles.is_empty() {
        return wait_then_dlopen(libos::Channel::from_handle(s.handles.remove(0)));
    }
    let mut bad = 0;
    // SAFETY: funkce knihovny (C ABI).
    unsafe {
        if dl_fn(1) != 43 || dl_tls_get() != 101 {
            bad |= E_MAIN;
        }
    }
    bad |= threads();
    let Some(iface) = libos::loader::interface() else {
        return bad | E_NO_LOADER;
    };
    bad |= phdrs(iface);
    bad |= r_debug(iface);
    bad |= dl(iface);
    bad |= sealed();
    bad |= dlopen_tls();
    // Konec programu vola fini (rt_own); tady explicitne a overit.
    libos::loader::run_fini();
    // SAFETY: funkce knihovny.
    if unsafe { dl_fini_get() } != 1 {
        bad |= E_FINI;
    }
    bad
}

/// Ctyri vlakna: kazde ma vlastni cerstvou kopii TLS knihovny (100), zmena
/// v nem neovlivni hlavni vlakno (101).
fn threads() -> i64 {
    static OK: AtomicU32 = AtomicU32::new(0);
    let mut hs = Vec::new();
    for i in 0..4 {
        match libos::thread::spawn("dltest-tls", move || {
            // SAFETY: funkce knihovny.
            let fresh = unsafe { dl_tls_get() } == 100;
            // SAFETY: dtto.
            let r = unsafe { dl_fn(2 + i) };
            // SAFETY: dtto.
            let mine = unsafe { dl_tls_get() } == 100 + 2 + i;
            if fresh && r == 2 + i + 40 + 1 + ((2 + i) & 1) && mine {
                OK.fetch_add(1, Ordering::SeqCst);
            }
        }) {
            Ok(h) => hs.push(h),
            Err(e) => {
                libos::println!("dltest-rs: vlakno nejde spustit: {e}");
                return E_SPAWN;
            }
        }
    }
    for h in hs {
        if let Err(e) = h.join(None) {
            libos::println!("dltest-rs: join: {e}");
            return E_SPAWN;
        }
    }
    // SAFETY: funkce knihovny.
    let main_tls = unsafe { dl_tls_get() };
    if OK.load(Ordering::SeqCst) != 4 || main_tls != 101 {
        return E_THREAD_TLS;
    }
    0
}

#[derive(Default)]
struct Seen {
    count: u32,
    libs: u32,
    tls: u32,
    program_dynamic: u64,
    /// RELRO libdltest.so.1 (adresa, delka).
    relro: (u64, u64),
}

unsafe extern "C" fn each(info: *mut PhdrInfo, _size: usize, data: *mut u8) -> i32 {
    // SAFETY: zavadec preda platny zaznam; `data` je nase `Seen`.
    let (info, seen) = unsafe { (&*info, &mut *(data as *mut Seen)) };
    // SAFETY: jmeno je C retezec zavadece.
    let name = unsafe { cstr(info.name) };
    if name == b"libdltest.so.1" || name == b"libdltest2.so.1" {
        seen.libs += 1;
    }
    if name == b"libdltest.so.1" {
        for i in 0..info.phnum as usize {
            // SAFETY: programove hlavicky knihovny (56 B).
            let p = unsafe { info.phdr.add(i * 56) };
            // SAFETY: dtto.
            let (t, vaddr, len) = unsafe {
                ((p as *const u32).read_unaligned(), (p.add(16) as *const u64).read_unaligned(), (p.add(40) as *const u64).read_unaligned())
            };
            if t == 0x6474_E552 {
                seen.relro = (info.addr + vaddr, len);
            }
        }
    }
    if info.tls_modid != 0 && !info.tls_data.is_null() {
        seen.tls += 1;
    }
    if seen.count == 0 {
        // Program: adresa jeho PT_DYNAMIC (pro r_debug).
        for i in 0..info.phnum as usize {
            // SAFETY: programove hlavicky programu (56 B).
            let p = unsafe { info.phdr.add(i * 56) };
            // SAFETY: dtto.
            let (t, vaddr) = unsafe { ((p as *const u32).read_unaligned(), (p.add(16) as *const u64).read_unaligned()) };
            if t == 2 {
                seen.program_dynamic = info.addr + vaddr;
            }
        }
    }
    seen.count += 1;
    0
}

/// C retezec jako bajty.
unsafe fn cstr<'a>(p: *const u8) -> &'a [u8] {
    let mut n = 0;
    // SAFETY: retezec zakonceny nulou (predal ho zavadec).
    unsafe {
        while p.add(n).read() != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n)
    }
}

fn phdrs(iface: &abi::loader::Interface) -> i64 {
    let mut seen = Seen::default();
    // SAFETY: callback a data podle `abi::loader::PhdrCallback`.
    unsafe { (iface.dl_iterate_phdr)(each, &mut seen as *mut Seen as *mut u8) };
    // Program, libdltest, libdltest2; TLS maji obe knihovny.
    if seen.count != 3 || seen.libs != 2 || seen.tls != 2 || seen.program_dynamic == 0 {
        return E_PHDR;
    }
    PROGRAM_DYNAMIC.store(seen.program_dynamic, Ordering::Relaxed);
    RELRO.store(seen.relro.0, Ordering::Relaxed);
    RELRO_LEN.store(seen.relro.1, Ordering::Relaxed);
    0
}

/// RELRO libdltest.so.1 (z `dl_iterate_phdr`).
static RELRO: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static RELRO_LEN: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Adresa `PT_DYNAMIC` programu (z `dl_iterate_phdr`).
static PROGRAM_DYNAMIC: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// `DT_DEBUG` programu ukazuje na `r_debug` (verze 1, seznam 3 modulu).
fn r_debug(_iface: &abi::loader::Interface) -> i64 {
    let mut p = PROGRAM_DYNAMIC.load(Ordering::Relaxed);
    if p == 0 {
        return E_RDEBUG;
    }
    let mut r = 0u64;
    // SAFETY: dynamicka sekce programu (tag, hodnota) do DT_NULL.
    unsafe {
        loop {
            let tag = (p as *const u64).read_unaligned();
            if tag == 0 {
                break;
            }
            if tag == 21 {
                r = ((p + 8) as *const u64).read_unaligned();
            }
            p += 16;
        }
    }
    if r == 0 {
        return E_RDEBUG;
    }
    // SAFETY: r_debug (SVR4): verze i32, ukazatel na link_map; link_map:
    // addr, name, ld, next, prev.
    unsafe {
        let version = (r as *const i32).read();
        let mut node = ((r + 8) as *const u64).read();
        let mut n = 0;
        while node != 0 && n < 16 {
            n += 1;
            node = ((node + 24) as *const u64).read();
        }
        if version != 1 || n != 3 {
            return E_RDEBUG;
        }
    }
    0
}

/// Zakladni `dl*`: vadne jmeno a cesta mimo `/system/lib` = chyba
/// s duvodem (precteni ji vymaze), uz nahrana knihovna = handle a `dlsym`
/// najde jeji funkci, `RTLD_NOLOAD` nenahrava; `dladdr` adresy funkce
/// knihovny najde `libdltest.so.1`.
fn dl(iface: &abi::loader::Interface) -> i64 {
    use abi::loader::rtld;
    use libos::loader::{dlclose, dlopen, dlsym};
    let bad = |what: &str| {
        libos::println!("dltest-rs: dl: {what}");
        E_DL
    };
    // SAFETY: funkce zavadece podle `abi::loader::Interface`.
    unsafe {
        let h = (iface.dlopen)(b"libdltest.so.1 \0".as_ptr(), rtld::NOW);
        let err = (iface.dlerror)();
        if !h.is_null() || err.is_null() || !(iface.dlerror)().is_null() {
            return bad("vadne jmeno");
        }
        let mut info = abi::loader::DlInfo { fname: core::ptr::null(), fbase: 0, sname: core::ptr::null(), saddr: 0 };
        if (iface.dladdr)(dl_fn as *const u8, &mut info) == 0 || cstr(info.fname) != b"libdltest.so.1" || info.fbase == 0 {
            return bad("dladdr");
        }
    }
    match dlopen("/apps/x/libx.so", rtld::NOW) {
        Err(e) if e.contains("/system/lib") => {}
        r => return bad(&alloc::format!("cesta mimo /system/lib: {r:?}")),
    }
    let Ok(h) = dlopen("/system/lib/libdltest.so.1", rtld::NOW) else { return bad("nahrana knihovna") };
    match dlsym(Some(h), "dl_fn") {
        Ok(p) if p as u64 == dl_fn as *const () as u64 => {}
        r => return bad(&alloc::format!("dlsym dl_fn: {r:?}")),
    }
    if dlsym(Some(h), "neexistuje").is_ok() || dlclose(h).is_err() {
        return bad("dlsym neexistujiciho / dlclose");
    }
    // NOLOAD nenahranou knihovnu nenahraje (null bez chyby).
    // SAFETY: funkce zavadece.
    unsafe {
        if !(iface.dlopen)(b"libdlplug.so.1\0".as_ptr(), rtld::NOW | rtld::NOLOAD).is_null() || !(iface.dlerror)().is_null() {
            return bad("NOLOAD");
        }
    }
    0
}

type IntFn = extern "C" fn() -> i32;
type AddrFn = extern "C" fn() -> *mut i32;
type SumFn = extern "C" fn() -> i64;

/// Funkce knihoven z `dlopen` (pro vlakna).
struct Plug {
    bump: IntFn,
    gd_bump: IntFn,
    addr: AddrFn,
    sum: SumFn,
    st_bump: IntFn,
}

fn sym<T: Copy>(lib: libos::loader::Library, name: &str) -> Option<T> {
    let p = libos::loader::dlsym(Some(lib), name).ok()?;
    // SAFETY: `T` je typ ukazatele na funkci s ABI C podle knihovny.
    Some(unsafe { core::mem::transmute_copy(&p) })
}

/// Kontroly TLS knihoven z `dlopen` v jednom vlakne (cerstve hodnoty):
/// dynamicke TLS pres TLSDESC (libdlplug) i `__tls_get_addr`
/// (libdlplug-gd), zarovnany blok s nulami, initial-exec v rezerve.
fn check_fresh(p: &Plug) -> bool {
    (p.bump)() == 8 && (p.bump)() == 9 && (p.gd_bump)() == 8 && (p.sum)() == 3 && (p.st_bump)() == 34
}

/// `dlopen` knihoven s TLS: vlakno zalozene pred `dlopen` a vlakno po nem
/// dostanou cerstve kopie (line bloky, initial-exec zkopirovane do
/// existujicich bloku); prilis velke initial-exec = srozumitelna chyba.
fn dlopen_tls() -> i64 {
    use abi::loader::rtld;
    use libos::loader::{dlopen, dlsym};
    static GO: AtomicU32 = AtomicU32::new(0);
    static OK: AtomicU32 = AtomicU32::new(0);
    static mut PLUG: Option<Plug> = None;
    let mut bad = 0;
    // Vlakno pred dlopen: ceka, az knihovny budou.
    let early = libos::thread::spawn("dltest-early", || {
        while GO.load(Ordering::Acquire) == 0 {
            libos::time::sleep(core::time::Duration::from_millis(1));
        }
        // SAFETY: PLUG zapsalo hlavni vlakno pred GO (Release/Acquire).
        if let Some(p) = unsafe { (*core::ptr::addr_of!(PLUG)).as_ref() }
            && check_fresh(p)
        {
            OK.fetch_add(1, Ordering::SeqCst);
        }
    });
    let early = match early {
        Ok(h) => h,
        Err(_) => return E_SPAWN,
    };
    let libs = (dlopen("libdlplug.so.1", rtld::NOW), dlopen("libdlplug-gd.so.1", rtld::NOW), dlopen("libdlstatic.so.1", rtld::NOW));
    let plug = match libs {
        (Ok(a), Ok(g), Ok(st)) => (|| {
            Some(Plug {
                bump: sym(a, "plug_tls_bump")?,
                gd_bump: sym(g, "plug_tls_bump")?,
                addr: sym(a, "plug_tls_addr")?,
                sum: sym(a, "plug_buf_sum")?,
                st_bump: sym(st, "st_bump")?,
            })
        })()
        .map(|p| (p, a)),
        r => {
            libos::println!("dltest-rs: dlopen: {:?} {:?} {:?}", r.0.err(), r.1.err(), r.2.err());
            None
        }
    };
    let Some((p, a)) = plug else {
        GO.store(1, Ordering::Release);
        let _ = early.join(None);
        return E_DLOPEN;
    };
    // Hlavni vlakno: inicializator (dl2_add(2) = 42), TLS, `dlsym` TLS
    // promenne = adresa ve vlakne.
    let init: Option<IntFn> = sym(a, "plug_init_get");
    if init.is_none_or(|f| f() != 42) || !check_fresh(&p) {
        bad |= E_DLOPEN;
    }
    match dlsym(Some(a), "plug_tls") {
        Ok(v) if v == (p.addr)() as *mut u8 => {}
        _ => bad |= E_DLOPEN,
    }
    // Ve vlaknech cerstve kopie (hlavni vlakno uz ma 9 a 34).
    // SAFETY: zapis pred spustenim cteni (GO, Release).
    unsafe { *core::ptr::addr_of_mut!(PLUG) = Some(Plug { ..p }) };
    GO.store(1, Ordering::Release);
    let late = libos::thread::spawn("dltest-late", || {
        // SAFETY: PLUG uz je zapsane (pred spustenim vlakna).
        if let Some(p) = unsafe { (*core::ptr::addr_of!(PLUG)).as_ref() }
            && check_fresh(p)
        {
            OK.fetch_add(1, Ordering::SeqCst);
        }
    });
    if early.join(None).is_err() || late.map(|h| h.join(None)).is_err() {
        return bad | E_SPAWN;
    }
    if OK.load(Ordering::SeqCst) != 2 {
        bad |= E_DLTLS;
    }
    // SAFETY: vlakna skoncila.
    if let Some(p) = unsafe { (*core::ptr::addr_of!(PLUG)).as_ref() }
        && ((p.bump)() != 10 || (p.st_bump)() != 35)
    {
        bad |= E_DLTLS;
    }
    // 4 KiB initial-exec se do vychozi rezervy (2 KiB) nevejde.
    match dlopen("libdlbig.so.1", rtld::NOW) {
        Err(e) if e.contains("static_tls_reserve") => {}
        r => {
            libos::println!("dltest-rs: libdlbig: {r:?}");
            bad |= E_RESERVE;
        }
    }
    bad
}

/// Kod knihovny (stranka s `dl_fn`) a jeji RELRO jsou zapecetene: prava ani
/// mapovani nejde zmenit (DL-4, `PROT_SEAL`).
fn sealed() -> i64 {
    use abi::vm::{MAP_READ, MAP_WRITE, region};
    let page = libos::vm::page_size();
    let code = (dl_fn as *const () as u64) & !(page - 1);
    let Ok(q) = libos::vm::query(code) else { return E_SEAL };
    if q.attrs & region::ATTR_SEALED == 0 || libos::vm::protect(code, page, MAP_READ).is_ok() || libos::vm::release(code, page).is_ok() {
        return E_SEAL;
    }
    let (r, len) = (RELRO.load(Ordering::Relaxed), RELRO_LEN.load(Ordering::Relaxed));
    let start = r.next_multiple_of(page);
    if r == 0 || r + len < start + page {
        // RELRO knihovny je mensi nez cela stranka: neni co overit.
        return 0;
    }
    let Ok(q) = libos::vm::query(start) else { return E_SEAL };
    if q.attrs & region::ATTR_SEALED == 0 || q.flags != MAP_READ || libos::vm::protect(start, page, MAP_READ | MAP_WRITE).is_ok() {
        return E_SEAL;
    }
    0
}

/// Cekaci rezim: ohlasi start (knihovny uz jsou nahrane), pocka na pokyn
/// a pak `dlopen` - libd mezitim mohla spadnout drive, nez supervisor
/// prevzal listek zavadece (`ADOPT`). Kod 0 = `dlopen` a inicializator
/// knihovny sedi.
fn wait_then_dlopen(ch: libos::Channel) -> i64 {
    if ch.write(b"start", Vec::new()).is_err() {
        return 3;
    }
    if ch.read(Some(libos::time::deadline_after(core::time::Duration::from_secs(60)))).is_err() {
        return 4;
    }
    match libos::loader::dlopen("libdlplug.so.1", abi::loader::rtld::NOW) {
        Ok(lib) => match sym::<IntFn>(lib, "plug_init_get") {
            Some(f) if f() == 42 => 0,
            _ => 1,
        },
        Err(e) => {
            libos::println!("dltest-rs: dlopen po padu libd: {e}");
            2
        }
    }
}
