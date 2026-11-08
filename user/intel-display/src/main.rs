//! intel-display: nativni nastaveni obrazu pro integrovanou (a castecne
//! diskretni) grafiku Intel Gen9 Skylake az Xe2 - bez akcelerace.
//!
//! Logika je v `libs/intelgfx` (testy nad simulovanymi registry); tady je
//! jen pristup k hardwaru: BAR0 (registry + GGTT), framebuffer firmwaru
//! (adresa z GGTT, stolen pamet nebo pamet karty), druhy buffer pro page
//! flip ve stolen pameti za framebufferem firmwaru.
//!
//! **Bezpecnost obrazu**: ovladac na registry obrazu nesaha, dokud
//! nema ulozeny zaznam firmwaru (objekt pameti drzi init) a dokud
//! kompozitor nepripoji snimek. Nepodporovana generace, dlazdicovy
//! framebuffer nebo zadny zapnuty vystup = konec pred nahlasenim
//! (init pouzije `fbdisplay`). Nova instance po padu (`restore`) zapise
//! zaznam zpet a slouzi jen rezim firmwaru.
//!
//! **Neovereno na hardwaru** (QEMU Intel neemuluje): postup testu na
//! skutecnem stroji je v `docs/cs/displej.md`.

#![no_std]
#![no_main]

extern crate alloc;

use core::time::Duration;

use displaydrv::Server;
use displaydrv::runtime::{Start, run};
use displaydrv::snapshot::{Snapshot, driver};
use intelgfx::backend::{Buffers, Intel, probe, restore_regs};
use intelgfx::platform::{Detect, Family, detect};
use intelgfx::{Clock, Mmio, gtt, regs};
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Mapping, Vmo, println};
use pci::{Address, Ecam};

libos::entry!(main);

const NAME: &str = "intel-display";

/// Registry v namapovanem BAR0.
struct Bar0 {
    map: Mapping,
    len: u64,
}

impl Mmio for Bar0 {
    fn read32(&mut self, reg: u32) -> u32 {
        if reg as u64 + 4 > self.len {
            return 0;
        }
        // SAFETY: offset overen proti delce mapovani BAR0.
        unsafe { core::ptr::read_volatile((self.map.addr() + reg as u64) as *const u32) }
    }

    fn write32(&mut self, reg: u32, v: u32) {
        if reg as u64 + 4 > self.len {
            return;
        }
        // SAFETY: jako `read32`.
        unsafe { core::ptr::write_volatile((self.map.addr() + reg as u64) as *mut u32, v) }
    }
}

struct OsClock;

impl Clock for OsClock {
    fn now(&self) -> u64 {
        libos::time::now()
    }

    fn delay_us(&mut self, us: u32) {
        if us >= 1000 {
            libos::time::sleep(Duration::from_micros(us as u64));
        } else {
            // Kratke cekani (AUX, GMBUS): aktivne.
            let end = libos::time::now() + us as u64 * 1000;
            while libos::time::now() < end {
                core::hint::spin_loop();
            }
        }
    }
}

/// Vlastni buffer: GGTT adresa nad 3.5 GiB (firmware ji nepouziva; overi se).
const OWN_GGTT: u32 = 0xE000_0000;
/// Zarovnani linearniho povrchu (SKL+: 256 KiB).
const SURF_ALIGN: u64 = 256 << 10;

