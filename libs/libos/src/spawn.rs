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
//! Ma-li volajici spojeni s libd, dostane kazdy potomek vlastni
//! (`bootstrap::kind::LIBD`) - muze tak spoustet dalsi programy se
//! zavadecem.

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
}

impl Default for Options {
    fn default() -> Self {
        Self {
            stack_size: STACK_SIZE,
            qos: None,
        }
    }
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

    // Program se zavadecem: oba na nahodne baze, auxv pro zavadec.
    let mut loader = None;
    if dynamic {
        loader = Some(map_with_loader(&process, image, &elf, page)?);
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
        let mut d = Vec::new();
        let h = Header { magic: bootstrap::LOADER_MAGIC, version: bootstrap::VERSION, count: 2, _reserved: 0 };
        d.extend_from_slice(&h.to_bytes());
        d.extend_from_slice(&Entry { kind: bootstrap::kind::SELF_PROCESS, arg: 0 }.to_bytes());
        d.extend_from_slice(&Entry { kind: bootstrap::kind::LIBD, arg: 0 }.to_bytes());
        parent_end.write(&d, alloc::vec![process.handle().duplicate(None)?, loader_libd()?])?;
    }
    // Spojeni s libd pro potomka (smi spoustet programy se zavadecem).
    let mut handles = handles;
    if let Some(ch) = child_libd() {
        handles.push(StartHandle::new(bootstrap::kind::LIBD, 0, ch));
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

/// Udaje pro zavadec programu (auxv).
struct Loader {
    phdr: u64,
    phnum: u64,
    entry: u64,
    ld_base: u64,
    ld_entry: u64,
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
            (AT_NULL, 0),
        ];
        let words = 3 + 2 * pairs.len();
        let len = (words * 8 + 16).next_multiple_of(16);
        let random_at = stack_top - 16;
        let mut b = Vec::with_capacity(len);
        b.resize(len, 0);
        let mut put = |i: usize, v: u64| b[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
        for (k, (t, v)) in pairs.iter().enumerate() {
            put(3 + 2 * k, *t);
            put(4 + 2 * k, if *t == AT_RANDOM { random_at } else { *v });
        }
        let mut r = [0u8; 16];
        crate::random::fill(&mut r);
        b[len - 16..].copy_from_slice(&r);
        b
    }
}

/// Spojeni s libd pro potomka (`None` = volajici spojeni nema nebo libd
/// neodpovida - potomek pak programy se zavadecem nespusti).
fn child_libd() -> Option<Handle> {
    #[cfg(feature = "libd")]
    {
        if crate::libd::available() {
            return crate::libd::connect().ok().map(Channel::into_handle);
        }
    }
    None
}

/// Spojeni s libd pro zavadec potomka.
fn loader_libd() -> Result<Handle> {
    #[cfg(feature = "libd")]
    {
        return crate::libd::connect().map(Channel::into_handle);
    }
    #[allow(unreachable_code)]
    Err(Error::NotSupported)
}

/// Program se zavadecem: program (sablona z initfs volajiciho, nebo kopie)
/// a zavadec `ld.so` (sablona od libd) na nahodne baze, kod obou
/// zapeceteny. Bez spojeni s libd = `NOT_SUPPORTED`.
fn map_with_loader(process: &Process, image: &[u8], elf: &elf::ElfFile, page: u64) -> Result<Loader> {
    #[cfg(feature = "libd")]
    {
        let ld_name = abi::auxv::INTERP.trim_start_matches(abi::auxv::SYSTEM_LIB);
        let (lib, vmos) = crate::libd::open(ld_name)?;
        let base = random_base(process, image_span(elf, page)?)?;
        map_image(process, image, elf, page, base, true)?;
        seal_readonly(process, elf, page, base)?;
        let ld_base = map_library(process, &lib, &vmos)?;
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
        });
    }
    #[allow(unreachable_code)]
    {
        let _ = (process, image, elf, page);
        Err(Error::NotSupported)
    }
}
