//! Protokol schranky (milnik 6c).
//!
//! Schranku drzi modul kompozitoru (`libs/comp/src/clipboard.rs`); zpravy
//! jdou zatim v obalce okenniho protokolu (`guiproto` `CLIPBOARD`), aby
//! sluzba sla pozdeji prestehovat do vlastniho procesu bez zmeny zprav.
//!
//! Pravidla:
//! - **Jen na akci uzivatele:** nabidnout (kopirovat) a vyzadat (vlozit)
//!   smi jen okno s fokusem, ktere prave dostalo fyzicky vstup (klavesa
//!   nebo klik od skutecneho zarizeni, ne umely). Jedno gesto = jedno
//!   vlozeni. Aplikace na pozadi schranku neprecte.
//! - **Gesto = seriove cislo udalosti** (jako serial ve Waylandu):
//!   kompozitor prideli kazdemu fyzickemu stisku klavesy a tlacitka, ktery
//!   doruci oknu, cislo (`guiproto::window::InputRecord::serial`); aplikace
//!   ho posle v `Offer` / `Request`. Plati jen cislo, ktere kompozitor sam
//!   dorucil tomuto oknu, jen jednou, jen mezi poslednimi
//!   `comp::clipboard::GESTURES` stisky okna, kdyz ma okno fokus,
//!   a nejvys `comp::clipboard::GESTURE_MAX` (10 s). Pomale zpracovani
//!   (emulace) tak nevadi, podvrzene, cizi ani znovu pouzite cislo
//!   neprojde.
//! - **Liny prenos:** vlastnik nabidne formaty (`Offer`), data posle az na
//!   zadost (`Send`) pri vlozeni.
//! - **Po skonceni vlastnika** zustane posledni obsah: male formaty si
//!   kompozitor vyzada hned po nabidce a drzi kopii.
//! - **Zabezpecena pole** (role `password-input`) do schranky nikdy
//!   nekopiruji; kompozitor nabidku z okna s fokusem v takovem poli
//!   odmitne.
//! - Historie schranky zatim neni.
//!
//! Data do [`MAX_INLINE`] jdou primo ve zprave; vetsi pres sdilenou pamet
//! (dalsi krok, zprava `DataShm` je rezervovana).

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

pub use abi::{Error, Result};
use wire::{Reader, Writer};

/// Text v UTF-8 (jediny format, ktery knihovna UI zatim nabizi).
pub const TEXT: &str = "text/plain;charset=utf-8";

/// Nejvic bajtu dat primo ve zprave (zbytek kanalu na hlavicku obalky).
pub const MAX_INLINE: usize = 60 * 1024;

/// Nejvic formatu jedne nabidky a nejdelsi jmeno formatu.
pub const MAX_FORMATS: usize = 16;
pub const MAX_FORMAT_LEN: usize = 128;

mod op {
    pub const OFFER: u32 = 1;
    pub const REQUEST: u32 = 2;
    pub const DATA: u32 = 3;
    pub const SEND: u32 = 101;
    pub const DELIVER: u32 = 102;
    pub const DENIED: u32 = 103;
    pub const CHANGED: u32 = 104;
}

/// Proc kompozitor pozadavek odmitl.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denied {
    /// Okno nema fokus nebo pozadavek neprisel hned po fyzickem vstupu.
    NoGesture,
    /// Schranka je prazdna nebo nema pozadovany format.
    Empty,
    /// Fokus je v zabezpecenem poli.
    Secure,
    /// Vlastnik data neposlal (skoncil, vyprsel cas, prilis velka).
    Unavailable,
}

impl Denied {
    fn code(self) -> u8 {
        match self {
            Denied::NoGesture => 1,
            Denied::Empty => 2,
            Denied::Secure => 3,
            Denied::Unavailable => 4,
        }
    }

    fn from_code(c: u8) -> Result<Self> {
        Ok(match c {
            1 => Denied::NoGesture,
            2 => Denied::Empty,
            3 => Denied::Secure,
            4 => Denied::Unavailable,
            _ => return Err(Error::InvalidArgs),
        })
    }
}

/// Zprava aplikace -> schranka.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMsg {
    /// Kopirovat: aplikace ma data v techto formatech. `gesture` =
    /// seriove cislo vstupni udalosti, ktera kopirovani vyvolala
    /// (`InputRecord::serial`).
    Offer { gesture: u32, formats: Vec<String> },
    /// Vlozit: aplikace chce data ve formatu `format`; `gesture` jako
    /// u `Offer`.
    Request { gesture: u32, serial: u32, format: String },
    /// Odpoved vlastnika na `Send`.
    Data { serial: u32, data: Vec<u8> },
}

