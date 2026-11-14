//! Start procesu: startovni zprava a [`Startup`].
//!
//! Vlastni behove prostredi libos (vstupni bod, heap, panic handler) je
//! v `rt_own` a pouziva ho kazdy program `no_std`. Na cili magnumopus
//! (aplikace se `std`) ho dodava `std` pres `osrt` a libos jen cte
//! startovni zpravu ([`startup`]).

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use abi::bootstrap::{self, Entry, Header};

use crate::hw::Resource;
use crate::{Channel, Handle, Process, Vmo, sys};

/// Handle vlastniho procesu pro knihovnu (mapovani pameti).
static SELF_PROCESS: AtomicU32 = AtomicU32::new(0);

/// Syrovy handle vlastniho procesu (platny po startu).
pub fn self_process() -> u32 {
    SELF_PROCESS.load(Ordering::Relaxed)
}

/// Nastavi handle vlastniho procesu bez startovni zpravy. Jen pro zavadec
/// `ld.so`: startovni zpravu necte (patri programu), handle ma ze zpravy
/// zavadece (`abi::bootstrap::LOADER_MAGIC`).
pub fn set_self_process(raw: u32) {
    SELF_PROCESS.store(raw, Ordering::Relaxed);
}

/// Co proces dostal pri startu.
pub struct Startup {
    /// Handle na vlastni proces (mapovani pameti, vlakna).
    pub process: Process,
    /// Obraz initfs (dostava jen init).
    pub initfs: Option<Vmo>,
    /// Framebuffer z firmware (`kind::FRAMEBUFFER`: init od jadra, textova
    /// konzole od initu). Geometrie a prevzeti: [`crate::sys::fb_info`],
    /// [`crate::sys::fb_claim`].
    pub framebuffer: Option<Vmo>,
    /// Handly predane rodicem (`kind::USER` bez stitku), v poradi zpravy.
    pub handles: Vec<Handle>,
    /// Handly se stitkem (`kind::USER` s nenulovym `arg`,
    /// [`crate::spawn::StartHandle::tagged`]): volitelne handly, ktere se
    /// hledaji podle stitku, ne podle poradi ([`Startup::take_tagged`]).
    pub tagged: Vec<(u32, Handle)>,
    /// Pristup k hardwaru (dostava init, ten ho preda spravci zarizeni).
    pub root_resource: Option<Resource>,
    /// Okna ECAM (konfigurace PCI Express): objekt pameti a `arg` ze zpravy
    /// (prvni sbernice | posledni << 8 | segment << 16).
    pub pci_ecam: Vec<(Vmo, u32)>,
    /// Udaje o ACPI od jadra (`kind::ACPI`, [`abi::acpi::Info`]; dostava jen
    /// init, ten je preda sluzbe ACPI).
    pub acpi: Option<Vmo>,
    /// Bootstrap kanal: spojeni s rodicem.
    pub bootstrap: Channel,
    /// Priznaky startu (`abi::bootstrap::flags`; nastavuje je jen jadro
    /// initu, napr. testovaci beh).
    pub flags: u32,
    /// Blok parametru procesu (`kind::PARAMS`: argumenty, prostredi,
    /// pracovni adresar; cte [`crate::params`]).
    pub params: Option<Vmo>,
}

impl Startup {
    /// Vezme handle se stitkem `tag` (prvni takovy); `None` = rodic ho
    /// neposlal.
    pub fn take_tagged(&mut self, tag: u32) -> Option<Handle> {
        let i = self.tagged.iter().position(|(t, _)| *t == tag)?;
        Some(self.tagged.remove(i).1)
    }
}

pub(crate) fn fail(msg: &str) -> ! {
    sys::debug_log(msg);
    sys::process_exit(abi::process::EXIT_PANIC)
}

/// Nejvic handlu ve startovni zprave (= nejvic handlu jedne zpravy kanalu).
const MAX_START_HANDLES: usize = abi::syscall::CHANNEL_MAX_HANDLES;
/// Startovni zprava: hlavicka a polozka na kazdy handle.
const START_DATA_MAX: usize = bootstrap::HEADER_SIZE + MAX_START_HANDLES * bootstrap::ENTRY_SIZE;

/// Handly ze startovni zpravy roztridene podle druhu (bez heapu).
struct Received {
    process: Option<u32>,
    job: Option<u32>,
    flags: u32,
    initfs: Option<u32>,
    framebuffer: Option<u32>,
    acpi: Option<u32>,
    params: Option<u32>,
    resource: Option<u32>,
    ecam: [(u32, u32); 4],
    ecam_count: usize,
    user: [(u32, u32); MAX_START_HANDLES],
    user_count: usize,
}

