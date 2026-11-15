//! Spusteni programu (ELF) v novem procesu.
//!
//! Jadro o formatu programu nic nevi: libos vytvori prazdny proces,
//! namapuje do nej segmenty programu a zasobnik a spusti prvni vlakno se
//! startovni zpravou (`abi::bootstrap`).
//!
//! Proces vznika v jobu (`spawn` = stejny job jako rodic, `spawn_in` =
//! zadany) a pamet programu (segmenty, zasobnik) se strhava z rozpoctu
//! jobu potomka, ne rodice. Potomek dostane svuj job bez prava `WRITE`.
//!
//! **Sdileny kod:** program z initfs (nemenny obsah, `initfs::offset_of`)
//! se nahraje jednou do sablony (objekt pameti na segment, na ucet
//! spoustejiciho). Segmenty bez zapisu (kod, konstanty) se mapuji primo ze
//! sablony - vsechny procesy stejneho programu sdileji tytez stranky.
//! Zapisovatelne segmenty dostane kazdy proces jako klon s kopii pri
//! zapisu (`VMO_CLONE`, na ucet jobu potomka). Program odjinud (napr.
//! nacteny ze souboru do haldy) se nahrava jako drive, bez sdileni.
//!
//! **Program se zavadecem** (`ET_DYN` s `PT_INTERP` = `/system/lib/ld.so`,
//! `docs/cs/dynamicke-linkovani.md`): program a zavadec (sablona od sluzby
//! knihoven `libd`, sdileny kod) se namapuji na nahodne baze (ASLR), jejich
//! kod se zapeceti, na vrchol zasobniku jde auxv (`abi::auxv`) a do
//! bootstrap kanalu pred startovni zpravu zprava zavadece (handle procesu
//! a spojeni s libd). Vlakno zacne na vstupu zavadece; ten nahraje knihovny
//! a skoci na vstup programu.
//!
//! Relaci s libd a listek k ni (`bootstrap::kind::LIBD`, arg 0 relace,
//! arg 1 listek) dostane jen zavadec programu se zavadecem a staticky
//! program s [`Options::libd`] (spoustece). Staticky program bez volby
//! zadne spojeni nema a spawn kvuli nemu s libd ani supervisorem
//! nemluvi.

use alloc::sync::Arc;
use alloc::vec::Vec;

use abi::bootstrap::{self, Entry, Header};
use abi::vm::{MAP_EXECUTE, MAP_FIXED, MAP_IN_RESERVATION, MAP_READ, MAP_WRITE};
use abi::{Error, Result};

use crate::job::{CHILD_RIGHTS, Job};
use crate::sync::Mutex;
use crate::{Channel, Handle, Process, Thread, Vmo};

/// Segment sablony: kam patri, prava a objekt s obsahem.
struct Segment {
    start: u64,
    len: u64,
    flags: u32,
    vmo: Vmo,
}

/// Sablona programu z initfs: klic (offset v initfs, delka obrazu,
/// velikost stranky potomka) a segmenty.
struct Template {
    key: (u64, u64, u64),
    segments: Vec<Segment>,
}

/// Sablony uz spustenych programu z initfs (zijou do konce procesu).
static TEMPLATES: Mutex<Vec<Arc<Template>>> = Mutex::new(Vec::new());

/// Pocet sablon sdilenych programu (pro testy a prehled).
pub fn shared_images() -> usize {
    TEMPLATES.lock().len()
}

/// Prava segmentu ELF jako `MAP_*`.
fn segment_flags(seg: &elf::Segment) -> u32 {
    let mut flags = MAP_READ;
    if seg.flags & elf::PF_W != 0 {
        flags |= MAP_WRITE;
    }
    if seg.flags & elf::PF_X != 0 {
        flags |= MAP_EXECUTE;
    }
    flags
}

