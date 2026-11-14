//! Zavedeni programu: zprava zavadece, auxv, knihovny, TLS, relokace,
//! RELRO, inicializatory.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use abi::auxv::{AT_BASE, AT_ENTRY, AT_NULL, AT_PHDR, AT_PHNUM};
use abi::bootstrap::{self, Entry, Header};
use abi::vm::{MAP_FIXED, MAP_IN_RESERVATION, MAP_READ};
use dynload::{Apply, Def, TlsModule};
use elf::dynamic::Dynamic;
use elf::reloc::Kind;
use elf::symbols::bind;
use elf::{ElfFile, Machine, Tls, pt};
use libos::{Handle, Process, sys};

use crate::iface::{self, ModInfo, State, TlsState, TlsTemplate};
use crate::{fail, tls};

/// Nahrany modul.
struct Module {
    /// Jmeno z `DT_NEEDED` (program: "program").
    name: String,
    soname: Option<String>,
    base: u64,
    elf: ElfFile<'static>,
    dynamic: Dynamic,
    tls: Option<Tls>,
    /// TLS modulu: (cislo od 1, posun od TP).
    tls_info: Option<(u64, i64)>,
}

/// Hodnoty z auxv.
struct Aux {
    phdr: u64,
    phnum: u64,
    entry: u64,
    /// Baze zavadece.
    base: u64,
    /// Kam zapsat adresu tabulky zavadece (`AT_MO_LOADER`); 0 = spawn
    /// misto nepripravil.
    loader_slot: u64,
}

fn check<T, E: core::fmt::Display>(r: Result<T, E>, what: &str) -> T {
    r.unwrap_or_else(|e| fail(&format!("{what}: {e}")))
}

/// Zprava zavadece (`abi::bootstrap::LOADER_MAGIC`): handle procesu
/// a spojeni s libd.
fn loader_message(bootstrap: u64) -> (u32, u32) {
    let mut data = [0u8; 64];
    let mut handles = [0u32; 4];
    let (r, (dlen, hlen)) = sys::channel_read(bootstrap as u32, &mut data, &mut handles);
    if r.is_err() {
        fail("chybi zprava zavadece (program spusteny bez spawn?)");
    }
    let Some(h) = Header::from_bytes_with(&data[..dlen], bootstrap::LOADER_MAGIC) else {
        fail("neplatna zprava zavadece");
    };
    let (mut process, mut libd) = (None, None);
    for (i, &raw) in handles[..(h.count as usize).min(hlen)].iter().enumerate() {
        let off = bootstrap::HEADER_SIZE + i * bootstrap::ENTRY_SIZE;
        match data.get(off..dlen).and_then(Entry::from_bytes).map(|e| e.kind) {
            Some(bootstrap::kind::SELF_PROCESS) => process = Some(raw),
            Some(bootstrap::kind::LIBD) => libd = Some(raw),
            _ => {
                let _ = sys::handle_close(raw);
            }
        }
    }
    match (process, libd) {
        (Some(p), Some(l)) => (p, l),
        _ => fail("zprava zavadece bez procesu nebo spojeni s libd"),
    }
}

/// auxv za argc, argv a envp (`abi::auxv`).
///
/// # Safety
///
/// `sp` je pocatecni SP vlakna, blok zapsal `spawn`.
unsafe fn read_auxv(sp: *const u64) -> Aux {
    // SAFETY: rozlozeni SysV na zasobniku (`abi::auxv`).
    unsafe {
        let argc = sp.read();
        let mut p = sp.add(1 + argc as usize + 1);
        while p.read() != 0 {
            p = p.add(1);
        }
        p = p.add(1);
        let mut aux = Aux { phdr: 0, phnum: 0, entry: 0, base: 0, loader_slot: 0 };
        loop {
            let (t, v) = (p.read(), p.add(1).read());
            match t {
                AT_NULL => break,
                AT_PHDR => aux.phdr = v,
                AT_PHNUM => aux.phnum = v,
                AT_ENTRY => aux.entry = v,
                AT_BASE => aux.base = v,
                abi::loader::AT_MO_LOADER => aux.loader_slot = p.add(1) as u64,
                _ => {}
            }
            p = p.add(2);
        }
        aux
    }
}

