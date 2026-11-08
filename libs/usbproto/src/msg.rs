//! Zpravy tridni ovladac <-> `xhci` (verze 1).
//!
//! Pozadavek: hlavicka 16 B (`op` u32, `id` u32, `a` u32, `b` u32), pak
//! data. Odpoved: hlavicka 16 B (`id` u32, `status` i32 = 0 nebo kod chyby
//! `abi::Error`, `completion` u32 = kod dokonceni xHCI, `actual` u32 =
//! prenesene bajty), pak data (IN). Vse little-endian.
//!
//! Na kazdy pozadavek prijde prave jedna odpoved se stejnym `id`; poradi
//! odpovedi se muze lisit od poradi pozadavku (`INTERRUPT_IN` ceka, dokud
//! zarizeni nema data, kontrolni prenos mezitim projde). Na jeden endpoint
//! smi cekat nejvys jeden pozadavek (jinak `ALREADY_EXISTS`).

use alloc::vec::Vec;

use crate::desc::{DeviceDescriptor, Setup};

/// Kody operaci (`op`).
pub mod op {
    /// Informace o zarizeni a rozhrani: odpoved = [`Info`].
    pub const INFO: u32 = 1;
    /// Kontrolni prenos na endpointu 0: data = setup 8 B a u OUT data;
    /// odpoved u IN = prectena data.
    pub const CONTROL: u32 = 2;
    /// Interrupt IN: `a` = adresa endpointu (napr. 0x81), `b` = nejvic bajtu.
    /// Odpoved az prijdou data.
    pub const INTERRUPT_IN: u32 = 3;
    /// Interrupt OUT: `a` = adresa endpointu, data = obsah.
    pub const INTERRUPT_OUT: u32 = 4;
    /// Bulk IN: `a` = adresa endpointu (napr. 0x81), `b` = nejvic bajtu
    /// (nejvys [`MAX_DATA`](super::MAX_DATA)). Odpoved = prectena data
    /// (kratsi pri kratkem paketu).
    pub const BULK_IN: u32 = 5;
    /// Bulk OUT: `a` = adresa endpointu, data = obsah.
    pub const BULK_OUT: u32 = 6;
    /// Jen kanal hubu: na port `a` se pripojilo zarizeni rychlosti `b`
    /// ([`crate::Speed::to_u8`]); port je po resetu povoleny. `xhci`
    /// zarizeni nastavi a spusti ovladace jeho rozhrani. Odpoved: `actual`
    /// = cislo slotu.
    pub const ATTACH: u32 = 16;
    /// Jen kanal hubu: zarizeni z portu `a` zmizelo.
    pub const DETACH: u32 = 17;
    /// Jen kanal hubu: prvni pruchod porty dokoncen (radic pak hlasi
    /// spravci zarizeni "usb pripraveno").
    pub const SCAN_DONE: u32 = 18;
}

/// Kody dokonceni xHCI, ktere tridni ovladace rozlisuji.
pub mod completion {
    pub const SUCCESS: u32 = 1;
    pub const DATA_BUFFER: u32 = 2;
    pub const BABBLE: u32 = 3;
    pub const TRANSACTION: u32 = 4;
    pub const TRB: u32 = 5;
    /// Zarizeni odmitlo pozadavek (STALL).
    pub const STALL: u32 = 6;
    pub const SHORT_PACKET: u32 = 13;
}

pub const HEADER: usize = 16;
/// Nejvic dat v jednom prenosu (pozadavek i odpoved se vejdou do zpravy).
pub const MAX_DATA: usize = 16 * 1024;

/// Pozadavek.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request<'a> {
    pub op: u32,
    pub id: u32,
    pub a: u32,
    pub b: u32,
    pub data: &'a [u8],
}

impl<'a> Request<'a> {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(HEADER + self.data.len());
        for x in [self.op, self.id, self.a, self.b] {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v.extend_from_slice(self.data);
        v
    }

    pub fn decode(b: &'a [u8]) -> Option<Self> {
        if b.len() < HEADER || b.len() - HEADER > MAX_DATA + Setup::SIZE {
            return None;
        }
        let w = |i: usize| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
        Some(Self {
            op: w(0),
            id: w(1),
            a: w(2),
            b: w(3),
            data: &b[HEADER..],
        })
    }

    /// Kontrolni prenos: setup a data OUT.
    pub fn control(id: u32, setup: Setup, out: &[u8]) -> Vec<u8> {
        let mut data = Vec::with_capacity(8 + out.len());
        data.extend_from_slice(&setup.to_bytes());
        data.extend_from_slice(out);
        Request {
            op: op::CONTROL,
            id,
            a: 0,
            b: 0,
            data: &data,
        }
        .encode()
    }
}

/// Odpoved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply<'a> {
    pub id: u32,
    /// 0 = uspech, jinak kod chyby `abi::Error` (`IO_ERROR` = chyba prenosu,
    /// podrobnosti v `completion`).
    pub status: i32,
    pub completion: u32,
    pub actual: u32,
    pub data: &'a [u8],
}

impl<'a> Reply<'a> {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(HEADER + self.data.len());
        v.extend_from_slice(&self.id.to_le_bytes());
        v.extend_from_slice(&self.status.to_le_bytes());
        v.extend_from_slice(&self.completion.to_le_bytes());
        v.extend_from_slice(&self.actual.to_le_bytes());
        v.extend_from_slice(self.data);
        v
    }

    pub fn decode(b: &'a [u8]) -> Option<Self> {
        if b.len() < HEADER {
            return None;
        }
        let w = |i: usize| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
        Some(Self {
            id: w(0),
            status: w(1) as i32,
            completion: w(2),
            actual: w(3),
            data: &b[HEADER..],
        })
    }
}

/// Odpoved na `INFO`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub speed: crate::Speed,
    /// Cislo rozhrani, kteremu kanal patri (u root hubu 0).
    pub interface: u8,
    /// Cesta porty od radice (root port, port hubu, ...), max. 6 urovni.
    pub path: Vec<u8>,
    pub device: DeviceDescriptor,
    /// Cela konfigurace (vsechny deskriptory), jak ji zarizeni poslalo.
    pub config: Vec<u8>,
}

impl Info {
    pub fn encode(&self, raw_device: &[u8; DeviceDescriptor::SIZE]) -> Vec<u8> {
        let mut v = Vec::new();
        v.push(self.speed.to_u8());
        v.push(self.interface);
        v.push(self.path.len() as u8);
        v.extend_from_slice(&self.path);
        v.extend_from_slice(raw_device);
        v.extend_from_slice(&self.config);
        v
    }

    pub fn decode(b: &[u8]) -> Option<Self> {
        let speed = crate::Speed::from_u8(*b.first()?)?;
        let interface = *b.get(1)?;
        let depth = *b.get(2)? as usize;
        if depth > 6 {
            return None;
        }
        let path = b.get(3..3 + depth)?.to_vec();
        let dev_off = 3 + depth;
        let device = DeviceDescriptor::parse(b.get(dev_off..dev_off + DeviceDescriptor::SIZE)?)?;
        Some(Self {
            speed,
            interface,
            path,
            device,
            config: b[dev_off + DeviceDescriptor::SIZE..].to_vec(),
        })
    }
}
