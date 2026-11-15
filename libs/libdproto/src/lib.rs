//! Protokol sluzby knihoven `libd` (`docs/cs/dynamicke-linkovani.md`, DL-4).
//!
//! Klient (zavadec `ld.so`, `libos::spawn`) posila po kanalu zadost, libd
//! odpovi na tomtez kanalu. Vse little endian. Vstup od klienta je pro libd
//! **neduveryhodny**: dekodovani hlida delky a meze, jmena knihoven prochazi
//! [`valid_name`] (jen soubor v `/system/lib`, zadne cesty).
//!
//! | Zadost | Data | Handly | Odpoved |
//! |---|---|---|---|
//! | [`Request::Open`] | jmeno knihovny | - | [`Library`] + VMO na segment |
//! | [`Request::OpenImage`] | jmeno (jen do logu), delka | VMO s obsahem souboru | [`Library`] + VMO na segment |
//! | [`Request::Connect`] | - | konec noveho kanalu | **zadna** (nova relace pro dite) |
//! | [`Request::Stats`] | - | - | [`Stats`] |
//!
//! Chyba = [`Reply::Error`] s kodem `abi::Error`.
//!
//! **Knihovny z disku** (DL-4c, `OpenImage`): libd disk nikdy necte a nema
//! relaci FS. Soubor precte klient (zavadec) relaci FS procesu - s pravy
//! aplikace - a posle obsah jako VMO; libd si ho zkopiruje k sobe (klient
//! ho pak uz nezmeni), spocita BLAKE3 a sablonu sdili podle obsahu stejne
//! jako u `/system/lib`. Jmeno z `OpenImage` se nikdy nepouzije pro hledani
//! (`Open` podle jmena najde jen initfs), takze podvrzeny soubor se jmenem
//! systemove knihovny jinym procesum nic nepodstrci. Sablon z disku je
//! nejvys [`MAX_DISK_TEMPLATES`] a [`MAX_DISK_BYTES`]; nad mez libd zahodi
//! nejdele nepouzite (procesy je mapuji dal).
//!
//! **Relace** tvori strom: korenove spojeni je bootstrap kanal libd (drzi ho
//! adapter sluzby v initu, `docs/cs/obnova-sluzeb.md`), dalsi vznikaji
//! `Connect` z existujici relace - **bez odpovedi**, aby rodic (spawn,
//! adapter) nikdy necekal. Listek k nove relaci si rodic zaridi u supervisoru
//! (`ADOPT`, take bez cekani). Nad mez ([`MAX_CHILD_CONNECTIONS`] zijicich
//! z jedne relace = na proces, krome korene; [`MAX_CONNECTIONS`] celkem)
//! libd novy konec zavre: dite pri prvnim pouziti dostane `PEER_CLOSED`
//! a pres listek se dozvi, ze jde o odmitnuti (instance bezi dal).

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

#[cfg(test)]
mod tests;

/// Nejdelsi jmeno knihovny (bajty).
pub const MAX_NAME: usize = 255;
/// Nejvic segmentu `PT_LOAD` v knihovne.
pub const MAX_SEGMENTS: usize = 16;
/// Nejvetsi soubor knihovny (vetsi libd odmitne).
pub const MAX_FILE: u64 = 256 << 20;
/// Nejvic zijicich relaci vytvorenych z jedne relace (`Connect`) - mez na
/// proces (proces ma jednu relaci; deti dostanou relace z ni). Korenove
/// spojeni (init, adapter) mez nema.
pub const MAX_CHILD_CONNECTIONS: u32 = 256;
/// Nejvic spojeni celkem (nad mez libd nove spojeni zavre; klient pak
/// dostane `PEER_CLOSED`).
pub const MAX_CONNECTIONS: u32 = 4096;
/// Nejvic sablon knihoven z disku (`OpenImage`) v pameti libd.
pub const MAX_DISK_TEMPLATES: usize = 512;
/// Nejvic bajtu sablon knihoven z disku (soucet delek VMO segmentu).
pub const MAX_DISK_BYTES: u64 = 512 << 20;

