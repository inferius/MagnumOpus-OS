//! Zavedeni programu: zprava zavadece, auxv, knihovny ze startu, TLS,
//! relokace, pecet, inicializatory. Moduly a jejich operace jsou
//! v `module.rs` (spolecne s `dlopen`); chyba tady = konec procesu.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use abi::auxv::{AT_BASE, AT_ENTRY, AT_NULL, AT_PHDR, AT_PHNUM};
use abi::bootstrap::{self, Entry, Header};
use dynload::TlsModule;
use elf::{Machine, pt};
use libos::{Handle, Process, sys};

use crate::fail;
use crate::iface::{self, State, TlsState, TlsTemplate};
use crate::module::{self, Init, Module, Res, TlsPlace, module_memory};
use crate::tls;

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
    /// Rezerva statickeho TLS (`AT_MO_TLS_RESERVE`).
    tls_reserve: Option<u64>,
}

/// Vysledek nebo konec procesu s hlaskou.
fn or_fail<T>(r: Res<T>) -> T {
    r.unwrap_or_else(|e| fail(&e))
}

/// Zprava zavadece (`abi::bootstrap::LOADER_MAGIC`): handle procesu,
/// relace s libd a listek k ni.
fn loader_message(bootstrap: u64) -> (u32, u32, u32) {
    let mut data = [0u8; 64];
    let mut handles = [0u32; 4];
    let (r, (dlen, hlen)) = sys::channel_read(bootstrap as u32, &mut data, &mut handles);
    if r.is_err() {
        fail("chybi zprava zavadece (program spusteny bez spawn?)");
    }
    let Some(h) = Header::from_bytes_with(&data[..dlen], bootstrap::LOADER_MAGIC) else {
        fail("neplatna zprava zavadece");
    };
    let (mut process, mut libd, mut lease) = (None, None, None);
    for (i, &raw) in handles[..(h.count as usize).min(hlen)].iter().enumerate() {
        let off = bootstrap::HEADER_SIZE + i * bootstrap::ENTRY_SIZE;
        match data.get(off..dlen).and_then(Entry::from_bytes).map(|e| (e.kind, e.arg)) {
            Some((bootstrap::kind::SELF_PROCESS, _)) => process = Some(raw),
            Some((bootstrap::kind::LIBD, 0)) => libd = Some(raw),
            Some((bootstrap::kind::LIBD, 1)) => lease = Some(raw),
            _ => {
                let _ = sys::handle_close(raw);
            }
        }
    }
    match (process, libd, lease) {
        (Some(p), Some(s), Some(l)) => (p, s, l),
        _ => fail("zprava zavadece bez procesu, relace s libd nebo listku"),
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
        let mut aux = Aux { phdr: 0, phnum: 0, entry: 0, base: 0, loader_slot: 0, tls_reserve: None };
        loop {
            let (t, v) = (p.read(), p.add(1).read());
            match t {
                AT_NULL => break,
                AT_PHDR => aux.phdr = v,
                AT_PHNUM => aux.phnum = v,
                AT_ENTRY => aux.entry = v,
                AT_BASE => aux.base = v,
                abi::loader::AT_MO_LOADER => aux.loader_slot = p.add(1) as u64,
                abi::loader::AT_MO_TLS_RESERVE => aux.tls_reserve = Some(v),
                _ => {}
            }
            p = p.add(2);
        }
        aux
    }
}

/// Program namapovany `spawn`: baze z `AT_PHDR` a `PT_PHDR` (jinak hlavicky
/// hned za hlavickou ELF), rozsah z PT_LOAD v pameti.
fn program(aux: &Aux) -> Res<Module> {
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
    module::module("program", base, unsafe { module_memory(base, end) })
}

