//! Moduly (program a knihovny): nahrani od libd, relokace v rozsahu
//! symbolu, pecet kodu a RELRO, inicializatory. Spolecne pro start
//! programu (`load.rs`) a `dlopen` (`dl.rs`): chyby vraci jako text (start
//! je ukonci, `dlopen` je preda do `dlerror`).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use abi::vm::{MAP_FIXED, MAP_IN_RESERVATION, MAP_READ};
use dynload::{Apply, Def};
use elf::dynamic::Dynamic;
use elf::reloc::Kind;
use elf::symbols::{bind, kind};
use elf::{ElfFile, Machine, Tls, pt};
use libos::Process;

use crate::tls;

pub type Res<T> = Result<T, String>;

/// Chyba s popisem, kde nastala.
pub fn ctx<T, E: core::fmt::Display>(r: Result<T, E>, what: &str) -> Res<T> {
    r.map_err(|e| format!("{what}: {e}"))
}

/// Kde lezi TLS modulu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsPlace {
    /// Ve statickem bloku vlakna, posun od TP (moduly ze startu a moduly
    /// z `dlopen` s initial-exec v rezerve).
    Static(i64),
    /// Blok se alokuje az pri prvnim pristupu vlakna (DTV, `dlopen`).
    Dynamic,
}

/// Nahrany modul.
pub struct Module {
    /// Jmeno v `/system/lib` (program: "program").
    pub name: String,
    pub soname: Option<String>,
    /// Jmeno jako C retezec (`dl_iterate_phdr`, `dladdr`, `r_debug`;
    /// program = "").
    pub cname: Vec<u8>,
    pub base: u64,
    /// Rozsah od baze (konec posledniho segmentu).
    pub span: u64,
    pub elf: ElfFile<'static>,
    pub dynamic: Dynamic,
    pub tls: Option<Tls>,
    /// Cislo modulu TLS (od 1) a umisteni.
    pub tls_id: Option<(u64, TlsPlace)>,
    /// Segmenty k zapeceteni po relokaci: (adresa, delka, prava).
    pub seal: Vec<(u64, u64, u32)>,
    /// Pocet otevreni pres `dlopen` (moduly se neodstranuji).
    pub refs: u32,
}

impl Module {
    /// Odpovida jmenu z `DT_NEEDED` nebo `dlopen`?
    pub fn is(&self, name: &str) -> bool {
        self.name == name || self.soname.as_deref() == Some(name)
    }

    /// TLS pro `dynload::Def` (dynamicky modul: posun 0, TP relativni
    /// pristup k nemu relokace odmitne).
    fn tls_def(&self) -> Option<(u64, i64)> {
        self.tls_id.map(|(id, p)| (id, if let TlsPlace::Static(o) = p { o } else { 0 }))
    }

    /// Adresa `PT_DYNAMIC` v pameti (0 = neni).
    pub fn dynamic_addr(&self) -> u64 {
        self.elf
            .program_headers()
            .flatten()
            .find(|p| p.p_type == pt::DYNAMIC)
            .map_or(0, |p| self.base + p.vaddr)
    }

    /// Jmena zavislosti (`DT_NEEDED`).
    pub fn needed(&self) -> Res<Vec<String>> {
        self.dynamic
            .needed()
            .iter()
            .map(|&o| ctx(self.elf.dyn_str(&self.dynamic, o), &self.name).map(ToString::to_string))
            .collect()
    }