/// Pamet modulu od baze do konce posledniho segmentu.
///
/// # Safety
///
/// Rozsah patri modulu namapovanemu v tomto procesu (do konce procesu).
unsafe fn module_memory(base: u64, span: u64) -> &'static [u8] {
    // SAFETY: predano volajicim.
    unsafe { core::slice::from_raw_parts(base as *const u8, span as usize) }
}

/// Program namapovany `spawn`: baze z `AT_PHDR` a `PT_PHDR` (jinak hlavicky
/// hned za hlavickou ELF), rozsah z PT_LOAD v pameti.
fn program(aux: &Aux) -> Module {
    let mut phdr_vaddr = 64;
    let mut end = 0u64;
    for i in 0..aux.phnum {
        let p = (aux.phdr + i * 56) as *const u8;
        // SAFETY: programove hlavicky programu (auxv od spawn), 56 B kazda.
        let (t, vaddr, memsz) = unsafe {
            (
                (p as *const u32).read_unaligned(),
                (p.add(16) as *const u64).read_unaligned(),
                (p.add(40) as *const u64).read_unaligned(),
            )
        };
        if t == pt::PHDR {
            phdr_vaddr = vaddr;
        }
        if t == pt::LOAD {
            end = end.max(vaddr + memsz);
        }
    }
    let base = aux.phdr - phdr_vaddr;
    // SAFETY: segmenty programu namapoval spawn od baze do `end`.
    let mem = unsafe { module_memory(base, end) };
    module(String::from("program"), base, mem)
}

fn module(name: String, base: u64, mem: &'static [u8]) -> Module {
    let elf = check(ElfFile::parse_loaded(mem, Machine::NATIVE), &name);
    let dynamic = check(elf.dynamic(), &name).unwrap_or_else(|| fail(&format!("{name}: chybi PT_DYNAMIC")));
    let soname = dynamic.soname.and_then(|o| elf.dyn_str(&dynamic, o).ok()).map(ToString::to_string);
    let tls = check(elf.tls(), &name);
    Module { name, soname, base, elf, dynamic, tls, tls_info: None }
}

/// Nahraje knihovnu `needed` od sluzby knihoven libd na nahodnou bazi:
/// kod primo ze zapecetene sablony (sdilene stranky, zapeceti se), data
/// jako klon s kopii pri zapisu.
fn load_library(process: &Process, needed: &str) -> Module {
    let name = needed.strip_prefix(abi::auxv::SYSTEM_LIB).unwrap_or(needed);
    if !libdproto::valid_name(name) {
        fail(&format!("knihovna {needed}: jen jmeno v /system/lib"));
    }
    let (lib, vmos) = match libos::libd::open(name) {
        Ok(r) => r,
        Err(libos::Error::NotFound) => fail(&format!("knihovna {needed} nenalezena")),
        Err(e) => fail(&format!("knihovna {needed}: {e}")),
    };
    let base = check(libos::spawn::random_base(process, lib.span), "misto pro knihovnu");
    let fixed = MAP_FIXED | MAP_IN_RESERVATION;
    for (seg, vmo) in lib.segments.iter().zip(&vmos) {
        if seg.kind == libdproto::kind::SHARED {
            check(process.map(vmo, 0, seg.len, seg.prot | fixed, base + seg.start), "mapovani knihovny");
            check(process.protect(base + seg.start, seg.len, seg.prot | abi::vm::PROT_SEAL), "pecet kodu");
        } else {
            let own = check(vmo.clone_for(0, seg.len, process), "data knihovny");
            check(process.map(&own, 0, seg.len, seg.prot | fixed, base + seg.start), "mapovani dat");
        }
    }
    // SAFETY: segmenty jsou prave namapovane v rezervaci `base..base+span`.
    let mem = unsafe { module_memory(base, lib.span) };
    let m = module(needed.to_string(), base, mem);
    if check(m.elf.interp(), needed).is_some() {
        fail(&format!("{needed} neni sdilena knihovna"));
    }
    m
}

