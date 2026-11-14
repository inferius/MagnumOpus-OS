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

use alloc::sync::Arc;
use alloc::vec::Vec;

use abi::bootstrap::{self, Entry, Header};
use abi::vm::{MAP_EXECUTE, MAP_FIXED, MAP_READ, MAP_WRITE};
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
    let elf = elf::ElfFile::parse(image).map_err(|_| Error::Corrupted)?;
    let process = Process::create(job, name)?;
    if let Some(q) = opts.qos {
        crate::sched::set_process(&process, q)?;
    }
    // Segmenty se zarovnavaji na stranku potomka (muze mit jinou nez my).
    // Segmenty s ruznymi pravy ve stejne strance (program linkovany na
    // mensi stranku) skonci chybou mapovani, ne smichanymi pravy.
    let page = crate::vm::page_size_of(&process)?;

    match crate::initfs::offset_of(image) {
        Some(off) => {
            let t = template(off, image, &elf, page)?;
            for seg in &t.segments {
                if seg.flags & MAP_WRITE != 0 {
                    // Data: vlastni klon s kopii pri zapisu (ucet potomka).
                    let own = seg.vmo.clone_for(0, seg.len, &process)?;
                    process.map(&own, 0, seg.len, seg.flags | MAP_FIXED, seg.start)?;
                } else {
                    // Kod a konstanty: primo sdilene stranky sablony.
                    process.map(&seg.vmo, 0, seg.len, seg.flags | MAP_FIXED, seg.start)?;
                }
            }
        }
        None => {
            for seg in elf.load_segments() {
                let seg = seg.map_err(|_| Error::Corrupted)?;
                let start = seg.vaddr & !(page - 1);
                let end = (seg.vaddr + seg.mem_size).next_multiple_of(page);
                let vmo = Vmo::create_for(end - start, &process)?;
                vmo.write(seg.vaddr - start, elf.segment_data(&seg))?;
                let flags = segment_flags(&seg);
                if flags & MAP_WRITE == 0 {
                    vmo.seal()?;
                }
                process.map(&vmo, 0, end - start, flags | MAP_FIXED, start)?;
            }
        }
    }

    let stack_size = opts.stack_size.max(page).next_multiple_of(page);
    let stack = Vmo::create_for(stack_size, &process)?;
    // Pod zasobnikem ochranna oblast (preteceni = hlaska, ne cizi pamet).
    let (_, _, stack_top) = crate::vm::map_stack(&process, &stack, stack_size)?;

    let (parent_end, child_end) = Channel::create()?;
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
    process.start(&thread, elf.entry, stack_top, child_end.into_handle())?;
    Ok((process, parent_end))
}