/// Sablona programu `image` (offset `off` v initfs) pro stranku `page`;
/// vytvori ji pri prvnim spusteni.
fn template(off: u64, image: &[u8], elf: &elf::ElfFile, page: u64) -> Result<Arc<Template>> {
    let key = (off, image.len() as u64, page);
    if let Some(t) = TEMPLATES.lock().iter().find(|t| t.key == key) {
        return Ok(t.clone());
    }
    let mut segments = Vec::new();
    for seg in elf.load_segments() {
        let seg = seg.map_err(|_| Error::Corrupted)?;
        let start = seg.vaddr & !(page - 1);
        let end = (seg.vaddr + seg.mem_size).next_multiple_of(page);
        let vmo = Vmo::create(end - start)?;
        vmo.write(seg.vaddr - start, elf.segment_data(&seg))?;
        let flags = segment_flags(&seg);
        // Kod a konstanty: pecet (spustit jde jen zapecetena pamet).
        if flags & MAP_WRITE == 0 {
            vmo.seal()?;
        }
        segments.push(Segment { start, len: end - start, flags, vmo });
    }
    let t = Arc::new(Template { key, segments });
    let mut all = TEMPLATES.lock();
    // Soubezne spusteni stejneho programu: plati prvni sablona.
    if let Some(other) = all.iter().find(|o| o.key == key) {
        return Ok(other.clone());
    }
    all.push(t.clone());
    Ok(t)
}

/// Namapuje segmenty `elf` do `process` posunute o `base` (0 = ET_EXEC na
/// pevnych adresach); `reserved` = segmenty lezi v rezervaci modulu.
/// Program z initfs sdili stranky kodu pres sablonu.
fn map_image(process: &Process, image: &[u8], elf: &elf::ElfFile, page: u64, base: u64, reserved: bool) -> Result<()> {
    let fixed = MAP_FIXED | if reserved { MAP_IN_RESERVATION } else { 0 };
    match crate::initfs::offset_of(image) {
        Some(off) => {
            let t = template(off, image, elf, page)?;
            for seg in &t.segments {
                if seg.flags & MAP_WRITE != 0 {
                    // Data: vlastni klon s kopii pri zapisu (ucet potomka).
                    let own = seg.vmo.clone_for(0, seg.len, process)?;
                    process.map(&own, 0, seg.len, seg.flags | fixed, base + seg.start)?;
                } else {
                    // Kod a konstanty: primo sdilene stranky sablony.
                    process.map(&seg.vmo, 0, seg.len, seg.flags | fixed, base + seg.start)?;
                }
            }
        }
        None => {
            for seg in elf.load_segments() {
                let seg = seg.map_err(|_| Error::Corrupted)?;
                let start = seg.vaddr & !(page - 1);
                let end = (seg.vaddr + seg.mem_size).next_multiple_of(page);
                let vmo = Vmo::create_for(end - start, process)?;
                vmo.write(seg.vaddr - start, elf.segment_data(&seg))?;
                let flags = segment_flags(&seg);
                if flags & MAP_WRITE == 0 {
                    vmo.seal()?;
                }
                process.map(&vmo, 0, end - start, flags | fixed, base + start)?;
            }
        }
    }
    Ok(())
}

/// Zapeceti segmenty bez zapisu (kod a konstanty) modulu na `base`: jejich
/// mapovani uz nejde nahradit ani zmenit (`PROT_SEAL`).
fn seal_readonly(process: &Process, elf: &elf::ElfFile, page: u64, base: u64) -> Result<()> {
    for seg in elf.load_segments() {
        let seg = seg.map_err(|_| Error::Corrupted)?;
        let flags = segment_flags(&seg);
        if flags & MAP_WRITE == 0 {
            let start = seg.vaddr & !(page - 1);
            let end = (seg.vaddr + seg.mem_size).next_multiple_of(page);
            process.protect(base + start, end - start, flags | abi::vm::PROT_SEAL)?;
        }
    }
    Ok(())
}

