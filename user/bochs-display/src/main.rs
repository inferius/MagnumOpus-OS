//! bochs-display: vzorovy nativni ovladac displeje pro QEMU
//! (`-device bochs-display`, na x86 i `secondary-vga` a `VGA` - stejne
//! registry DISPI v MMIO BAR2). Umi zmenu rozliseni za behu (rezimy
//! z EDID, ktere QEMU generuje) a page flip (dva buffery ve VRAM).
//! Slouzi k testu protokolu displeje a kompozitoru v QEMU na x86_64
//! i aarch64; skutecny hardware je `intel-display`.
//!
//! Startovni zprava a konfigurace: `displaydrv::runtime`. Pred prvni
//! zmenou registru ulozi stav firmwaru (zaznam drzi init); nova instance
//! po padu (`restore`) ho vrati a dal slouzi jen rezim firmwaru
//! (`docs/cs/displej.md`, bezpecnost obrazu).

#![no_std]
#![no_main]

extern crate alloc;

mod dispi;

use displaydrv::Server;
use displaydrv::runtime::{Start, run};
use displaydrv::snapshot::{Snapshot, driver};
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Mapping, println};
use pci::{Address, Ecam};

use dispi::Bochs;

libos::entry!(main);

const NAME: &str = "bochs-display";

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
    let bochs = match open(&st, a) {
        Ok(b) => b,
        Err(e) => {
            println!("{NAME} {a}: zarizeni nelze otevrit: {e}");
            return 2;
        }
    };
    let mut bochs = bochs;
    if c.restore {
        // Nova instance po padu: vratit stav firmwaru, dal jen jeho rezim.
        match Snapshot::read(st.snapshot_bytes(), driver::BOCHS) {
            Some(s) => {
                bochs.restore(&s);
                println!("{NAME} {a}: stav firmwaru obnoven ({}x{}), dal jen rezim firmwaru", s.w, s.h);
            }
            None => println!("{NAME} {a}: zaznam firmwaru chybi, registry beze zmeny"),
        }
    } else {
        let s = bochs.snapshot();
        if !s.write(st.snapshot_bytes()) {
            println!("{NAME} {a}: zaznam firmwaru se nevejde");
            return 3;
        }
    }
    bochs.mark_saved();
    let edid = if bochs.edid_raw_ok() { "EDID ok" } else { "bez EDID" };
    match displaydrv::Backend::size(&bochs, 0) {
        Some((w, h)) => println!("{NAME} {a}: bezi, rezim {w}x{h}, {edid}"),
        None => println!("{NAME} {a}: bezi, vystup vypnuty, {edid}"),
    }
    let mut server = Server::new(bochs);
    if c.restore {
        server.announce_reset();
    }
    run(&mut server, &st.client);
    // Cisty konec (vypnuti): vratit obraz firmwaru.
    if let Some(s) = Snapshot::read(st.snapshot_bytes(), driver::BOCHS) {
        server.backend.restore(&s);
    }
    0
}

fn open(st: &Start, a: Address) -> libos::Result<Bochs> {
    let ecam = Ecam::single(&st.config_vmo, a)?;
    ecam.enable(a);
    let bar = |i: u8| -> libos::Result<(u64, u64)> {
        let b = ecam.memory_bar(a, i).ok_or(libos::Error::NotSupported)?;
        let s = ecam.memory_bar_size(a, i).ok_or(libos::Error::NotSupported)?;
        Ok((b.phys, s))
    };
    let (vram_phys, vram_size) = bar(0)?;
    let (mmio_phys, mmio_size) = bar(2)?;
    let page = libos::vm::page_size();
    let vram_vmo = st.resource.device_memory(vram_phys, vram_size)?;
    let vram = Mapping::new(&vram_vmo, 0, vram_size, MAP_READ | MAP_WRITE)?;
    let mmio_len = mmio_size.max(page).next_multiple_of(page);
    let regs_vmo = st.resource.device_memory(mmio_phys, mmio_len)?;
    let regs = Mapping::new(&regs_vmo, 0, mmio_len, MAP_READ | MAP_WRITE)?;
    Bochs::new(regs, vram, vram_size, st.config.restore).map_err(|_| libos::Error::NotSupported)
}
