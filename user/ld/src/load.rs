//! Zavedeni programu: zprava zavadece, auxv, knihovny, TLS, relokace,
//! RELRO, inicializatory.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use abi::auxv::{AT_ENTRY, AT_NULL, AT_PHDR, AT_PHNUM};
use abi::bootstrap::{self, Entry, Header};
use abi::vm::{MAP_EXECUTE, MAP_FIXED, MAP_IN_RESERVATION, MAP_READ, MAP_WRITE};
use dynload::{Apply, Def, TlsModule};
use elf::dynamic::Dynamic;
use elf::reloc::Kind;
use elf::symbols::bind;
use elf::{ElfFile, ElfType, Machine, Tls, pt};
use libos::initfs::Initfs;
use libos::{Handle, Process, Vmo, sys};

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
}

fn check<T, E: core::fmt::Display>(r: Result<T, E>, what: &str) -> T {
    r.unwrap_or_else(|e| fail(&format!("{what}: {e}")))
}

/// Zprava zavadece (`abi::bootstrap::LOADER_MAGIC`): handle procesu
/// a initfs.
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
    let (mut process, mut initfs) = (None, None);
    for (i, &raw) in handles[..(h.count as usize).min(hlen)].iter().enumerate() {
        let off = bootstrap::HEADER_SIZE + i * bootstrap::ENTRY_SIZE;
        match data.get(off..dlen).and_then(Entry::from_bytes).map(|e| e.kind) {
            Some(bootstrap::kind::SELF_PROCESS) => process = Some(raw),
            Some(bootstrap::kind::INITFS) => initfs = Some(raw),
            _ => {
                let _ = sys::handle_close(raw);
            }
        }
    }
    match (process, initfs) {
        (Some(p), Some(i)) => (p, i),
        _ => fail("zprava zavadece bez procesu nebo initfs"),
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
        let mut aux = Aux { phdr: 0, phnum: 0, entry: 0 };
        loop {
            let (t, v) = (p.read(), p.add(1).read());
            match t {
                AT_NULL => break,
                AT_PHDR => aux.phdr = v,
                AT_PHNUM => aux.phnum = v,
                AT_ENTRY => aux.entry = v,
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

/// Nahraje knihovnu `needed` z initfs na nahodnou bazi.
fn load_library(fs: &Initfs, process: &Process, needed: &str, page: u64) -> Module {
    let path = dynload::initfs_path(needed).unwrap_or_else(|| fail(&format!("knihovna {needed}: jen /system/lib")));
    let image = fs.find(&path).unwrap_or_else(|| fail(&format!("knihovna {needed} nenalezena ({path})")));
    let file = check(ElfFile::parse_object(image, Machine::NATIVE), needed);
    if file.elf_type != ElfType::Dyn || check(file.interp(), needed).is_some() {
        fail(&format!("{needed} neni sdilena knihovna"));
    }
    let mut end = 0;
    for seg in file.load_segments() {
        let seg = check(seg, needed);
        end = end.max(seg.vaddr + seg.mem_size);
    }
    let span = end.next_multiple_of(abi::vm::MAX_PAGE_SIZE);
    let base = check(libos::spawn::random_base(process, span), "misto pro knihovnu");
    for seg in file.load_segments() {
        let seg = check(seg, needed);
        let start = seg.vaddr & !(page - 1);
        let stop = (seg.vaddr + seg.mem_size).next_multiple_of(page);
        let vmo = check(Vmo::create(stop - start), "pamet knihovny");
        check(vmo.write(seg.vaddr - start, file.segment_data(&seg)), "zapis knihovny");
        let mut flags = MAP_READ;
        if seg.flags & elf::PF_W != 0 {
            flags |= MAP_WRITE;
        }
        if seg.flags & elf::PF_X != 0 {
            flags |= MAP_EXECUTE;
        }
        // Kod a konstanty zapecetit: spustit jde jen zapecetena pamet (W^X).
        if flags & MAP_WRITE == 0 {
            check(vmo.seal(), "pecet kodu");
        }
        check(process.map(&vmo, 0, stop - start, flags | MAP_FIXED | MAP_IN_RESERVATION, base + start), "mapovani knihovny");
    }
    // SAFETY: segmenty jsou prave namapovane v rezervaci `base..base+span`.
    let mem = unsafe { module_memory(base, end) };
    module(needed.to_string(), base, mem)
}

/// Rozlozi a vytvori staticke TLS hlavniho vlakna, nastavi TP.
fn setup_tls(mods: &mut [Module]) {
    let list: Vec<Option<TlsModule>> = mods.iter().map(|m| m.tls.map(|t| TlsModule { size: t.mem_size, align: t.align })).collect();
    if list.iter().all(Option::is_none) {
        return;
    }
    let layout = dynload::tls_layout(Machine::NATIVE, &list);
    if layout.align > libos::vm::page_size() || mods.len() > tls::MAX_MODULES {
        fail("TLS: prilis velke zarovnani nebo mnoho modulu");
    }
    let len = layout.size.next_multiple_of(libos::vm::page_size());
    let block = check(libos::vm::alloc(len, MAP_READ | MAP_WRITE), "pamet TLS");
    let tp = block + layout.tp;
    let mut id = 0u64;
    for (m, off) in mods.iter_mut().zip(&layout.offsets) {
        let (Some(t), Some(off)) = (m.tls, off) else { continue };
        id += 1;
        let data = check(m.elf.tls_data(&t), &m.name);
        // SAFETY: cil lezi v bloku TLS (rozlozeni z `tls_layout`).
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), tp.wrapping_add_signed(*off) as *mut u8, data.len()) };
        m.tls_info = Some((id, *off));
        tls::set_offset(id, *off);
    }
    if cfg!(target_arch = "x86_64") {
        // TCB: na TP ukazatel sam na sebe (`mov rax, fs:0`).
        // SAFETY: TP + 16 B TCB lezi v bloku.
        unsafe { (tp as *mut u64).write(tp) };
    }
    check(sys::thread_set_pointer(tp), "ukazatel vlakna");
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
        _ => None,
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
fn protect_relro(m: &Module, page: u64) {
    let Some(r) = check(m.elf.relro(), &m.name) else { return };
    let start = (m.base + r.vaddr).next_multiple_of(page);
    let end = (m.base + r.vaddr + r.mem_size) & !(page - 1);
    if end > start {
        check(libos::vm::protect(start, end - start, MAP_READ), "RELRO");
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

/// Zavede program; vraci jeho vstupni bod (skok dela `_start`).
pub extern "C" fn main(sp: *const u64, bootstrap: u64) -> u64 {
    let (process_raw, initfs_raw) = loader_message(bootstrap);
    libos::rt::set_self_process(process_raw);
    let process = Process::from_handle(Handle::from_raw(process_raw));
    let initfs_vmo = Vmo::from_handle(Handle::from_raw(initfs_raw));
    let fs = check(Initfs::from_vmo(&initfs_vmo), "initfs");
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
            let m = load_library(&fs, &process, &n, page);
            mods.push(m);
        }
        next += 1;
    }

    setup_tls(&mut mods);
    // Zavislosti napred (opacne poradi nahrani), program posledni.
    for i in (0..mods.len()).rev() {
        relocate(&mods, i);
    }
    for m in &mods {
        protect_relro(m, page);
    }
    for m in mods.iter().rev() {
        run_init(m);
    }

    // Uklid: initfs a handly zavadece program nedostane.
    fs.close();
    drop(initfs_vmo);
    drop(process);
    aux.entry
}