/// Namapuje knihovnu od libd (zavadec `ld.so`) na nahodnou bazi v `process`:
/// kod primo ze zapecetene sablony (sdilene stranky, zapeceti se),
/// data jako klon s kopii pri zapisu. Vraci bazi.
#[cfg(feature = "libd")]
fn map_library(process: &Process, lib: &libdproto::Library, vmos: &[Vmo]) -> Result<u64> {
    let base = random_base(process, lib.span)?;
    let fixed = MAP_FIXED | MAP_IN_RESERVATION;
    for (seg, vmo) in lib.segments.iter().zip(vmos) {
        if seg.kind == libdproto::kind::SHARED {
            process.map(vmo, 0, seg.len, seg.prot | fixed, base + seg.start)?;
            process.protect(base + seg.start, seg.len, seg.prot | abi::vm::PROT_SEAL)?;
        } else {
            let own = vmo.clone_for(0, seg.len, process)?;
            process.map(&own, 0, seg.len, seg.prot | fixed, base + seg.start)?;
        }
    }
    Ok(base)
}

/// Rozsah adres modulu ET_DYN (od 0 po konec posledniho segmentu,
/// zaokrouhleno na stranku).
fn image_span(elf: &elf::ElfFile, page: u64) -> Result<u64> {
    let mut end = 0;
    for seg in elf.load_segments() {
        let seg = seg.map_err(|_| Error::Corrupted)?;
        end = end.max(seg.vaddr + seg.mem_size);
    }
    if end == 0 {
        return Err(Error::Corrupted);
    }
    Ok(end.next_multiple_of(page.max(abi::vm::MAX_PAGE_SIZE)))
}

/// Zarezervuje v `process` `len` B na nahodne bazi zarovnane na 64 KiB
/// v `abi::auxv::ASLR_START..ASLR_END` (obsazene misto = dalsi pokus).
pub fn random_base(process: &Process, len: u64) -> Result<u64> {
    use abi::auxv::{ASLR_ALIGN, ASLR_END, ASLR_START};
    let slots = (ASLR_END - ASLR_START - len) / ASLR_ALIGN;
    for _ in 0..64 {
        let (r, _) = crate::random::u64();
        let base = ASLR_START + (r % slots) * ASLR_ALIGN;
        if process.reserve(base, len, MAP_FIXED).is_ok() {
            return Ok(base);
        }
    }
    Err(Error::NoResources)
}

/// Vychozi velikost zasobniku hlavniho vlakna noveho procesu.
pub const STACK_SIZE: u64 = 256 * 1024;

/// Volby spusteni.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Zasobnik hlavniho vlakna (zaokrouhli se na stranku potomka).
    pub stack_size: u64,
    /// Trida planovace vsech vlaken procesu (nastavi se pred startem).
    pub qos: Option<crate::sched::Qos>,
    /// Rezerva statickeho TLS pro `dlopen` modulu s initial-exec (manifest
    /// `[app] static_tls_reserve`; `None` = vychozi 2 KiB, zavadec omezi na
    /// 64 KiB). Jen program se zavadecem.
    pub static_tls_reserve: Option<u64>,
    /// Rozsahy adres `(zacatek, delka)` rezervovane v potomkovi pred
    /// namapovanim programu, zavadece a zasobniku (manifest `[app]
    /// reserve`, spoustec Windows vrstvy); delka 0 = volne misto. Program je
    /// najde v auxv `AT_MO_RESERVE` (`loader::reserved`). Jen program se
    /// zavadecem (staticky = `NOT_SUPPORTED`); zarovnani na stranku
    /// potomka, prekryv nebo rozsah mimo pamet procesu = `INVALID_ARGS`.
    pub reserve: [(u64, u64); abi::loader::RESERVE_MAX],
    /// Staticky program dostane relaci s libd a listek (smi spoustet
    /// programy se zavadecem: shell, launchd, relace). Program se zavadecem
    /// relaci ma vzdy (pres zavadec).
    pub libd: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            stack_size: STACK_SIZE,
            qos: None,
            static_tls_reserve: None,
            reserve: [(0, 0); abi::loader::RESERVE_MAX],
            libd: false,
        }
    }
}

/// Potrebuje obraz zavadec (`ET_DYN` s `PT_INTERP`)? Spoustec podle toho
/// pozna, jestli ma pripravit [`Origin`] (napr. novou relaci FS).
pub fn needs_loader(image: &[u8]) -> bool {
    elf::ElfFile::parse_object(image, elf::Machine::NATIVE)
        .is_ok_and(|e| e.elf_type == elf::ElfType::Dyn && e.interp().ok().flatten().is_some())
}

