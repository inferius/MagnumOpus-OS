//! Povoleni aplikaci (docs/cs/ucty.md, kapitola Povoleni aplikaci):
//! rozhodnuti uzivatele "smi aplikace X pouzit Y" (dnes mikrofon) ulozene
//! trvale po uzivateli a aplikaci ve sluzbe uctu.
//!
//! - **Kanal `permissions`** (sluzba zarizeni, dnes `audiod`; dostava ho od
//!   initu): [`Ask`] -> [`Answer`]. Ulozene rozhodnuti odpovi hned, jinak
//!   `accountd` ukaze systemovy dialog Povoleni (Povolit / Jen tentokrat /
//!   Nepovolit) a "Povolit" / "Nepovolit" ulozi.
//! - **Kanal self** (uzivatel sam, prikaz shellu `povoleni`): seznam vlastnich
//!   rozhodnuti a odvolani (`Request::PermList`, `Request::PermRevoke`).
//!
//! Ulozeny tvar (`/db/permissions/<uid>` v relaci FS sluzby uctu): radky
//! `identita<TAB>povoleni<TAB>allow|deny`. Aplikace ho nevidi ani nezmeni.

use alloc::string::String;
use alloc::vec::Vec;

use wire::{Reader, Writer};

use crate::{Error, Result};

/// Druh povoleni (hodnoty jsou soucasti protokolu a ulozeneho tvaru).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Perm {
    Microphone = 1,
}

impl Perm {
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            1 => Ok(Perm::Microphone),
            _ => Err(Error::InvalidArgs),
        }
    }

    /// Jmeno v ulozenem tvaru a v prikazu shellu.
    pub fn name(self) -> &'static str {
        match self {
            Perm::Microphone => "mikrofon",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "mikrofon" => Some(Perm::Microphone),
            _ => None,
        }
    }

    /// Text dialogu (cesky, system).
    pub fn question(self) -> &'static str {
        match self {
            Perm::Microphone => "chce nahrávat zvuk z mikrofonu.",
        }
    }
}

/// Odpoved uzivatele.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Decision {
    Deny = 0,
    Allow = 1,
    /// Povolit jen tentokrat (do konce behu aplikace), neukladat.
    Once = 2,
}

impl Decision {
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            0 => Ok(Decision::Deny),
            1 => Ok(Decision::Allow),
            2 => Ok(Decision::Once),
            _ => Err(Error::InvalidArgs),
        }
    }

    pub fn allows(self) -> bool {
        self != Decision::Deny
    }
}

/// Volby dialogu v poradi (index = volba v `DialogReply::Answer`).
/// "Nepovolit" je posledni: Esc v dialogu kompozitoru vybere posledni volbu.
pub const DIALOG_OPTIONS: [&str; 3] = ["Povolit", "Jen tentokrát", "Nepovolit"];

/// Volba dialogu -> rozhodnuti (Esc a zavreni dialogu = Nepovolit).
pub fn decision_of_choice(choice: u32) -> Decision {
    match choice {
        0 => Decision::Allow,
        1 => Decision::Once,
        _ => Decision::Deny,
    }
}

/// Nejdelsi identita aplikace.
pub const MAX_APP: usize = 128;

/// Dotaz sluzby zarizeni (kanal `permissions`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub id: u32,
    pub uid: u64,
    pub app_id: String,
    pub app_name: String,
    pub perm: Perm,
}

impl Ask {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(self.id).u64(self.uid).str(&self.app_id).str(&self.app_name).u8(self.perm as u8);
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let a = Self { id: r.u32()?, uid: r.u64()?, app_id: r.str()?, app_name: r.str()?, perm: Perm::from_u8(r.u8()?)? };
        if !r.is_empty() || !valid_app(&a.app_id) || a.app_name.len() > MAX_APP {
            return Err(Error::InvalidArgs);
        }
        Ok(a)
    }
}

/// Zprava na kanalu `permissions`: dotaz sluzby zarizeni, nebo sprava
/// rozhodnuti uzivatele (init jmenem relace shellu, prikaz `povoleni`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChanMsg {
    Ask(Ask),
    List { uid: u64 },
    Revoke { uid: u64, app_id: String, perm: Perm },
}