/// Precte startovni zpravu z bootstrap kanalu (`bootstrap` = syrovy handle
/// z registru pri startu) a sestavi [`Startup`].
///
/// `init_heap(process)` se zavola po precteni zpravy a pred prvni alokaci
/// (handle vlastniho procesu uz plati: [`self_process`]). Vlastni runtime
/// libos v nem pripravi heap; `std` (cil magnumopus) ma alokator, ktery si
/// pamet mapuje sam, a preda prazdnou funkci.
pub fn startup(bootstrap: u64, init_heap: impl FnOnce(u32)) -> Startup {
    let bootstrap = Channel::from_handle(Handle::from_raw(bootstrap as u32));
    // Startovni zprava ceka ve fronte uz pri startu; heap jeste neni, takze
    // ji cteme do bufferu na zasobniku.
    let mut data = [0u8; START_DATA_MAX];
    let mut raw = [0u32; MAX_START_HANDLES];
    let (r, (dlen, hlen)) = sys::channel_read(bootstrap.handle().raw(), &mut data, &mut raw);
    if r.is_err() {
        fail("libos: chybi nebo je prilis velka startovni zprava");
    }
    let Some(header) = Header::from_bytes(&data[..dlen]) else {
        fail("libos: neplatna startovni zprava");
    };

    let mut got = Received {
        process: None,
        job: None,
        flags: 0,
        initfs: None,
        framebuffer: None,
        acpi: None,
        params: None,
        resource: None,
        ecam: [(0, 0); 4],
        ecam_count: 0,
        user: [(0, 0); MAX_START_HANDLES],
        user_count: 0,
    };
    let count = (header.count as usize).min(hlen);
    for (i, &handle) in raw[..count].iter().enumerate() {
        let off = bootstrap::HEADER_SIZE + i * bootstrap::ENTRY_SIZE;
        // Poskozena zprava (kratsi nez pocet polozek) nesmi shodit start.
        let (kind, arg) = data
            .get(off..dlen)
            .and_then(Entry::from_bytes)
            .map_or((0, 0), |e| (e.kind, e.arg));
        match kind {
            bootstrap::kind::SELF_PROCESS => {
                got.process = Some(handle);
                got.flags = arg;
            }
            bootstrap::kind::JOB => got.job = Some(handle),
            bootstrap::kind::INITFS => got.initfs = Some(handle),
            bootstrap::kind::FRAMEBUFFER => got.framebuffer = Some(handle),
            bootstrap::kind::ACPI => got.acpi = Some(handle),
            bootstrap::kind::PARAMS => got.params = Some(handle),
            #[cfg(feature = "libd")]
            bootstrap::kind::LIBD => crate::libd::set_connection(Channel::from_handle(Handle::from_raw(handle))),
            // Bez klienta libd (sestaveni pro std) spojeni zavrit: nesmi se
            // dostat mezi handly programu a posunout jejich poradi.
            #[cfg(not(feature = "libd"))]
            bootstrap::kind::LIBD => {
                let _ = sys::handle_close(handle);
            }
            bootstrap::kind::ROOT_RESOURCE => got.resource = Some(handle),
            bootstrap::kind::PCI_ECAM if got.ecam_count < got.ecam.len() => {
                got.ecam[got.ecam_count] = (handle, arg);
                got.ecam_count += 1;
            }
            _ if got.user_count < got.user.len() => {
                got.user[got.user_count] = (handle, arg);
                got.user_count += 1;
            }
            _ => {}
        }
    }
    let Some(process) = got.process else {
        fail("libos: startovni zprava neobsahuje handle procesu");
    };

    SELF_PROCESS.store(process, Ordering::Relaxed);
    if let Some(job) = got.job {
        crate::job::set_current(job);
    }
    init_heap(process);
    Startup {
        process: Process::from_handle(Handle::from_raw(process)),
        initfs: got.initfs.map(|h| Vmo::from_handle(Handle::from_raw(h))),
        framebuffer: got.framebuffer.map(|h| Vmo::from_handle(Handle::from_raw(h))),
        handles: got.user[..got.user_count]
            .iter()
            .filter(|&&(_, tag)| tag == 0)
            .map(|&(h, _)| Handle::from_raw(h))
            .collect(),
        tagged: got.user[..got.user_count]
            .iter()
            .filter(|&&(_, tag)| tag != 0)
            .map(|&(h, tag)| (tag, Handle::from_raw(h)))
            .collect(),
        root_resource: got
            .resource
            .map(|h| Resource::from_handle(Handle::from_raw(h))),
        pci_ecam: got.ecam[..got.ecam_count]
            .iter()
            .map(|&(h, arg)| (Vmo::from_handle(Handle::from_raw(h)), arg))
            .collect(),
        acpi: got.acpi.map(|h| Vmo::from_handle(Handle::from_raw(h))),
        bootstrap,
        flags: got.flags,
        params: got.params.map(|h| Vmo::from_handle(Handle::from_raw(h))),
    }
}