/// Program ze souboru (DL-4c): relace FS pro zavadec (knihovny z disku,
/// `$ORIGIN`) a absolutni cesta programu. Relace ma mit identitu a cesty
/// potomka (typicky `CONNECT` na relaci, kterou potomek dostane); zavadec
/// s ni cte knihovny s pravy aplikace. Jen program se zavadecem (statickemu
/// se nepreda nic).
pub struct Origin {
    /// Konec kanalu relace FS.
    pub fs: Handle,
    /// Absolutni cesta souboru programu (nejvys `bootstrap::MAX_ORIGIN_PATH`).
    pub path: alloc::string::String,
}

/// Handle predavany novemu procesu ve startovni zprave.
pub struct StartHandle {
    pub kind: u32,
    pub arg: u32,
    pub handle: Handle,
}

impl StartHandle {
    /// Handle podle dohody s programem (`kind::USER`).
    pub fn user(handle: Handle) -> Self {
        Self {
            kind: bootstrap::kind::USER,
            arg: 0,
            handle,
        }
    }

    /// Volitelny handle se stitkem `tag` (nenulovym): program ho najde
    /// pres `Startup::take_tagged` bez ohledu na poradi a na to, ktere
    /// jine volitelne handly chybi.
    pub fn tagged(tag: u32, handle: Handle) -> Self {
        assert!(tag != 0, "stitek 0 = handle bez stitku");
        Self { kind: bootstrap::kind::USER, arg: tag, handle }
    }

    pub fn new(kind: u32, arg: u32, handle: Handle) -> Self {
        Self { kind, arg, handle }
    }
}

/// Spusti program `image` (ELF) jako proces `name` ve stejnem jobu jako
/// volajici a preda mu handly `handles` ve startovni zprave (vlastni proces
/// a job doplni sam). Vraci proces a konec bootstrap kanalu rodice (pro
/// dalsi komunikaci s potomkem).
pub fn spawn(name: &str, image: &[u8], handles: Vec<StartHandle>) -> Result<(Process, Channel)> {
    let job = crate::job::current()?;
    spawn_in(&job, name, image, handles)
}

/// Jako [`spawn`] s vychozi tridou systemove sluzby podle jmena
/// ([`crate::sched::service_default`]).
pub fn service(name: &str, image: &[u8], handles: Vec<StartHandle>) -> Result<(Process, Channel)> {
    let job = crate::job::current()?;
    let opts = Options {
        qos: crate::sched::service_default(name),
        ..Options::default()
    };
    spawn_with(&job, name, image, handles, opts)
}

/// Jako [`spawn`], ale v jobu `job` (napr. podrizenem s limity).
pub fn spawn_in(
    job: &Job,
    name: &str,
    image: &[u8],
    handles: Vec<StartHandle>,
) -> Result<(Process, Channel)> {
    spawn_with(job, name, image, handles, Options::default())
}

/// Jako [`spawn_in`] s volbami (velikost zasobniku).
pub fn spawn_with(
    job: &Job,
    name: &str,
    image: &[u8],
    handles: Vec<StartHandle>,
    opts: Options,
) -> Result<(Process, Channel)> {
    spawn_with_thread(job, name, image, handles, opts).map(|(p, _, ch)| (p, ch))
}

/// Jako [`spawn_with`], navic vrati handle hlavniho vlakna (debugger,
/// pozastaveni a kontext vlakna).
pub fn spawn_with_thread(
    job: &Job,
    name: &str,
    image: &[u8],
    handles: Vec<StartHandle>,
    opts: Options,
) -> Result<(Process, Thread, Channel)> {
    spawn_from(job, name, image, handles, opts, None)
}