    /// Potrebuje modul staticke TLS (initial-exec: `DF_STATIC_TLS` nebo
    /// relokace posunu od TP)?
    pub fn needs_static_tls(&self) -> Res<bool> {
        if self.dynamic.static_tls() {
            return Ok(true);
        }
        for r in ctx(self.elf.relas(&self.dynamic), &self.name)? {
            if ctx(r, &self.name)?.kind == Kind::TlsTpOffset {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Pamet modulu od baze do konce posledniho segmentu.
///
/// # Safety
///
/// Rozsah patri modulu namapovanemu v tomto procesu (do konce procesu).
pub unsafe fn module_memory(base: u64, span: u64) -> &'static [u8] {
    // SAFETY: predano volajicim.
    unsafe { core::slice::from_raw_parts(base as *const u8, span as usize) }
}

/// Modul z pameti (uz namapovany).
pub fn module(name: &str, base: u64, mem: &'static [u8]) -> Res<Module> {
    let elf = ctx(ElfFile::parse_loaded(mem, Machine::NATIVE), name)?;
    let dynamic = ctx(elf.dynamic(), name)?.ok_or_else(|| format!("{name}: chybi PT_DYNAMIC"))?;
    let soname = dynamic.soname.and_then(|o| elf.dyn_str(&dynamic, o).ok()).map(ToString::to_string);
    let tls = ctx(elf.tls(), name)?;
    let mut cname = if name == "program" { Vec::new() } else { name.as_bytes().to_vec() };
    cname.push(0);
    Ok(Module {
        name: name.to_string(),
        soname,
        cname,
        base,
        span: mem.len() as u64,
        elf,
        dynamic,
        tls,
        tls_id: None,
        seal: Vec::new(),
        refs: 0,
    })
}

/// Nahraje knihovnu `name` (jmeno v `/system/lib`) od sluzby knihoven libd
/// na nahodnou bazi: kod primo ze zapecetene sablony (sdilene stranky), data
/// jako klon s kopii pri zapisu. Kod se zapeceti az po uspesne relokaci
/// ([`seal`]); do te doby jde modul zahodit ([`discard`]).
pub fn load_library(process: &Process, name: &str) -> Res<Module> {
    let (lib, vmos) = match libos::libd::open(name) {
        Ok(r) => r,
        Err(libos::Error::NotFound) => return Err(format!("knihovna {name} nenalezena")),
        Err(e) => return Err(format!("knihovna {name}: {e}")),
    };
    let base = ctx(libos::spawn::random_base(process, lib.span), "misto pro knihovnu")?;
    let fixed = MAP_FIXED | MAP_IN_RESERVATION;
    let mut seal = Vec::new();
    let mapped = (|| -> Res<()> {
        for (seg, vmo) in lib.segments.iter().zip(&vmos) {
            if seg.kind == libdproto::kind::SHARED {
                ctx(process.map(vmo, 0, seg.len, seg.prot | fixed, base + seg.start), "mapovani knihovny")?;
                seal.push((base + seg.start, seg.len, seg.prot));
            } else {
                let own = ctx(vmo.clone_for(0, seg.len, process), "data knihovny")?;
                ctx(process.map(&own, 0, seg.len, seg.prot | fixed, base + seg.start), "mapovani dat")?;
            }
        }
        Ok(())
    })();
    // SAFETY: segmenty jsou namapovane v rezervaci `base..base+span`.
    let m = mapped.and_then(|()| module(name, base, unsafe { module_memory(base, lib.span) }));
    let m = m.and_then(|m| match ctx(m.elf.interp(), name)? {
        Some(_) => Err(format!("{name} neni sdilena knihovna")),
        None => Ok(m),
    });
    match m {
        Ok(mut m) => {
            m.seal = seal;
            Ok(m)
        }
        Err(e) => {
            let _ = libos::vm::release(base, lib.span);
            Err(e)
        }
    }
}

/// Zahodi modul, ktery se nepodarilo dokoncit (jeste nezapeceteny).
pub fn discard(m: &Module) {
    let _ = libos::vm::release(m.base, m.span);
}

/// Definice symbolu `name` v rozsahu `scope` (indexy modulu v poradi
/// hledani): (definice, index modulu).
fn lookup(mods: &[Module], scope: &[usize], name: &str) -> Res<Option<(Def, usize)>> {
    for &i in scope {
        let m = &mods[i];
        let syms = ctx(m.elf.symbols(&m.dynamic), &m.name)?;
        if let Some((_, s)) = ctx(syms.lookup(name), &m.name)?
            && s.is_defined()
            && s.bind != bind::LOCAL
        {
            return Ok(Some((Def { base: m.base, value: s.value, tls: m.tls_def() }, i)));
        }
    }
    Ok(None)
}

/// Symbol pro `dlsym`: adresa v rozsahu `scope`; TLS = (cislo modulu,
/// posun v bloku, umisteni), IFUNC = adresa resitele.
pub enum Found {
    Addr(u64),
    Tls(u64, u64, TlsPlace),
    Ifunc(u64),
}

pub fn find_symbol(mods: &[Module], scope: &[usize], name: &str) -> Res<Option<Found>> {
    for &i in scope {
        let m = &mods[i];
        let syms = ctx(m.elf.symbols(&m.dynamic), &m.name)?;
        let Some((_, s)) = ctx(syms.lookup(name), &m.name)? else { continue };
        if !s.is_defined() || !s.is_exported() {
            continue;
        }
        return Ok(Some(match s.kind {
            kind::TLS => match m.tls_id {
                Some((id, place)) => Found::Tls(id, s.value, place),
                None => return Err(format!("{name}: TLS symbol v modulu bez TLS ({})", m.name)),
            },
            kind::GNU_IFUNC => Found::Ifunc(m.base + s.value),
            _ => Found::Addr(m.base + s.value),
        }));
    }
    Ok(None)
}

/// Funkce zavadece, ktere moduly volaji jmenem.
fn builtin(name: &str) -> Option<u64> {
    match name {
        "__tls_get_addr" => Some(tls::tls_get_addr as *const () as u64),
        _ => crate::dl::builtin(name),
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

/// Relokace modulu `i` (vse hned); symboly v rozsahu `scope`.
pub fn relocate(mods: &[Module], scope: &[usize], i: usize) -> Res<()> {
    let m = &mods[i];
    let syms = ctx(m.elf.symbols(&m.dynamic), &m.name)?;
    for r in ctx(m.elf.relas(&m.dynamic), &m.name)? {
        let r = ctx(r, &m.name)?;
        if r.kind == Kind::None {
            continue;
        }
        // Definice a modul, ktery ji ma (TLS: kde lezi jeho blok).
        let (def, owner) = if r.sym == 0 {
            (None, Some(i))
        } else {
            let s = ctx(syms.get(r.sym as usize), &m.name)?;
            // Lokalni a skryte symboly patri modulu samotnemu.
            if s.bind == bind::LOCAL || (s.is_defined() && !s.is_exported()) {
                (Some(Def { base: m.base, value: s.value, tls: m.tls_def() }), Some(i))
            } else if let Some(a) = builtin(s.name) {
                (Some(Def { base: 0, value: a, tls: None }), None)
            } else {
                match lookup(mods, scope, s.name)? {
                    Some((d, o)) => (Some(d), Some(o)),
                    None if s.is_weak() => (None, None),
                    None => return Err(format!("nedefinovany symbol {} (v {})", s.name, m.name)),
                }
            }
        };
        let place = owner.and_then(|o| mods[o].tls_id).map(|t| t.1);
        let at = m.base + r.offset;
        if place == Some(TlsPlace::Dynamic) {
            match r.kind {
                Kind::TlsTpOffset => {
                    return Err(format!("{}: pristup initial-exec k TLS modulu nahraneho pres dlopen", m.name));
                }
                Kind::TlsDesc => {
                    // Dynamicky deskriptor: argument = [cislo modulu, posun].
                    let id = mods[owner.unwrap_or(i)].tls_id.map_or(0, |t| t.0);
                    let value = def.map_or(0, |d| d.value);
                    let arg = Box::leak(Box::new([id, value.wrapping_add_signed(r.addend)]));
                    // SAFETY: misto relokace (2 slova) v datech modulu.
                    unsafe {
                        put(at, tls::tlsdesc_dynamic as *const () as u64);
                        put(at + 8, arg.as_ptr() as u64);
                    }
                    continue;
                }
                _ => {}
            }
        }
        let tlsdesc = tls::tlsdesc_static as *const () as u64;
        match dynload::compute(r.kind, m.base, r.addend, def, m.tls_def(), tlsdesc) {
            // SAFETY: misto relokace lezi v modulu (zapisovatelny segment
            // nebo RELRO, jeste pred zapecetenim).
            Ok(Apply::Word(v)) => unsafe { put(at, v) },
            Ok(Apply::Desc(f, a)) => unsafe {
                put(at, f);
                put(at + 8, a);
            },
            Ok(Apply::CallIfunc(f)) => {
                // SAFETY: resitel IFUNC modulu (kod modulu, zavislosti uz
                // jsou relokovane).
                let f: extern "C" fn() -> u64 = unsafe { core::mem::transmute(f as usize) };
                unsafe { put(at, f()) };
            }
            Err(e) => return Err(format!("{}: {e}", m.name)),
        }
    }
    for a in ctx(m.elf.relrs(&m.dynamic), &m.name)? {
        let at = m.base + ctx(a, &m.name)?;
        // SAFETY: relativni relokace v datech modulu.
        unsafe { put(at, (at as *const u64).read_unaligned().wrapping_add(m.base)) };
    }
    Ok(())
}

/// RELRO jen pro cteni a zapecetit (nevratne: GOT nejde vratit na RW ani
/// nahradit novym mapovanim). Cele stranky uvnitr oblasti.
pub fn seal_relro(base: u64, r: elf::Relro, page: u64) -> Res<()> {
    let start = (base + r.vaddr).next_multiple_of(page);
    let end = (base + r.vaddr + r.mem_size) & !(page - 1);
    if end > start {
        ctx(libos::vm::seal(start, end - start, MAP_READ), "RELRO")?;
    }
    Ok(())
}

/// Zapeceti kod (segmenty bez zapisu) a RELRO modulu po relokaci.
pub fn seal(m: &mut Module, page: u64) -> Res<()> {
    for (addr, len, prot) in core::mem::take(&mut m.seal) {
        ctx(libos::vm::seal(addr, len, prot), "pecet kodu")?;
    }
    if let Some(r) = ctx(m.elf.relro(), &m.name)? {
        seal_relro(m.base, r, page)?;
    }
    Ok(())
}

/// Inicializatory modulu (`DT_INIT`, `DT_INIT_ARRAY`): adresy uz s bazi.
pub struct Init {
    init: Option<u64>,
    array: (u64, u64),
}

impl Init {
    pub fn of(m: &Module) -> Self {
        let a = m.dynamic.init_array;
        Self { init: m.dynamic.init.map(|f| m.base + f), array: (m.base + a.addr, a.size / 8) }
    }

    /// Zavola inicializatory (bez zamku zavadece: smi volat `dlopen`).
    pub fn run(&self) {
        type F = extern "C" fn(i32, *const u8, *const u8);
        if let Some(f) = self.init {
            // SAFETY: inicializator modulu (relokovany kod modulu).
            let f: F = unsafe { core::mem::transmute(f as usize) };
            f(0, core::ptr::null(), core::ptr::null());
        }
        for i in 0..self.array.1 {
            // SAFETY: pole ukazatelu (relokovane) v modulu.
            let f = unsafe { ((self.array.0 + i * 8) as *const u64).read_unaligned() };
            if f != 0 && f != u64::MAX {
                // SAFETY: inicializator modulu.
                let f: F = unsafe { core::mem::transmute(f as usize) };
                f(0, core::ptr::null(), core::ptr::null());
            }
        }
    }
}

/// Lokalni rozsah modulu `root`: on a jeho zavislosti do sirky (vse uz
/// nahrane).
pub fn group(mods: &[Module], root: usize) -> Res<Vec<usize>> {
    let mut g = alloc::vec![root];
    let mut k = 0;
    while k < g.len() {
        for n in mods[g[k]].needed()? {
            let n = dynload::system_name(&n).unwrap_or(&n).to_string();
            if let Some(j) = mods.iter().position(|m| m.is(&n))
                && !g.contains(&j)
            {
                g.push(j);
            }
        }
        k += 1;
    }
    Ok(g)
}