/// Rozlozi staticke TLS vsech modulu ze startu a za ne rezervu pro
/// `dlopen`, vytvori blok hlavniho vlakna a nastavi TP.
fn setup_tls(mods: &mut [Module], reserve: u64) -> TlsState {
    let list: Vec<Option<TlsModule>> = mods.iter().map(|m| m.tls.map(|t| TlsModule { size: t.mem_size, align: t.align })).collect();
    let layout = dynload::tls_layout(Machine::NATIVE, &list, reserve);
    let page = libos::vm::page_size();
    if layout.align > page || mods.len() > tls::MAX_MODULES {
        fail("TLS: prilis velke zarovnani nebo mnoho modulu");
    }
    let mut templates = Vec::new();
    let mut id = 0u64;
    for (m, off) in mods.iter_mut().zip(&layout.offsets) {
        let (Some(t), Some(off)) = (m.tls, off) else { continue };
        id += 1;
        let data = or_fail(module::ctx(m.elf.tls_data(&t), &m.name));
        templates.push(TlsTemplate { module: id, off: *off, src: data.as_ptr() as u64, len: data.len() as u64 });
        m.tls_id = Some((id, TlsPlace::Static(*off)));
    }
    // TP v bloku zarovnaneho na stranku: zaruka zarovnani = nejvyssi
    // mocnina dvou, ktera deli posun TP (aspon `layout.align`).
    let tp_align = if layout.tp == 0 { page } else { (1u64 << layout.tp.trailing_zeros()).min(page) };
    let surplus = dynload::Surplus::new(Machine::NATIVE, &layout, tp_align);
    // DTV za statickym TLS (zarovnane na 8 B).
    let dtv = layout.size.next_multiple_of(8);
    let mut state = TlsState {
        len: (dtv + tls::DTV_BYTES).next_multiple_of(page),
        dtv,
        tp: layout.tp,
        templates,
        dynamic: Vec::new(),
        surplus,
        threads: Vec::new(),
        next_id: id + 1,
    };
    let tp = iface::new_block(&state);
    if tp == 0 {
        fail("pamet TLS");
    }
    if sys::thread_set_pointer(tp).is_err() {
        fail("ukazatel vlakna");
    }
    state.threads.push(tp);
    state
}

/// RELRO zavadece samotneho (jeho GOT a tabulka `INTERFACE`): po
/// samorelokaci ho nic nemeni.
fn seal_own_relro(base: u64, page: u64) -> Res<()> {
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
                module::seal_relro(base, elf::Relro { vaddr, mem_size }, page)?;
            }
        }
    }
    Ok(())
}

/// `r_debug` pro debuggery: seznam modulu a jeho adresa do `DT_DEBUG`
/// programu (dynamicka sekce je v RELRO - zapis pred zapecetenim).
fn debug_hook(mods: &[Module], ldbase: u64) {
    let list: Vec<(u64, *const u8, u64)> = mods.iter().map(|m| (m.base, m.cname.as_ptr(), m.dynamic_addr())).collect();
    let r = iface::r_debug_add(&list, ldbase);
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
    let (process_raw, libd_raw, lease_raw) = loader_message(bootstrap);
    libos::rt::set_self_process(process_raw);
    let process = Process::from_handle(Handle::from_raw(process_raw));
    // Relaci s libd a listek si zavadec nechava (dlopen, obnova po padu
    // libd); program dostane vlastni az na pozadani (`libd_session`).
    libos::libd::set_connection(
        libos::Channel::from_handle(Handle::from_raw(libd_raw)),
        Some(libos::Channel::from_handle(Handle::from_raw(lease_raw))),
    );
    let page = libos::vm::page_size();
    // SAFETY: SP vlakna s blokem od spawn.
    let aux = unsafe { read_auxv(sp) };

    // Program a zavislosti do sirky (kazda knihovna jednou, podle jmena
    // z DT_NEEDED nebo SONAME).
    let mut mods = alloc::vec![or_fail(program(&aux))];
    let mut next = 0;
    while next < mods.len() {
        for n in or_fail(mods[next].needed()) {
            let Some(name) = dynload::system_name(&n).filter(|x| libdproto::valid_name(x)) else {
                fail(&format!("knihovna {n}: jen jmeno v /system/lib"));
            };
            let name: String = name.to_string();
            if mods.iter().any(|m| m.is(&name)) {
                continue;
            }
            mods.push(or_fail(module::load_library(&process, &name)));
        }
        next += 1;
    }

    let tls_state = setup_tls(&mut mods, dynload::tls_reserve(aux.tls_reserve));
    // Zavislosti napred (opacne poradi nahrani), program posledni; rozsah
    // = vsechny moduly ze startu v poradi nahrani.
    let scope: Vec<usize> = (0..mods.len()).collect();
    for i in (0..mods.len()).rev() {
        or_fail(module::relocate(&mods, &scope, i));
    }
    debug_hook(&mods, aux.base);
    for m in &mut mods {
        or_fail(module::seal(m, page));
    }
    let inits: Vec<Init> = mods.iter().rev().map(Init::of).collect();
    iface::set_state(State { process, tls: tls_state, mods, global: scope });
    if aux.loader_slot != 0 {
        // SAFETY: misto hodnoty `AT_MO_LOADER` v auxv na zasobniku (spawn).
        unsafe { (aux.loader_slot as *mut u64).write(&iface::INTERFACE as *const _ as u64) };
    }
    or_fail(seal_own_relro(aux.base, page));
    // Inicializatory az se stavem (smi volat `dlopen`, `dl_iterate_phdr`).
    for i in inits {
        i.run();
    }
    aux.entry
}