/// Jako [`spawn_with_thread`] pro program ze souboru: zavadec dostane
/// `origin` (relaci FS a cestu programu) a nahraje knihovny i z disku
/// (`DT_RUNPATH` s `$ORIGIN`, absolutni cesty; `docs/cs/dynamicke-linkovani.md`).
pub fn spawn_from(
    job: &Job,
    name: &str,
    image: &[u8],
    handles: Vec<StartHandle>,
    opts: Options,
    origin: Option<Origin>,
) -> Result<(Process, Thread, Channel)> {
    if let Some(o) = &origin
        && (o.path.len() > bootstrap::MAX_ORIGIN_PATH || !o.path.starts_with('/'))
    {
        return Err(Error::InvalidArgs);
    }
    let elf = elf::ElfFile::parse_object(image, elf::Machine::NATIVE).map_err(|_| Error::Corrupted)?;
    // Sdilena knihovna (ET_DYN bez zavadece) neni program.
    let dynamic = elf.elf_type == elf::ElfType::Dyn;
    if dynamic && elf.interp().map_err(|_| Error::Corrupted)? != Some(abi::auxv::INTERP) {
        return Err(Error::NotSupported);
    }
    let process = Process::create(job, name)?;
    if let Some(q) = opts.qos {
        crate::sched::set_process(&process, q)?;
    }
    // Segmenty se zarovnavaji na stranku potomka (muze mit jinou nez my).
    // Segmenty s ruznymi pravy ve stejne strance (program linkovany na
    // mensi stranku) skonci chybou mapovani, ne smichanymi pravy.
    let page = crate::vm::page_size_of(&process)?;

    // Rezervace adres jako prvni: nahodne baze modulu i zasobnik se jim
    // vyhnou (T5, docs/cs/dynamicke-linkovani.md).
    let reserve = reserve_ranges(&process, &opts.reserve, page, dynamic)?;

    // Program se zavadecem: oba na nahodne baze, auxv pro zavadec.
    let mut loader = None;
    if dynamic {
        let mut l = map_with_loader(&process, image, &elf, page)?;
        l.tls_reserve = opts.static_tls_reserve;
        l.reserve = reserve;
        loader = Some(l);
    } else {
        map_image(&process, image, &elf, page, 0, false)?;
    }

    let stack_size = opts.stack_size.max(page).next_multiple_of(page);
    let stack = Vmo::create_for(stack_size, &process)?;
    // Pod zasobnikem ochranna oblast (preteceni = hlaska, ne cizi pamet).
    let (_, _, stack_top) = crate::vm::map_stack(&process, &stack, stack_size)?;
    // auxv na vrchol zasobniku (jen program se zavadecem).
    let (entry, sp) = match &loader {
        Some(l) => {
            let block = l.auxv(stack_top, page);
            stack.write(stack_size - block.len() as u64, &block)?;
            (l.ld_entry, stack_top - block.len() as u64)
        }
        None => (elf.entry, stack_top),
    };

    let (parent_end, child_end) = Channel::create()?;
    if loader.is_some() {
        // Zprava zavadece pred startovni zpravou: handle procesu a spojeni
        // s libd (knihovny). Startovni zprava a kanal zustanou programu.
        // Program ze souboru: navic relace FS a za polozkami jeho cesta.
        let mut d = Vec::new();
        let (session, lease) = child_libd()?;
        let count = if origin.is_some() { 4 } else { 3 };
        let h = Header { magic: bootstrap::LOADER_MAGIC, version: bootstrap::VERSION, count, _reserved: 0 };
        d.extend_from_slice(&h.to_bytes());
        d.extend_from_slice(&Entry { kind: bootstrap::kind::SELF_PROCESS, arg: 0 }.to_bytes());
        d.extend_from_slice(&Entry { kind: bootstrap::kind::LIBD, arg: 0 }.to_bytes());
        d.extend_from_slice(&Entry { kind: bootstrap::kind::LIBD, arg: 1 }.to_bytes());
        let mut hs = alloc::vec![process.handle().duplicate(None)?, session, lease];
        if let Some(o) = origin {
            d.extend_from_slice(&Entry { kind: bootstrap::kind::FS, arg: o.path.len() as u32 }.to_bytes());
            d.extend_from_slice(o.path.as_bytes());
            hs.push(o.fs);
        }
        parent_end.write(&d, hs)?;
    }
    // Spoustec (staticky program s volbou): relace a listek ve startovni
    // zprave.
    let mut handles = handles;
    if opts.libd && loader.is_none() {
        let (session, lease) = child_libd()?;
        handles.push(StartHandle::new(bootstrap::kind::LIBD, 0, session));
        handles.push(StartHandle::new(bootstrap::kind::LIBD, 1, lease));
    }
    let mut data = Vec::new();
    let header = Header {
        magic: bootstrap::MAGIC,
        version: bootstrap::VERSION,
        count: 2 + handles.len() as u32,
        _reserved: 0,
    };
    data.extend_from_slice(&header.to_bytes());
    data.extend_from_slice(
        &Entry {
            kind: bootstrap::kind::SELF_PROCESS,
            arg: 0,
        }
        .to_bytes(),
    );
    data.extend_from_slice(
        &Entry {
            kind: bootstrap::kind::JOB,
            arg: 0,
        }
        .to_bytes(),
    );
    let mut all = Vec::with_capacity(2 + handles.len());
    all.push(process.handle().duplicate(None)?);
    all.push(job.duplicate(Some(CHILD_RIGHTS))?.into_handle());
    for h in handles {
        data.extend_from_slice(
            &Entry {
                kind: h.kind,
                arg: h.arg,
            }
            .to_bytes(),
        );
        all.push(h.handle);
    }
    parent_end.write(&data, all)?;

    let thread = Thread::create(&process, name)?;
    process.start(&thread, entry, sp, child_end.into_handle())?;
    Ok((process, thread, parent_end))
}