fn main(startup: libos::Startup) -> i64 {
    let Some(mut st) = Start::take(startup, NAME) else {
        return 1;
    };
    let c = st.config;
    let a = Address {
        bus: c.bus,
        device: c.device,
        function: c.function,
    };
    let Ok(ecam) = Ecam::single(&st.config_vmo, a) else {
        println!("{NAME} {a}: konfigurace PCI nedostupna");
        return 2;
    };
    let (vendor, device) = (ecam.read16(a, 0), ecam.read16(a, 2));
    let platform = match detect(vendor, device) {
        Detect::Supported(p) => p,
        Detect::Unsupported(name) => {
            println!("{NAME} {a}: {name} zatim nepodporovano, obraz zustava na framebufferu firmwaru");
            return 4;
        }
        Detect::Unknown => {
            println!("{NAME} {a}: nezname zarizeni {vendor:04x}:{device:04x} (podporujeme Gen9 az Xe2)");
            return 4;
        }
    };
    ecam.enable(a);
    let mut mmio = match map_bar(&st, &ecam, a, 0) {
        Ok((map, len)) => Bar0 { map, len },
        Err(e) => {
            println!("{NAME} {a}: BAR0 nelze namapovat: {e}");
            return 2;
        }
    };
    let mut clock = OsClock;
    if c.restore {
        match Snapshot::read(st.snapshot_bytes(), driver::INTEL) {
            Some(s) => {
                restore_regs(&mut mmio, &s);
                println!("{NAME} {a}: stav firmwaru obnoven ({}x{}), dal jen rezim firmwaru", s.w, s.h);
            }
            None => println!("{NAME} {a}: zaznam firmwaru chybi, registry beze zmeny"),
        }
    }
    let head = match probe(&mut mmio, &mut clock, &platform) {
        Ok(h) => h,
        Err(e) => {
            println!("{NAME} {a}: {}: vystup od firmwaru nelze prevzit: {e}", platform.name);
            return 5;
        }
    };
    let plane = head.active.plane;
    let fb_len = plane.stride_bytes() as u64 * head.native.1 as u64;
    // Framebuffer firmwaru: fyzicka adresa z GGTT.
    let Some((fb_phys, local)) = gtt::contiguous(&mut mmio, plane.ggtt(), fb_len) else {
        println!("{NAME} {a}: framebuffer firmwaru neni souvisly, nelze prevzit");
        return 5;
    };
    let fb_cpu = if local {
        // Diskretni karta: pamet karty pres BAR2 (LMEMBAR).
        map_bar(&st, &ecam, a, 2).ok().filter(|(_, len)| fb_phys + fb_len <= *len).map(|(m, _)| (m, fb_phys))
    } else {
        st.resource.device_memory_cached(fb_phys, fb_len.next_multiple_of(4096), libos::abi::vm::cache::WRITE_COMBINING).ok().and_then(|v| Mapping::new(&v, 0, fb_len.next_multiple_of(4096), MAP_READ | MAP_WRITE).ok().map(|m| (m, 0)))
    };
    let Some((fb_map, fb_off)) = fb_cpu else {
        println!("{NAME} {a}: framebuffer firmwaru nelze namapovat");
        return 5;
    };
    let mut buffers = Buffers {
        ggtt: [plane.ggtt(), 0],
        cpu: [(fb_map.addr() + fb_off) as *mut u8, core::ptr::null_mut()],
        count: 1,
    };
    // Druhy buffer: stolen pamet za framebufferem firmwaru (jen
    // integrovana grafika do Gen13; MTL+ ma stolen pamet jinde).
    let mut _own: Option<(Vmo, Mapping)> = None;
    if !c.restore && !local && platform.family < Family::Gen14 {
        let bdsm = (ecam.read32(a, regs::PCI_BDSM) & 0xFFF0_0000) as u64;
        let ggc = ecam.read16(a, regs::PCI_GGC);
        let gms = (ggc >> 8) as u64;
        let dsm_size = if gms < 0xF0 { gms * (32 << 20) } else { (gms - 0xF0 + 1) * (4 << 20) };
        let own_phys = (fb_phys + fb_len).next_multiple_of(SURF_ALIGN);
        let fits = bdsm != 0 && fb_phys >= bdsm && own_phys + fb_len <= bdsm + dsm_size;
        if fits && gtt::is_free(&mut mmio, OWN_GGTT, fb_len) {
            let len = fb_len.next_multiple_of(4096);
            if let Ok(v) = st.resource.device_memory_cached(own_phys, len, libos::abi::vm::cache::WRITE_COMBINING)
                && let Ok(m) = Mapping::new(&v, 0, len, MAP_READ | MAP_WRITE)
            {
                gtt::map(&mut mmio, OWN_GGTT, own_phys, len, false);
                buffers.ggtt[1] = OWN_GGTT;
                buffers.cpu[1] = m.addr() as *mut u8;
                buffers.count = 2;
                _own = Some((v, m));
            }
        }
    }
    let mut intel = Intel::new(mmio, clock, platform, head, buffers, c.restore);
    if !c.restore {
        let s = intel.snapshot();
        if !s.write(st.snapshot_bytes()) {
            println!("{NAME} {a}: zaznam firmwaru se nevejde");
            return 3;
        }
    }
    intel.mark_saved();
    let h = &intel.head;
    println!(
        "{NAME} {a}: {} ({}), vystup {} pipe {}, {}x{} @ {} mHz, monitor '{}', {} rezimu, {} buffery",
        platform.name,
        if platform.discrete { "diskretni" } else { "integrovana" },
        guiproto::modes::kind::name(h.kind),
        h.active.pipe,
        h.native.0,
        h.native.1,
        h.refresh_mhz,
        h.monitor,
        1 + h.scaled.len(),
        intel.buffers.count
    );
    let mut server = Server::new(intel);
    if c.restore {
        server.announce_reset();
    }
    run(&mut server, &st.client);
    // Cisty konec: obraz firmwaru zpet (framebuffer firmwaru zustava platny).
    if let Some(s) = Snapshot::read(st.snapshot_bytes(), driver::INTEL) {
        server.backend.restore(&s);
    }
    drop(fb_map);
    0
}

/// Namapuje pametovy BAR `i` cely (BAR0 registry bez cache, BAR2 pamet
/// karty write-combining).
fn map_bar(st: &Start, ecam: &Ecam, a: Address, i: u8) -> libos::Result<(Mapping, u64)> {
    let b = ecam.memory_bar(a, i).ok_or(libos::Error::NotSupported)?;
    let size = ecam.memory_bar_size(a, i).ok_or(libos::Error::NotSupported)?;
    let cache = if i == 0 { libos::abi::vm::cache::UNCACHED } else { libos::abi::vm::cache::WRITE_COMBINING };
    let vmo = st.resource.device_memory_cached(b.phys, size, cache)?;
    let map = Mapping::new(&vmo, 0, size, MAP_READ | MAP_WRITE)?;
    Ok((map, size))
}
