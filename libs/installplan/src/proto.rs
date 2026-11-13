//! Protokol pruvodce (textovy v shellu, pozdeji GUI) <-> `installd`
//! (verze 1). Zprava = kod operace `u32` + pole (`libs/wire`). Na kazdy
//! pozadavek jedna odpoved; `Install` posila pred konecnou odpovedi
//! zpravy [`Reply::Progress`].

use alloc::string::String;
use alloc::vec::Vec;

pub use abi::{Error, Result};
use wire::{Reader, Writer};

use crate::map::{Seg, SegKind};
use crate::{Choice, Line, Mode};

/// Disk v seznamu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskEntry {
    pub index: u32,
    pub model: String,
    pub bytes: u64,
    /// Disk zive flashky (instalovat na nej nejde).
    pub live: bool,
    /// Oddily na disku (popis pro vypis).
    pub parts: Vec<String>,
    pub choices: Vec<Choice>,
}

/// Rozlozeni klavesnice (z `keymap`, stejny zdroj jako inputd).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardEntry {
    pub name: String,
    pub description: String,
    /// Nahled: radky klaves (pro GUI a vypis).
    pub rows: Vec<String>,
}

/// Udaje uctu pro instalaci.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountInput {
    pub name: String,
    pub display: String,
    pub password: String,
    /// PIN (prazdny = bez PIN).
    pub pin: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Disks,
    Keyboards,
    /// Nahled zmen disku pro zvoleny zpusob.
    Preview { disk: u32, mode: Mode },
    /// Instalace. `confirm` = opsany nazev disku (u celeho disku).
    Install { disk: u32, mode: Mode, confirm: String, settings: String, account: AccountInput },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Failed(Error, String),
    Disks(Vec<DiskEntry>),
    Keyboards(Vec<KeyboardEntry>),
    /// Nahled: text, potrebne potvrzeni (`Some(model)` = opsat nazev),
    /// na disku zustane jiny system; pro GUI radky zmen a mapa disku
    /// teď a po instalaci (graf oddilu).
    Preview { text: String, type_name: Option<String>, other_os: bool, lines: Vec<Line>, before: Vec<Seg>, after: Vec<Seg> },
    /// Krok `step` (index do `PROGRESS`) a popis.
    Progress { step: u32, text: String },
    /// Hotovo: souhrn (co delat dal).
    Done(String),
}

mod op {
    pub const DISKS: u32 = 1;
    pub const KEYBOARDS: u32 = 2;
    pub const PREVIEW: u32 = 3;
    pub const INSTALL: u32 = 4;
    pub const R_FAILED: u32 = 100;
    pub const R_DISKS: u32 = 101;
    pub const R_KEYBOARDS: u32 = 102;
    pub const R_PREVIEW: u32 = 103;
    pub const R_PROGRESS: u32 = 104;
    pub const R_DONE: u32 = 105;
}

/// Nejvic polozek v seznamech (ochrana pameti).
const MAX: usize = 256;

fn mode_from(r: &mut Reader) -> Result<Mode> {
    Mode::from_key(&r.str()?).ok_or(Error::InvalidArgs)
}

fn put_segs(w: &mut Writer, segs: &[Seg]) {
    w.u32(segs.len() as u32);
    for s in segs {
        w.u8(s.kind.code()).u64(s.bytes).str(&s.label).u8(s.new as u8);
    }
}

fn get_segs(r: &mut Reader) -> Result<Vec<Seg>> {
    let n = count(r)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let kind = SegKind::from_code(r.u8()?).ok_or(Error::InvalidArgs)?;
        out.push(Seg { kind, bytes: r.u64()?, label: r.str()?, new: r.u8()? == 1 });
    }
    Ok(out)
}

fn count(r: &mut Reader) -> Result<usize> {
    let n = r.u32()? as usize;
    if n > MAX { Err(Error::InvalidArgs) } else { Ok(n) }
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            Request::Disks => w = Writer::op(op::DISKS),
            Request::Keyboards => w = Writer::op(op::KEYBOARDS),
            Request::Preview { disk, mode } => {
                w = Writer::op(op::PREVIEW);
                w.u32(*disk).str(mode.key());
            }
            Request::Install { disk, mode, confirm, settings, account } => {
                w = Writer::op(op::INSTALL);
                w.u32(*disk).str(mode.key()).str(confirm).str(settings);
                w.str(&account.name).str(&account.display).str(&account.password).str(&account.pin);
            }
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let req = match r.u32()? {
            op::DISKS => Request::Disks,
            op::KEYBOARDS => Request::Keyboards,
            op::PREVIEW => Request::Preview { disk: r.u32()?, mode: mode_from(&mut r)? },
            op::INSTALL => Request::Install {
                disk: r.u32()?,
                mode: mode_from(&mut r)?,
                confirm: r.str()?,
                settings: r.str()?,
                account: AccountInput { name: r.str()?, display: r.str()?, password: r.str()?, pin: r.str()? },
            },
            _ => return Err(Error::InvalidArgs),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(req)
    }
}