/// Rozlozi staticke TLS vsech modulu (s rezervou pro `dlopen`), vytvori
/// blok hlavniho vlakna a nastavi TP. Vraci stav pro bloky dalsich vlaken.
fn setup_tls(mods: &mut [Module]) -> Option<TlsState> {
    let list: Vec<Option<TlsModule>> = mods.iter().map(|m| m.tls.map(|t| TlsModule { size: t.mem_size, align: t.align })).collect();
    let layout = dynload::tls_layout(Machine::NATIVE, &list, dynload::STATIC_TLS_SURPLUS);
    if layout.align > libos::vm::page_size() || mods.len() > tls::MAX_MODULES {
        fail("TLS: prilis velke zarovnani nebo mnoho modulu");
    }
    let mut templates = Vec::new();
    let mut id = 0u64;
    for (m, off) in mods.iter_mut().zip(&layout.offsets) {
        let (Some(t), Some(off)) = (m.tls, off) else { continue };
        id += 1;
        let data = check(m.elf.tls_data(&t), &m.name);
        templates.push(TlsTemplate { module: id, off: *off, src: data.as_ptr() as u64, len: data.len() as u64 });
        m.tls_info = Some((id, *off));
    }
    // DTV za statickym TLS (zarovnane na 8 B).
    let dtv = layout.size.next_multiple_of(8);
    let state = TlsState { len: (dtv + tls::DTV_BYTES).next_multiple_of(libos::vm::page_size()), dtv, tp: layout.tp, templates };
    let tp = iface::new_block(&state);
    if tp == 0 {
        fail("pamet TLS");
    }
    check(sys::thread_set_pointer(tp), "ukazatel vlakna");
    Some(state)
}

/// Definice symbolu podle jmena v globalnim rozsahu (program, pak knihovny
/// v poradi nahrani); `None` = nikde.
fn lookup(mods: &[Module], name: &str) -> Option<Def> {
    for m in mods {
        let syms = m.elf.symbols(&m.dynamic).ok()?;
        if let Ok(Some((_, s))) = syms.lookup(name) {
            return Some(Def { base: m.base, value: s.value, tls: m.tls_info });
        }
    }
    None
}

/// Funkce zavadece, ktere moduly volaji jmenem.
fn builtin(name: &str) -> Option<u64> {
    match name {
        "__tls_get_addr" => Some(tls::tls_get_addr as *const () as u64),
        _ => iface::builtin(name),
    }
}

/// Zapise slovo do pameti modulu.
///
/// # Safety
///
/// `at` je misto relokace v zapisovatelnem segmentu modulu.
unsafe fn put(at: u64, v: u64) {
    // SAFETY: predano volajicim.
    unsafe { (at as *mut u64).write_unaligned(v) };
}

/// Relokace modulu `i` (vse hned).
fn relocate(mods: &[Module], i: usize) {
    let m = &mods[i];
    let syms = check(m.elf.symbols(&m.dynamic), &m.name);
    let tlsdesc = tls::tlsdesc_static as *const () as u64;
    for r in check(m.elf.relas(&m.dynamic), &m.name) {
        let r = check(r, &m.name);
        if r.kind == Kind::None {
            continue;
        }
        let def = if r.sym == 0 {
            None
        } else {
            let s = check(syms.get(r.sym as usize), &m.name);
            // Lokalni a skryte symboly patri modulu samotnemu.
            if s.bind == bind::LOCAL || (s.is_defined() && !s.is_exported()) {
                Some(Def { base: m.base, value: s.value, tls: m.tls_info })
            } else if let Some(a) = builtin(s.name) {
                Some(Def { base: 0, value: a, tls: None })
            } else {
                match lookup(mods, s.name) {
                    Some(d) => Some(d),
                    None if s.is_weak() => None,
                    None => fail(&format!("nedefinovany symbol {} (v {})", s.name, m.name)),
                }
            }
        };
        let at = m.base + r.offset;
        match dynload::compute(r.kind, m.base, r.addend, def, m.tls_info, tlsdesc) {
            // SAFETY: misto relokace lezi v modulu (zapisovatelny segment
            // nebo RELRO, jeste pred prepnutim na jen pro cteni).
            Ok(Apply::Word(v)) => unsafe { put(at, v) },
            Ok(Apply::Desc(f, a)) => unsafe {
                put(at, f);
                put(at + 8, a);
            },
            Ok(Apply::CallIfunc(f)) => {
                // SAFETY: resitel IFUNC modulu (kod modulu, uz relokovany
                // jsou zavislosti).
                let f: extern "C" fn() -> u64 = unsafe { core::mem::transmute(f as usize) };
                unsafe { put(at, f()) };
            }
            Err(e) => fail(&format!("{}: {e}", m.name)),
        }
    }
    for a in check(m.elf.relrs(&m.dynamic), &m.name) {
        let at = m.base + check(a, &m.name);
        // SAFETY: relativni relokace v datech modulu.
        unsafe { put(at, (at as *const u64).read_unaligned().wrapping_add(m.base)) };
    }
}

