//! Konfigurace ovladace od spravce zarizeni (prvni zprava na bootstrap
//! kanalu): `pci <sbernice> <zarizeni> <funkce>` desitkove, za tim volitelne
//! BARy, ktere ovladac dostal: `bar <index> <adresa> <velikost>` (hex
//! s `0x`).
//!
//! ```text
//! pci 0 4 0 bar 0 0xfea00000 0x100000
//! ```
//!
//! Prvni cast je stejna jako u starsich ovladacu (`virtio::read_driver_config`).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Pametovy BAR predany ovladaci.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bar {
    pub index: u8,
    pub phys: u64,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverConfig {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub bars: Vec<Bar>,
}

fn hex(s: &str) -> Option<u64> {
    u64::from_str_radix(s.strip_prefix("0x")?, 16).ok()
}

/// Rozebere zpravu; cokoli navic nebo mimo rozsah = `None`.
pub fn parse(text: &str) -> Option<DriverConfig> {
    let mut w = text.split_whitespace();
    if w.next()? != "pci" {
        return None;
    }
    let mut num = || w.next()?.parse::<u8>().ok();
    let (bus, device, function) = (num()?, num()?, num()?);
    if device > 31 || function > 7 {
        return None;
    }
    let mut bars = Vec::new();
    while let Some(word) = w.next() {
        if word != "bar" {
            return None;
        }
        let index = w.next()?.parse::<u8>().ok().filter(|&i| i < 6)?;
        let phys = hex(w.next()?)?;
        let size = hex(w.next()?).filter(|&s| s > 0)?;
        phys.checked_add(size)?;
        bars.push(Bar { index, phys, size });
    }
    Some(DriverConfig {
        bus,
        device,
        function,
        bars,
    })
}

/// Zprava pro ovladac (pouziva devmgr).
pub fn format(c: &DriverConfig) -> String {
    let mut s = format!("pci {} {} {}", c.bus, c.device, c.function);
    for b in &c.bars {
        s.push_str(&format!(" bar {} {:#x} {:#x}", b.index, b.phys, b.size));
    }
    s
}