impl Reply {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            Reply::Failed(e, text) => {
                w = Writer::op(op::R_FAILED);
                w.i32(e.code() as i32).str(text);
            }
            Reply::Disks(list) => {
                w = Writer::op(op::R_DISKS);
                w.u32(list.len() as u32);
                for d in list {
                    w.u32(d.index).str(&d.model).u64(d.bytes).u8(d.live as u8);
                    w.u32(d.parts.len() as u32);
                    for p in &d.parts {
                        w.str(p);
                    }
                    w.u32(d.choices.len() as u32);
                    for c in &d.choices {
                        w.str(c.mode.key()).u8(c.available as u8).str(&c.title).str(&c.detail);
                    }
                }
            }
            Reply::Keyboards(list) => {
                w = Writer::op(op::R_KEYBOARDS);
                w.u32(list.len() as u32);
                for k in list {
                    w.str(&k.name).str(&k.description).u32(k.rows.len() as u32);
                    for row in &k.rows {
                        w.str(row);
                    }
                }
            }
            Reply::Preview { text, type_name, other_os, lines, before, after } => {
                w = Writer::op(op::R_PREVIEW);
                w.str(text).str(type_name.as_deref().unwrap_or("")).u8(type_name.is_some() as u8).u8(*other_os as u8);
                w.u32(lines.len() as u32);
                for l in lines {
                    let k = match l {
                        Line::Keep(_) => 0,
                        Line::New(_) => 1,
                        Line::Erase(_) => 2,
                    };
                    w.u8(k).str(l.body());
                }
                put_segs(&mut w, before);
                put_segs(&mut w, after);
            }
            Reply::Progress { step, text } => {
                w = Writer::op(op::R_PROGRESS);
                w.u32(*step).str(text);
            }
            Reply::Done(text) => {
                w = Writer::op(op::R_DONE);
                w.str(text);
            }
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let flag = |r: &mut Reader| -> Result<bool> {
            match r.u8()? {
                0 => Ok(false),
                1 => Ok(true),
                _ => Err(Error::InvalidArgs),
            }
        };
        let reply = match r.u32()? {
            op::R_FAILED => {
                let e = Error::from_code(r.i32()? as i64).ok_or(Error::InvalidArgs)?;
                Reply::Failed(e, r.str()?)
            }
            op::R_DISKS => {
                let n = count(&mut r)?;
                let mut list = Vec::with_capacity(n);
                for _ in 0..n {
                    let (index, model, bytes, live) = (r.u32()?, r.str()?, r.u64()?, flag(&mut r)?);
                    let np = count(&mut r)?;
                    let parts = (0..np).map(|_| r.str()).collect::<Result<Vec<_>>>()?;
                    let nc = count(&mut r)?;
                    let mut choices = Vec::with_capacity(nc);
                    for _ in 0..nc {
                        choices.push(Choice { mode: mode_from(&mut r)?, available: flag(&mut r)?, title: r.str()?, detail: r.str()? });
                    }
                    list.push(DiskEntry { index, model, bytes, live, parts, choices });
                }
                Reply::Disks(list)
            }
            op::R_KEYBOARDS => {
                let n = count(&mut r)?;
                let mut list = Vec::with_capacity(n);
                for _ in 0..n {
                    let (name, description) = (r.str()?, r.str()?);
                    let nr = count(&mut r)?;
                    let rows = (0..nr).map(|_| r.str()).collect::<Result<Vec<_>>>()?;
                    list.push(KeyboardEntry { name, description, rows });
                }
                Reply::Keyboards(list)
            }
            op::R_PREVIEW => {
                let text = r.str()?;
                let name = r.str()?;
                let has = flag(&mut r)?;
                let other_os = flag(&mut r)?;
                let n = count(&mut r)?;
                let mut lines = Vec::with_capacity(n);
                for _ in 0..n {
                    let k = r.u8()?;
                    let t = r.str()?;
                    lines.push(match k {
                        0 => Line::Keep(t),
                        1 => Line::New(t),
                        2 => Line::Erase(t),
                        _ => return Err(Error::InvalidArgs),
                    });
                }
                let before = get_segs(&mut r)?;
                let after = get_segs(&mut r)?;
                Reply::Preview { text, type_name: has.then_some(name), other_os, lines, before, after }
            }
            op::R_PROGRESS => Reply::Progress { step: r.u32()?, text: r.str()? },
            op::R_DONE => Reply::Done(r.str()?),
            _ => return Err(Error::InvalidArgs),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(reply)
    }
}