/// RELRO jen pro cteni (cele stranky uvnitr oblasti).
/// RELRO jen pro cteni a zapecetit (nevratne: GOT nejde vratit na RW ani
/// nahradit novym mapovanim). Cele stranky uvnitr oblasti.
fn seal_relro(base: u64, r: elf::Relro, page: u64) {
    let start = (base + r.vaddr).next_multiple_of(page);
    let end = (base + r.vaddr + r.mem_size) & !(page - 1);
    if end > start {
        check(libos::vm::seal(start, end - start, MAP_READ), "RELRO");
    }
}

fn protect_relro(m: &Module, page: u64) {
    if let Some(r) = check(m.elf.relro(), &m.name) {
        seal_relro(m.base, r, page);
    }
}

/// RELRO zavadece samotneho (jeho GOT a tabulka `INTERFACE`): po
/// samorelokaci ho nic nemeni.
fn seal_own_relro(base: u64, page: u64) {
    // SAFETY: hlavicka ELF a programove hlavicky zavadece v jeho prvnim
    // segmentu (namapoval spawn).
    unsafe {
        let phoff = ((base + 32) as *const u64).read_unaligned();
        let phnum = ((base + 56) as *const u16).read_unaligned() as u64;
        for i in 0..phnum {
            let p = base + phoff + i * 56;
            if (p as *const u32).read_unaligned() == pt::GNU_RELRO {
                let vaddr = ((p + 16) as *const u64).read_unaligned();
                let mem_size = ((p + 40) as *const u64).read_unaligned();
                seal_relro(base, elf::Relro { vaddr, mem_size }, page);
            }
        }
    }
}

/// `DT_INIT` a `DT_INIT_ARRAY` modulu.
fn run_init(m: &Module) {
    type Init = extern "C" fn(i32, *const u8, *const u8);
    if let Some(f) = m.dynamic.init {
        // SAFETY: inicializator modulu (relokovany kod modulu).
        let f: Init = unsafe { core::mem::transmute((m.base + f) as usize) };
        f(0, core::ptr::null(), core::ptr::null());
    }
    let a = m.dynamic.init_array;
    for i in 0..a.size / 8 {
        // SAFETY: pole ukazatelu (relokovane) v modulu.
        let f = unsafe { ((m.base + a.addr + i * 8) as *const u64).read_unaligned() };
        if f != 0 && f != u64::MAX {
            // SAFETY: inicializator modulu.
            let f: Init = unsafe { core::mem::transmute(f as usize) };
            f(0, core::ptr::null(), core::ptr::null());
        }
    }
}

/// Udaje modulu pro tabulku zavadece.
fn mod_info(m: &Module) -> ModInfo {
    let mut cname = if m.name == "program" { Vec::new() } else { m.name.as_bytes().to_vec() };
    cname.push(0);
    let fa = m.dynamic.fini_array;
    ModInfo {
        base: m.base,
        span: m.elf.data().len() as u64,
        cname,
        phdr: m.base + check(m.elf.phdr_vaddr(), &m.name),
        phnum: m.elf.phnum(),
        tls: m.tls_info,
        fini: m.dynamic.fini.map(|f| m.base + f),
        fini_array: (m.base + fa.addr, fa.size / 8),
    }
}