mod tag {
    pub const OPEN: u8 = 1;
    pub const CONNECT: u8 = 2;
    pub const STATS: u8 = 3;
    pub const OPEN_IMAGE: u8 = 4;
    pub const OK: u8 = 0;
    pub const ERROR: u8 = 0xFF;
}

/// Jmeno knihovny v `/system/lib` (= `SONAME`, napr. `libavcodec.so.61`
/// nebo `ld.so`): 1 az [`MAX_NAME`] bajtu z `A-Z a-z 0-9 . _ + -`, nezacina
/// teckou. Zadne lomitko, `..`, nula ani ridici znaky.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && !name.starts_with('.')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'))
}

/// Zadost klienta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Open { name: String },
    /// Knihovna z disku: obsah je v prilozenem VMO (prvnich `len` bajtu),
    /// `name` je jen pro hlasky (zakladni jmeno souboru, [`valid_name`]).
    OpenImage { name: String, len: u64 },
    Connect,
    Stats,
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        match self {
            Request::Open { name } => {
                b.push(tag::OPEN);
                b.extend_from_slice(&(name.len() as u16).to_le_bytes());
                b.extend_from_slice(name.as_bytes());
            }
            Request::OpenImage { name, len } => {
                b.push(tag::OPEN_IMAGE);
                b.extend_from_slice(&(name.len() as u16).to_le_bytes());
                b.extend_from_slice(name.as_bytes());
                b.extend_from_slice(&len.to_le_bytes());
            }
            Request::Connect => b.push(tag::CONNECT),
            Request::Stats => b.push(tag::STATS),
        }
        b
    }

    /// `None` = vadna zprava (neznamy druh, delka, jmeno neprojde
    /// [`valid_name`]).
    pub fn decode(b: &[u8]) -> Option<Self> {
        match *b.first()? {
            tag::OPEN => {
                let len = u16::from_le_bytes(b.get(1..3)?.try_into().ok()?) as usize;
                let raw = b.get(3..3 + len)?;
                if b.len() != 3 + len {
                    return None;
                }
                let name = core::str::from_utf8(raw).ok()?;
                valid_name(name).then(|| Request::Open { name: String::from(name) })
            }
            tag::OPEN_IMAGE => {
                let n = u16::from_le_bytes(b.get(1..3)?.try_into().ok()?) as usize;
                let raw = b.get(3..3 + n)?;
                let len = u64::from_le_bytes(b.get(3 + n..11 + n)?.try_into().ok()?);
                if b.len() != 11 + n || len == 0 || len > MAX_FILE {
                    return None;
                }
                let name = core::str::from_utf8(raw).ok()?;
                valid_name(name).then(|| Request::OpenImage { name: String::from(name), len })
            }
            tag::CONNECT if b.len() == 1 => Some(Request::Connect),
            tag::STATS if b.len() == 1 => Some(Request::Stats),
            _ => None,
        }
    }
}

/// Druh segmentu v odpovedi.
pub mod kind {
    /// Kod nebo konstanty: zapecetene VMO sablony, mapuje se primo (sdilene
    /// stranky). Handle `READ | MAP | EXECUTE | ...`, bez `WRITE`.
    pub const SHARED: u32 = 0;
    /// Data: VMO sablony jen pro cteni; zavadec si udela klon s kopii pri
    /// zapisu (`VMO_CLONE`) a mapuje ho RW. Obsahuje i nuly do `mem_size`.
    pub const DATA: u32 = 1;
}

/// Segment knihovny (`PT_LOAD`) v odpovedi na `Open`; VMO pokryva stranky
/// `vaddr & !(stranka-1) .. konec`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    /// Zacatek (zaokrouhleny dolu na stranku 64 KiB) relativne k bazi.
    pub start: u64,
    /// Delka VMO (nasobek stranky).
    pub len: u64,
    /// Prava `MAP_*`.
    pub prot: u32,
    pub kind: u32,
}

