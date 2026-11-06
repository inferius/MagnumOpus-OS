//! Pristup ke konfiguracnimu prostoru PCI Express pres ECAM (pro spravce
//! zarizeni a ovladace v userspace).
//!
//! ECAM mapuje konfiguraci kazde funkce na 4 KiB pameti:
//! `(sbernice - prvni) << 20 | zarizeni << 15 | funkce << 12 | registr`.

#![no_std]

extern crate alloc;

pub mod msix;

use alloc::vec::Vec;

use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Mapping, Result, Vmo};

pub const VENDOR_ID: u16 = 0x00;
pub const COMMAND: u16 = 0x04;
pub const STATUS: u16 = 0x06;
pub const CLASS_REV: u16 = 0x08;
pub const HEADER_TYPE: u16 = 0x0E;
pub const BAR0: u16 = 0x10;
pub const CAP_PTR: u16 = 0x34;

pub const COMMAND_MEMORY: u16 = 1 << 1;
pub const COMMAND_BUS_MASTER: u16 = 1 << 2;
pub const COMMAND_INTX_DISABLE: u16 = 1 << 10;
const STATUS_CAP_LIST: u16 = 1 << 4;

pub const CAP_MSIX: u8 = 0x11;
pub const CAP_VENDOR: u8 = 0x09;

/// Adresa funkce na sbernici.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Address {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

impl core::fmt::Display for Address {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:02x}:{:02x}.{}", self.bus, self.device, self.function)
    }
}

/// Namapovane okno ECAM pro rozsah sbernic.
pub struct Ecam {
    map: Mapping,
    bus_start: u8,
    bus_end: u8,
}

/// Zakladni informace o funkci.
#[derive(Debug, Clone, Copy)]
pub struct Function {
    pub address: Address,
    pub vendor: u16,
    pub device: u16,
    pub class: u8,
    pub subclass: u8,
    pub prog_if: u8,
}

/// Pametovy BAR (I/O BARy moderni zarizeni nepotrebuji).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryBar {
    pub phys: u64,
    pub is_64bit: bool,
}

impl Ecam {
    /// Namapuje okno ECAM; `arg` je hodnota ze startovni zpravy
    /// (prvni sbernice | posledni << 8 | segment << 16).
    pub fn new(vmo: &Vmo, arg: u32) -> Result<Self> {
        let bus_start = arg as u8;
        let bus_end = (arg >> 8) as u8;
        let len = (bus_end as u64 - bus_start as u64 + 1) << 20;
        let map = Mapping::new(vmo, 0, len, MAP_READ | MAP_WRITE)?;
        Ok(Self {
            map,
            bus_start,
            bus_end,
        })
    }

    fn offset(&self, a: Address, reg: u16) -> u64 {
        ((a.bus - self.bus_start) as u64) << 20
            | (a.device as u64) << 15
            | (a.function as u64) << 12
            | reg as u64
    }

    pub fn read32(&self, a: Address, reg: u16) -> u32 {
        self.map.read(self.offset(a, reg))
    }

    pub fn read16(&self, a: Address, reg: u16) -> u16 {
        self.map.read(self.offset(a, reg))
    }

    pub fn read8(&self, a: Address, reg: u16) -> u8 {
        self.map.read(self.offset(a, reg))
    }

    pub fn write32(&self, a: Address, reg: u16, v: u32) {
        self.map.write(self.offset(a, reg), v)
    }

    pub fn write16(&self, a: Address, reg: u16, v: u16) {
        self.map.write(self.offset(a, reg), v)
    }

    /// Projde vsechny sbernice a vrati existujici funkce.
    pub fn scan(&self) -> Vec<Function> {
        let mut out = Vec::new();
        for bus in self.bus_start..=self.bus_end {
            for device in 0..32 {
                for function in 0..8 {
                    let a = Address {
                        bus,
                        device,
                        function,
                    };
                    let vendor = self.read16(a, VENDOR_ID);
                    if vendor == 0xFFFF {
                        if function == 0 {
                            break; // zarizeni neexistuje
                        }
                        continue;
                    }
                    let class_rev = self.read32(a, CLASS_REV);
                    out.push(Function {
                        address: a,
                        vendor,
                        device: self.read16(a, VENDOR_ID + 2),
                        class: (class_rev >> 24) as u8,
                        subclass: (class_rev >> 16) as u8,
                        prog_if: (class_rev >> 8) as u8,
                    });
                    // Jednofunkcni zarizeni: dalsi funkce nehledat.
                    if function == 0 && self.read8(a, HEADER_TYPE) & 0x80 == 0 {
                        break;
                    }
                }
            }
        }
        out
    }

    /// Pametovy BAR `index` (u 64bitoveho BARu cte i dalsi registr).
    pub fn memory_bar(&self, a: Address, index: u8) -> Option<MemoryBar> {
        let reg = BAR0 + index as u16 * 4;
        let low = self.read32(a, reg);
        if low & 1 != 0 {
            return None; // I/O BAR
        }
        let is_64bit = (low >> 1) & 3 == 2;
        let mut phys = (low & !0xF) as u64;
        if is_64bit {
            phys |= (self.read32(a, reg + 4) as u64) << 32;
        }
        Some(MemoryBar { phys, is_64bit })
    }

    /// Seznam capability funkce: (offset, id).
    pub fn capabilities(&self, a: Address) -> Vec<(u16, u8)> {
        let mut out = Vec::new();
        if self.read16(a, STATUS) & STATUS_CAP_LIST == 0 {
            return out;
        }
        let mut ptr = (self.read8(a, CAP_PTR) & !3) as u16;
        // Omezeni poctu kroku: zacykleny seznam nesmi zaseknout ovladac.
        for _ in 0..64 {
            if ptr == 0 {
                break;
            }
            out.push((ptr, self.read8(a, ptr)));
            ptr = (self.read8(a, ptr + 1) & !3) as u16;
        }
        out
    }

    /// Zapne pristup do pameti zarizeni a DMA, vypne INTx (pouziva se MSI-X).
    pub fn enable(&self, a: Address) {
        let cmd = self.read16(a, COMMAND);
        self.write16(
            a,
            COMMAND,
            cmd | COMMAND_MEMORY | COMMAND_BUS_MASTER | COMMAND_INTX_DISABLE,
        );
    }
}