/// Zprava schranka -> aplikace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMsg {
    /// Vlastnikovi: posli data formatu `format` (odpoved `Data`).
    Send { serial: u32, format: String },
    /// Zadateli: data k `Request`.
    Deliver { serial: u32, format: String, data: Vec<u8> },
    Denied { serial: u32, why: Denied },
    /// Oknu s fokusem: co schranka obsahuje (polozka menu Vlozit).
    Changed { formats: Vec<String> },
}

fn formats_w(w: &mut Writer, f: &[String]) {
    w.u16(f.len() as u16);
    for s in f {
        w.str(s);
    }
}

fn formats_r(r: &mut Reader<'_>) -> Result<Vec<String>> {
    let n = r.u16()? as usize;
    if n > MAX_FORMATS {
        return Err(Error::InvalidArgs);
    }
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        let s = r.str()?;
        if s.is_empty() || s.len() > MAX_FORMAT_LEN {
            return Err(Error::InvalidArgs);
        }
        v.push(s);
    }
    Ok(v)
}

fn data_r(r: &mut Reader<'_>) -> Result<Vec<u8>> {
    let d = r.rest();
    if d.len() > MAX_INLINE {
        return Err(Error::InvalidArgs);
    }
    Ok(d.to_vec())
}

impl ClientMsg {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            ClientMsg::Offer { gesture, formats } => {
                w = Writer::op(op::OFFER);
                w.u32(*gesture);
                formats_w(&mut w, formats);
            }
            ClientMsg::Request { gesture, serial, format } => {
                w = Writer::op(op::REQUEST);
                w.u32(*gesture).u32(*serial).str(format);
            }
            ClientMsg::Data { serial, data } => {
                w = Writer::op(op::DATA);
                w.u32(*serial).bytes(data);
            }
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let m = match r.u32()? {
            op::OFFER => ClientMsg::Offer { gesture: r.u32()?, formats: formats_r(&mut r)? },
            op::REQUEST => ClientMsg::Request { gesture: r.u32()?, serial: r.u32()?, format: r.str()? },
            op::DATA => ClientMsg::Data { serial: r.u32()?, data: data_r(&mut r)? },
            _ => return Err(Error::NotSupported),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(m)
    }
}

impl ServerMsg {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            ServerMsg::Send { serial, format } => {
                w = Writer::op(op::SEND);
                w.u32(*serial).str(format);
            }
            ServerMsg::Deliver { serial, format, data } => {
                w = Writer::op(op::DELIVER);
                w.u32(*serial).str(format).bytes(data);
            }
            ServerMsg::Denied { serial, why } => {
                w = Writer::op(op::DENIED);
                w.u32(*serial).u8(why.code());
            }
            ServerMsg::Changed { formats } => {
                w = Writer::op(op::CHANGED);
                formats_w(&mut w, formats);
            }
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let m = match r.u32()? {
            op::SEND => ServerMsg::Send { serial: r.u32()?, format: r.str()? },
            op::DELIVER => ServerMsg::Deliver { serial: r.u32()?, format: r.str()?, data: data_r(&mut r)? },
            op::DENIED => ServerMsg::Denied { serial: r.u32()?, why: Denied::from_code(r.u8()?)? },
            op::CHANGED => ServerMsg::Changed { formats: formats_r(&mut r)? },
            _ => return Err(Error::NotSupported),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use alloc::vec;

    #[test]
    fn roundtrip() {
        let c = [
            ClientMsg::Offer { gesture: 3, formats: vec![TEXT.into()] },
            ClientMsg::Request { gesture: 4, serial: 7, format: TEXT.into() },
            ClientMsg::Data { serial: 7, data: "Čaj".as_bytes().to_vec() },
        ];
        for m in c {
            assert_eq!(ClientMsg::decode(&m.encode()).unwrap(), m);
        }
        let s = [
            ServerMsg::Send { serial: 1, format: TEXT.into() },
            ServerMsg::Deliver { serial: 2, format: TEXT.into(), data: vec![1, 2, 3] },
            ServerMsg::Denied { serial: 3, why: Denied::Secure },
            ServerMsg::Changed { formats: vec![] },
        ];
        for m in s {
            assert_eq!(ServerMsg::decode(&m.encode()).unwrap(), m);
        }
    }

    /// Neduveryhodna data: prilis mnoho formatu, velka data, smeti.
    #[test]
    fn rejects_bad_input() {
        let many = ClientMsg::Offer { gesture: 1, formats: (0..MAX_FORMATS + 1).map(|i| alloc::format!("f{i}")).collect() };
        assert!(ClientMsg::decode(&many.encode()).is_err());
        let big = ClientMsg::Data { serial: 1, data: vec![0; MAX_INLINE + 1] };
        assert!(ClientMsg::decode(&big.encode()).is_err());
        assert!(ClientMsg::decode(&[9, 0, 0, 0]).is_err());
        assert!(ClientMsg::decode(&[]).is_err());
        let empty = ClientMsg::Offer { gesture: 1, formats: vec![String::new()] };
        assert!(ClientMsg::decode(&empty.encode()).is_err());
    }
}