/// Rezervuje rozsahy `opts.reserve` v potomkovi (pevne adresy). Vraci
/// pouzite rozsahy (pro auxv).
fn reserve_ranges(
    process: &Process,
    ranges: &[(u64, u64); abi::loader::RESERVE_MAX],
    page: u64,
    dynamic: bool,
) -> Result<Vec<(u64, u64)>> {
    let used: Vec<(u64, u64)> = ranges.iter().copied().filter(|&(_, len)| len != 0).collect();
    if used.is_empty() {
        return Ok(used);
    }
    // Staticky program auxv nema: rezervace by nenasel.
    if !dynamic {
        return Err(Error::NotSupported);
    }
    for &(start, len) in &used {
        let end = start.checked_add(len).ok_or(Error::InvalidArgs)?;
        if start % page != 0 || len % page != 0 || start < abi::vm::USER_MIN || end > abi::vm::USER_MAX {
            return Err(Error::InvalidArgs);
        }
        // Prekryv s jinym rozsahem jadro odmitne (pevna adresa).
        process.reserve(start, len, MAP_FIXED).map_err(|e| if e == Error::NoMemory { e } else { Error::InvalidArgs })?;
    }
    Ok(used)
}

/// Udaje pro zavadec programu (auxv).
struct Loader {
    phdr: u64,
    phnum: u64,
    entry: u64,
    ld_base: u64,
    ld_entry: u64,
    /// `AT_MO_TLS_RESERVE` (`None` = vychozi zavadece).
    tls_reserve: Option<u64>,
    /// `AT_MO_RESERVE`: rezervovane rozsahy (prazdne = polozka chybi).
    reserve: Vec<(u64, u64)>,
}