/// `r_debug` pro debuggery: seznam modulu a jeho adresa do `DT_DEBUG`
/// programu (dynamicka sekce je v RELRO - zapis pred `protect_relro`).
fn debug_hook(mods: &[Module], ldbase: u64) {
    let mut list = Vec::new();
    for m in mods {
        let mut name = if m.name == "program" { Vec::new() } else { m.name.as_bytes().to_vec() };
        name.push(0);
        let name = alloc::boxed::Box::leak(name.into_boxed_slice());
        let dynamic = check(m.elf.program_headers().find(|p| p.as_ref().is_ok_and(|p| p.p_type == pt::DYNAMIC)).transpose(), &m.name);
        list.push((m.base, name.as_ptr(), dynamic.map_or(0, |p| m.base + p.vaddr)));
    }
    let r = iface::build_r_debug(&list, ldbase);
    let Some(dynamic) = list.first().map(|l| l.2).filter(|&d| d != 0) else { return };
    let mut p = dynamic;
    loop {
        // SAFETY: dynamicka sekce programu (pole tag/hodnota do DT_NULL),
        // jeste zapisovatelna (pred RELRO).
        let tag = unsafe { (p as *const u64).read_unaligned() };
        if tag == elf::dynamic::dt::NULL {
            break;
        }
        if tag == 21 {
            // DT_DEBUG
            unsafe { ((p + 8) as *mut u64).write_unaligned(r) };
            break;
        }
        p += 16;
    }
}

/// Zavede program; vraci jeho vstupni bod (skok dela `_start`).
pub extern "C" fn main(sp: *const u64, bootstrap: u64) -> u64 {
    let (process_raw, libd_raw) = loader_message(bootstrap);
    libos::rt::set_self_process(process_raw);
    let process = Process::from_handle(Handle::from_raw(process_raw));
    // Spojeni s libd si zavadec nechava (dlopen); program ma svoje.
    libos::libd::set_connection(libos::Channel::from_handle(Handle::from_raw(libd_raw)));
    let page = libos::vm::page_size();
    // SAFETY: SP vlakna s blokem od spawn.
    let aux = unsafe { read_auxv(sp) };

    // Program a zavislosti do sirky (kazda knihovna jednou, podle jmena
    // z DT_NEEDED nebo SONAME).
    let mut mods = alloc::vec![program(&aux)];
    let mut next = 0;
    while next < mods.len() {
        let needed: Vec<String> = mods[next]
            .dynamic
            .needed()
            .iter()
            .map(|&o| check(mods[next].elf.dyn_str(&mods[next].dynamic, o), &mods[next].name).to_string())
            .collect();
        for n in needed {
            if mods.iter().any(|m| m.name == n || m.soname.as_deref() == Some(n.as_str())) {
                continue;
            }
            let m = load_library(&process, &n);
            mods.push(m);
        }
        next += 1;
    }

    let tls_state = setup_tls(&mut mods);
    // Zavislosti napred (opacne poradi nahrani), program posledni.
    for i in (0..mods.len()).rev() {
        relocate(&mods, i);
    }
    debug_hook(&mods, aux.base);
    for m in &mods {
        protect_relro(m, page);
    }
    for m in mods.iter().rev() {
        run_init(m);
    }
    iface::set_state(State { tls: tls_state, mods: mods.iter().map(mod_info).collect() });
    if aux.loader_slot != 0 {
        // SAFETY: misto hodnoty `AT_MO_LOADER` v auxv na zasobniku (spawn).
        unsafe { (aux.loader_slot as *mut u64).write(&iface::INTERFACE as *const _ as u64) };
    }

    seal_own_relro(aux.base, page);
    // Handle vlastniho procesu si zavadec nechava (bloky TLS novych
    // vlaken, dlopen); program ma handle na tentyz proces stejne (zadna
    // prava navic).
    core::mem::forget(process);
    aux.entry
}
