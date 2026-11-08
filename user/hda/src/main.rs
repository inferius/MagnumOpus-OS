//! hda: ovladac radice Intel High Definition Audio (PCI trida 04:03).
//!
//! Spousti ho spravce zarizeni. Startovni zprava (kind USER, v tomto
//! poradi): omezeny Resource (BARy a konfigurace teto funkce, kvoty
//! preruseni a DMA), stranka konfiguracniho prostoru PCI, kanal ke
//! zvukove sluzbe (protokol `audioproto::device`). Prvni zprava na
//! bootstrap kanalu: `pci <sbernice> <zarizeni> <funkce>`.
//!
//! Postup: reset radice, CORB/RIRB, vycet vsech kodeku a widgetu, cesty
//! k vystupum, ke vstupu a k HDMI/DP (`hdaspec`), opravy desek, pak
//! obsluha pozadavku `audiod` a preruseni (`driver.rs`). Vsechny kodeky
//! s vystupem hraji tentyz stream (jedno cislo streamu na lince HDA).
//! Preruseni je MSI; bez nej se pozice cte casovacem. Konektory hlasi
//! nevyzadane odpovedi, zaloha je cteni kazdych par sekund.
//!
//! Kdyz zvukova sluzba spadne, kanal k ovladaci drzi supervisor (Kept):
//! ovladac bezi dal a nova instance sluzby si stream otevre znovu.

#![no_std]
#![no_main]

extern crate alloc;

mod codec;
mod codec_hdmi;
mod codec_in;
mod ctrl;
mod driver;
mod stream;

use libos::hw::Resource;
use libos::{Channel, Startup, Vmo, println};
use pci::{Address, Ecam};

libos::entry!(main);

/// PCI Express capability: Device Control, bit 11 = smi zarizeni pouzit
/// "no snoop" (DMA mimo koherenci cache).
const CAP_PCIE: u8 = 0x10;
const DEVCTL_NOSNOOP: u32 = 1 << 11;

fn main(startup: Startup) -> i64 {
    let mut handles = startup.handles.into_iter();
    let (Some(res), Some(cfg), Some(client)) = (handles.next(), handles.next(), handles.next()) else {
        println!("hda: chybi handly ve startovni zprave");
        return 1;
    };
    let res = Resource::from_handle(res);
    let cfg = Vmo::from_handle(cfg);
    let client = Channel::from_handle(client);
    let Some(address) = pci::read_driver_config(&startup.bootstrap) else {
        println!("hda: neplatna konfigurace od spravce zarizeni");
        return 2;
    };
    let mut drv = match driver::Driver::open(&cfg, address, res) {
        Ok(d) => d,
        Err(e) => {
            println!("hda {address}: inicializace selhala: {e}");
            return 3;
        }
    };
    drv.serve(&client);
    drv.shutdown();
    0
}

/// Upravy konfigurace PCI pred startem (fakta z Linuxu `snd_hda_intel`):
/// DMA vzdy koherentni s cache (nase pamet DMA je cachovana), u Intelu
/// trida provozu TC0 (jinak na nekterych cipsetech praskani).
pub fn quirks(ecam: &Ecam, a: Address, vendor: u16) {
    // PCIe Device Control: zakazat "no snoop".
    if let Some((cap, _)) = ecam.capabilities(a).into_iter().find(|&(_, id)| id == CAP_PCIE) {
        let reg = cap + 8;
        let v = ecam.read32(a, reg);
        if v & DEVCTL_NOSNOOP != 0 {
            ecam.write32(a, reg, v & !DEVCTL_NOSNOOP);
        }
    }
    match vendor {
        // Intel: TCSEL (0x44, bity 2:0) = TC0.
        0x8086 => {
            let v = ecam.read32(a, 0x44);
            ecam.write32(a, 0x44, v & !7);
        }
        // AMD/ATI: snoop v registru 0x42 (bity 2:0 = 0b010).
        0x1002 | 0x1022 => {
            let v = ecam.read32(a, 0x40);
            ecam.write32(a, 0x40, (v & !(7 << 16)) | 2 << 16);
        }
        _ => {}
    }
}