impl Loader {
    /// Blok na vrchol zasobniku (`abi::auxv`): argc, argv, envp, auxv a 16 B
    /// nahody; delka nasobek 16 (SP zarovnany jako po startu SysV).
    fn auxv(&self, stack_top: u64, page: u64) -> Vec<u8> {
        use abi::auxv::*;
        let pairs = [
            (AT_PHDR, self.phdr),
            (AT_PHENT, 56),
            (AT_PHNUM, self.phnum),
            (AT_PAGESZ, page),
            (AT_BASE, self.ld_base),
            (AT_ENTRY, self.entry),
            (AT_RANDOM, 0),
            // Misto pro tabulku zavadece (vyplni ld.so, abi::loader).
            (abi::loader::AT_MO_LOADER, 0),
            // Rezerva statickeho TLS (bez volby AT_IGNORE).
            match self.tls_reserve {
                Some(r) => (abi::loader::AT_MO_TLS_RESERVE, r),
                None => (AT_IGNORE, 0),
            },
            // Tabulka rezervaci za auxv (adresa doplnena nize).
            if self.reserve.is_empty() { (AT_IGNORE, 0) } else { (abi::loader::AT_MO_RESERVE, 0) },
            (AT_NULL, 0),
        ];
        let table = if self.reserve.is_empty() { 0 } else { 1 + 2 * self.reserve.len() };
        let words = 3 + 2 * pairs.len() + table;
        let len = (words * 8 + 16).next_multiple_of(16);
        let random_at = stack_top - 16;
        let table_at = stack_top - len as u64 + ((3 + 2 * pairs.len()) * 8) as u64;
        let mut b = Vec::with_capacity(len);
        b.resize(len, 0);
        let mut put = |i: usize, v: u64| b[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
        for (k, (t, v)) in pairs.iter().enumerate() {
            put(3 + 2 * k, *t);
            let v = match *t {
                AT_RANDOM => random_at,
                abi::loader::AT_MO_RESERVE => table_at,
                _ => *v,
            };
            put(4 + 2 * k, v);
        }
        if table != 0 {
            let at = 3 + 2 * pairs.len();
            put(at, self.reserve.len() as u64);
            for (i, &(start, rlen)) in self.reserve.iter().enumerate() {
                put(at + 1 + 2 * i, start);
                put(at + 2 + 2 * i, rlen);
            }
        }
        let mut r = [0u8; 16];
        crate::random::fill(&mut r);
        b[len - 16..].copy_from_slice(&r);
        b
    }
}

/// Relace s libd a listek pro potomka (`libd::child`: init primo od
/// supervisoru, jinak `NARROW` na vlastnim listku).
fn child_libd() -> Result<(Handle, Handle)> {
    #[cfg(feature = "libd")]
    {
        let (s, l) = crate::libd::child()?;
        return Ok((s.into_handle(), l.into_handle()));
    }
    #[allow(unreachable_code)]
    Err(Error::NotSupported)
}

/// Sablona zavadece `ld.so` od libd (mezipamet procesu: VMO jsou
/// zapecetene a nemenne, platne i po restartu libd).
#[cfg(feature = "libd")]
static LD_TEMPLATE: crate::sync::Mutex<Option<(libdproto::Library, Vec<Vmo>)>> = crate::sync::Mutex::new(None);

/// Program se zavadecem: program (sablona z initfs volajiciho, nebo kopie)
/// a zavadec `ld.so` (sablona od libd) na nahodne baze, kod obou
/// zapeceteny. Bez spojeni s libd = `NOT_SUPPORTED`.
fn map_with_loader(process: &Process, image: &[u8], elf: &elf::ElfFile, page: u64) -> Result<Loader> {
    #[cfg(feature = "libd")]
    {
        let mut cache = LD_TEMPLATE.lock();
        if cache.is_none() {
            let ld_name = abi::auxv::INTERP.trim_start_matches(abi::auxv::SYSTEM_LIB);
            *cache = Some(crate::libd::open(ld_name)?);
        }
        let (lib, vmos) = cache.as_ref().ok_or(Error::NotFound)?;
        let base = random_base(process, image_span(elf, page)?)?;
        map_image(process, image, elf, page, base, true)?;
        seal_readonly(process, elf, page, base)?;
        let ld_base = map_library(process, lib, vmos)?;
        // Vstup zavadece: z hlavicky ELF v jeho prvnim (sdilenem) segmentu.
        let mut hdr = [0u8; 64];
        vmos.first().ok_or(Error::Corrupted)?.read(0, &mut hdr)?;
        let ld_entry = u64::from_le_bytes(hdr[24..32].try_into().map_err(|_| Error::Corrupted)?);
        return Ok(Loader {
            phdr: base + elf.phdr_vaddr().map_err(|_| Error::Corrupted)?,
            phnum: u64::from(elf.phnum()),
            entry: base + elf.entry,
            ld_base,
            ld_entry: ld_base + ld_entry,
            tls_reserve: None,
            reserve: Vec::new(),
        });
    }
    #[allow(unreachable_code)]
    {
        let _ = (process, image, elf, page);
        Err(Error::NotSupported)
    }
}