const M_ASK: u8 = 1;
const M_LIST: u8 = 2;
const M_REVOKE: u8 = 3;

impl ChanMsg {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            ChanMsg::Ask(a) => {
                w.u8(M_ASK).bytes(&a.encode());
            }
            ChanMsg::List { uid } => {
                w.u8(M_LIST).u64(*uid);
            }
            ChanMsg::Revoke { uid, app_id, perm } => {
                w.u8(M_REVOKE).u64(*uid).str(app_id).u8(*perm as u8);
            }
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let (&k, rest) = b.split_first().ok_or(Error::InvalidArgs)?;
        let mut r = Reader::new(rest);
        let m = match k {
            M_ASK => return Ask::decode(rest).map(ChanMsg::Ask),
            M_LIST => ChanMsg::List { uid: r.u64()? },
            M_REVOKE => ChanMsg::Revoke { uid: r.u64()?, app_id: r.str()?, perm: Perm::from_u8(r.u8()?)? },
            _ => return Err(Error::InvalidArgs),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(m)
    }
}

/// Odpoved na [`Ask`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Answer {
    pub id: u32,
    pub decision: Decision,
}

impl Answer {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(self.id).u8(self.decision as u8);
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let a = Self { id: r.u32()?, decision: Decision::from_u8(r.u8()?)? };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(a)
    }
}

/// Identita aplikace pro ulozeny tvar: neprazdna, bez tabulatoru a konce
/// radku, rozumna delka.
pub fn valid_app(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_APP && !id.contains(['\t', '\n', '\r'])
}

/// Ulozene rozhodnuti.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub app_id: String,
    pub perm: Perm,
    pub allow: bool,
}

/// Rozhodnuti uzivatele (soubor) -> seznam; vadne radky se preskoci.
pub fn parse(text: &str) -> Vec<Entry> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.trim_end_matches('\r').split('\t');
            let (app, perm, v) = (it.next()?, it.next()?, it.next()?);
            if it.next().is_some() || !valid_app(app) {
                return None;
            }
            let allow = match v {
                "allow" => true,
                "deny" => false,
                _ => return None,
            };
            Some(Entry { app_id: app.into(), perm: Perm::from_name(perm)?, allow })
        })
        .collect()
}

/// Seznam -> soubor (serazeny, jeden zapis na aplikaci a povoleni).
pub fn serialize(entries: &[Entry]) -> String {
    let mut v: Vec<&Entry> = entries.iter().collect();
    v.sort_by(|a, b| (a.app_id.as_str(), a.perm).cmp(&(b.app_id.as_str(), b.perm)));
    v.dedup_by(|a, b| a.app_id == b.app_id && a.perm == b.perm);
    let mut s = String::new();
    for e in v {
        s.push_str(&e.app_id);
        s.push('\t');
        s.push_str(e.perm.name());
        s.push('\t');
        s.push_str(if e.allow { "allow" } else { "deny" });
        s.push('\n');
    }
    s
}

/// Ulozi rozhodnuti (prepise predchozi pro stejnou aplikaci a povoleni).
pub fn set(entries: &mut Vec<Entry>, app_id: &str, perm: Perm, allow: bool) {
    entries.retain(|e| !(e.app_id == app_id && e.perm == perm));
    entries.push(Entry { app_id: app_id.into(), perm, allow });
}

/// Ulozene rozhodnuti, nebo `None` (zeptat se).
pub fn lookup(entries: &[Entry], app_id: &str, perm: Perm) -> Option<bool> {
    entries.iter().find(|e| e.app_id == app_id && e.perm == perm).map(|e| e.allow)
}

/// Seznam pro kanal self (`PermList`).
pub fn encode_entries(entries: &[Entry]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(entries.len() as u32);
    for e in entries {
        w.str(&e.app_id).u8(e.perm as u8).u8(e.allow as u8);
    }
    w.finish()
}

pub fn decode_entries(b: &[u8]) -> Result<Vec<Entry>> {
    let mut r = Reader::new(b);
    let n = r.u32()? as usize;
    if n > 4096 {
        return Err(Error::InvalidArgs);
    }
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        v.push(Entry { app_id: r.str()?, perm: Perm::from_u8(r.u8()?)?, allow: r.u8()? != 0 });
    }
    Ok(v)
}