const SEG_SIZE: usize = 8 + 8 + 4 + 4;

/// Odpoved na `Open`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Library {
    /// Rozsah modulu od baze (konec posledniho segmentu, zaokrouhleno).
    pub span: u64,
    pub segments: Vec<Segment>,
}

/// Statistika libd (testy, prehled).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    /// Sablony v pameti (ruzne obsahy knihoven).
    pub templates: u64,
    /// Zadosti `Open` celkem a kolik z nich nasla hotova sablona.
    pub opens: u64,
    pub hits: u64,
    /// Ziva spojeni.
    pub connections: u64,
    /// Sablony knihoven z disku (`OpenImage`) a kolik jich libd zahodila
    /// nad mezi ([`MAX_DISK_TEMPLATES`], [`MAX_DISK_BYTES`]).
    pub disk_templates: u64,
    pub evicted: u64,
}

/// Odpoved libd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Library(Library),
    Ok,
    Stats(Stats),
    Error(i32),
}

impl Reply {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        match self {
            Reply::Library(l) => {
                b.push(tag::OK);
                b.push(1);
                b.extend_from_slice(&l.span.to_le_bytes());
                b.extend_from_slice(&(l.segments.len() as u32).to_le_bytes());
                for s in &l.segments {
                    b.extend_from_slice(&s.start.to_le_bytes());
                    b.extend_from_slice(&s.len.to_le_bytes());
                    b.extend_from_slice(&s.prot.to_le_bytes());
                    b.extend_from_slice(&s.kind.to_le_bytes());
                }
            }
            Reply::Ok => {
                b.push(tag::OK);
                b.push(0);
            }
            Reply::Stats(s) => {
                b.push(tag::OK);
                b.push(2);
                for v in [s.templates, s.opens, s.hits, s.connections, s.disk_templates, s.evicted] {
                    b.extend_from_slice(&v.to_le_bytes());
                }
            }
            Reply::Error(code) => {
                b.push(tag::ERROR);
                b.extend_from_slice(&code.to_le_bytes());
            }
        }
        b
    }

    /// `None` = vadna odpoved (klient ji bere jako chybu protokolu).
    pub fn decode(b: &[u8]) -> Option<Self> {
        let u64_at = |o: usize| -> Option<u64> { Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?)) };
        let u32_at = |o: usize| -> Option<u32> { Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?)) };
        match *b.first()? {
            tag::ERROR if b.len() == 5 => Some(Reply::Error(u32_at(1)? as i32)),
            tag::OK => match *b.get(1)? {
                0 if b.len() == 2 => Some(Reply::Ok),
                1 => {
                    let span = u64_at(2)?;
                    let n = u32_at(10)? as usize;
                    if n == 0 || n > MAX_SEGMENTS || b.len() != 14 + n * SEG_SIZE {
                        return None;
                    }
                    let mut segments = Vec::with_capacity(n);
                    for i in 0..n {
                        let o = 14 + i * SEG_SIZE;
                        let s = Segment { start: u64_at(o)?, len: u64_at(o + 8)?, prot: u32_at(o + 16)?, kind: u32_at(o + 20)? };
                        let end = s.start.checked_add(s.len)?;
                        if s.len == 0 || end > span || s.kind > kind::DATA {
                            return None;
                        }
                        segments.push(s);
                    }
                    Some(Reply::Library(Library { span, segments }))
                }
                2 if b.len() == 2 + 48 => Some(Reply::Stats(Stats {
                    templates: u64_at(2)?,
                    opens: u64_at(10)?,
                    hits: u64_at(18)?,
                    connections: u64_at(26)?,
                    disk_templates: u64_at(34)?,
                    evicted: u64_at(42)?,
                })),
                _ => None,
            },
            _ => None,
        }
    }
}
