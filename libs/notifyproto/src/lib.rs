//! Protokol sluzby oznameni `notifyd` (verze 1; sluzba v `user/notifyd`,
//! logika v `libs/notifycore`, popis v `docs/cs/oznameni.md`).
//!
//! Dve strany, kazda na jinem kanalu (jina capability):
//!
//! - **odesilatel** (aplikace, sluzba): [`Send`] -> [`SendReply`].
//!   Identitu (aplikace z manifestu, uzivatel, systemova sluzba) kanalu
//!   urcil init pri jeho vydani; odesilatel ji nemuze zmenit ani
//!   podvrhnout. Oznameni identifikuje vlastni klic (`key`): stejny klic =
//!   nahrazeni (prubeh stahovani), `Close` ho zavre. Udalosti zpet: klik
//!   na akci, zavreni uzivatelem ([`SendReply::Event`]).
//! - **prohlizec** (kompozitor: vyskakovaci oznameni a centrum oznameni,
//!   shell): [`View`] -> [`ViewReply`]. Odber zacne snimkem a pokracuje
//!   zmenami; prohlizec vidi jen oznameni sveho uzivatele a systemova.
//!
//! Zprava = kod operace `u32` + pole (`libs/wire`, little-endian). Neplatna
//! zprava = `INVALID_ARGS`, nikdy panika. Texty delsi nez limity se zkrati
//! (sluzba, ne chyba), ridici znaky se odstrani.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

#[cfg(test)]
mod tests;

use alloc::string::String;
use alloc::vec::Vec;

pub use abi::{Error, Result};
use wire::{Reader, Writer};

pub const VERSION: u32 = 1;

/// Limity textu (znaky; delsi se zkrati) a akci.
pub const MAX_TITLE: usize = 120;
pub const MAX_BODY: usize = 1000;
pub const MAX_ACTIONS: usize = 3;
pub const MAX_ACTION_ID: usize = 32;
pub const MAX_ACTION_LABEL: usize = 40;
/// Nejvic oznameni v jedne zprave snimku (zbytek dalsimi zpravami).
pub const MAX_PER_MSG: usize = 32;

/// Dulezitost oznameni.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// Jen do centra oznameni, nevyskoci.
    Low,
    /// Vyskoci na 6 s, pak do centra.
    Normal,
    /// Vyskoci a zustane, dokud ho uzivatel nezavre nebo neklikne na akci.
    High,
    /// Projde i rezimem Nerusit (hovor, baterie, bezpecnost). Smi jen
    /// systemova identita; aplikaci se snizi na `High`.
    Urgent,
}

impl Priority {
    pub const fn to_u8(self) -> u8 {
        match self {
            Priority::Low => 0,
            Priority::Normal => 1,
            Priority::High => 2,
            Priority::Urgent => 3,
        }
    }

    pub fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            0 => Priority::Low,
            1 => Priority::Normal,
            2 => Priority::High,
            3 => Priority::Urgent,
            _ => return Err(Error::InvalidArgs),
        })
    }
}

/// Tlacitko oznameni: `id` vrati odesilateli, `label` vidi uzivatel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub id: String,
    pub label: String,
}

/// Proc oznameni zmizelo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Closed {
    /// Zavrel ho uzivatel (krizek, Vymazat vse).
    User,
    /// Zavrel ho odesilatel (`Close`).
    Sender,
    /// Vytlacilo ho novejsi (limit oznameni identity nebo historie).
    Limit,
    /// Odesilatel skoncil (kanal zavren) a oznameni bylo jen docasne.
    Gone,
}

impl Closed {
    const fn to_u8(self) -> u8 {
        match self {
            Closed::User => 0,
            Closed::Sender => 1,
            Closed::Limit => 2,
            Closed::Gone => 3,
        }
    }

    fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            0 => Closed::User,
            1 => Closed::Sender,
            2 => Closed::Limit,
            3 => Closed::Gone,
            _ => return Err(Error::InvalidArgs),
        })
    }
}

/// Oznameni, jak ho vidi prohlizec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// Cislo ve sluzbe (jedinecne, nikdy znovu).
    pub id: u64,
    /// Identita odesilatele (`org.example.app`, `system`).
    pub app_id: String,
    /// Jmeno pro uzivatele (z manifestu).
    pub app_name: String,
    pub title: String,
    pub body: String,
    pub priority: Priority,
    pub actions: Vec<Action>,
    /// Realny cas vzniku (ns od 1970).
    pub time: u64,
    /// Ma vyskocit (ne `Low`, ne potlacene Nerusit nebo limitem rychlosti).
    pub popup: bool,
    /// Odesilatel ho nahradil (prubeh): vyskakovaci se jen prekresli.
    pub replaced: bool,
}

fn put_str(w: &mut Writer, s: &str) {
    w.str(s);
}

impl Notification {
    pub fn encode(&self, w: &mut Writer) {
        w.u64(self.id);
        put_str(w, &self.app_id);
        put_str(w, &self.app_name);
        put_str(w, &self.title);
        put_str(w, &self.body);
        w.u8(self.priority.to_u8());
        encode_actions(w, &self.actions);
        w.u64(self.time).u8(self.popup as u8).u8(self.replaced as u8);
    }

    pub fn decode(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Notification {
            id: r.u64()?,
            app_id: r.str()?,
            app_name: r.str()?,
            title: r.str()?,
            body: r.str()?,
            priority: Priority::from_u8(r.u8()?)?,
            actions: decode_actions(r)?,
            time: r.u64()?,
            popup: r.u8()? != 0,
            replaced: r.u8()? != 0,
        })
    }
}

fn encode_actions(w: &mut Writer, a: &[Action]) {
    w.u8(a.len().min(255) as u8);
    for x in a.iter().take(255) {
        w.str(&x.id).str(&x.label);
    }
}

fn decode_actions(r: &mut Reader<'_>) -> Result<Vec<Action>> {
    let n = r.u8()? as usize;
    if n > MAX_ACTIONS {
        return Err(Error::InvalidArgs);
    }
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        v.push(Action { id: r.str()?, label: r.str()? });
    }
    Ok(v)
}

/// Kody operaci.
pub mod op {
    // Odesilatel.
    pub const NOTIFY: u32 = 1;
    pub const CLOSE: u32 = 2;
    // Prohlizec.
    pub const SUBSCRIBE: u32 = 10;
    pub const DISMISS: u32 = 11;
    pub const INVOKE: u32 = 12;
    pub const DISMISS_ALL: u32 = 13;
    pub const SET_DND: u32 = 14;
    pub const SEEN: u32 = 15;
    // Odpovedi a udalosti.
    pub const RESULT: u32 = 100;
    pub const SENT: u32 = 101;
    pub const ACTION: u32 = 102;
    pub const CLOSED: u32 = 103;
    pub const SNAPSHOT: u32 = 110;
    pub const ADDED: u32 = 111;
    pub const REMOVED: u32 = 112;
    pub const DND: u32 = 113;
}

/// Pozadavek odesilatele.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Send {
    /// Nove oznameni, nebo nahrazeni aktivniho se stejnym `key`.
    Notify { key: u64, title: String, body: String, priority: Priority, actions: Vec<Action> },
    /// Zavrit oznameni `key` (neexistujici = nic).
    Close { key: u64 },
}

/// Odpoved nebo udalost pro odesilatele.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendReply {
    /// Oznameni prijato (`popup` = vyskoci).
    Sent { key: u64, popup: bool },
    /// Pozadavek odmitnut (`INVALID_ARGS`, `NO_RESOURCES` = limit).
    Error { key: u64, error: Error },
    /// Uzivatel klikl na akci (`action` = id akce; prazdne = klik na
    /// oznameni samotne).
    Action { key: u64, action: String },
    /// Oznameni zmizelo.
    Closed { key: u64, why: Closed },
}

impl Send {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            Send::Notify { key, title, body, priority, actions } => {
                w = Writer::op(op::NOTIFY);
                w.u64(*key).str(title).str(body).u8(priority.to_u8());
                encode_actions(&mut w, actions);
            }
            Send::Close { key } => {
                w = Writer::op(op::CLOSE);
                w.u64(*key);
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let m = match r.u32()? {
            op::NOTIFY => Send::Notify {
                key: r.u64()?,
                title: r.str()?,
                body: r.str()?,
                priority: Priority::from_u8(r.u8()?)?,
                actions: decode_actions(&mut r)?,
            },
            op::CLOSE => Send::Close { key: r.u64()? },
            _ => return Err(Error::InvalidArgs),
        };
        Ok(m)
    }
}

impl SendReply {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            SendReply::Sent { key, popup } => {
                w = Writer::op(op::SENT);
                w.u64(*key).u8(*popup as u8);
            }
            SendReply::Error { key, error } => {
                w = Writer::op(op::RESULT);
                w.u64(*key).i32(*error as i32);
            }
            SendReply::Action { key, action } => {
                w = Writer::op(op::ACTION);
                w.u64(*key).str(action);
            }
            SendReply::Closed { key, why } => {
                w = Writer::op(op::CLOSED);
                w.u64(*key).u8(why.to_u8());
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        Ok(match r.u32()? {
            op::SENT => SendReply::Sent { key: r.u64()?, popup: r.u8()? != 0 },
            op::RESULT => SendReply::Error { key: r.u64()?, error: Error::from_code(r.i32()? as i64).ok_or(Error::InvalidArgs)? },
            op::ACTION => SendReply::Action { key: r.u64()?, action: r.str()? },
            op::CLOSED => SendReply::Closed { key: r.u64()?, why: Closed::from_u8(r.u8()?)? },
            _ => return Err(Error::InvalidArgs),
        })
    }
}

/// Rezim Nerusit uzivatele: konec (realny cas ns), `0` = vypnuto,
/// `u64::MAX` = dokud ho nevypne.
pub type DndUntil = u64;

/// Pozadavek prohlizece.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    /// Snimek (`Snapshot`, i ve vice zpravach) a pak zmeny.
    Subscribe,
    /// Zavrit oznameni (krizek).
    Dismiss { id: u64 },
    /// Klik na akci (`action` prazdne = na oznameni); oznameni se zavre.
    Invoke { id: u64, action: String },
    /// Vymazat vse (centrum oznameni).
    DismissAll,
    /// Nerusit do casu (`DndUntil`).
    SetDnd { until: DndUntil },
    /// Vyskakovaci oznameni zmizelo do centra (uz nevyskoci znovu, ani po
    /// restartu prohlizece).
    Seen { id: u64 },
}

/// Odpoved nebo zmena pro prohlizec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewReply {
    /// Cast snimku (`last` = posledni cast); pak `Dnd` se stavem.
    Snapshot { list: Vec<Notification>, last: bool },
    /// Nove nebo nahrazene oznameni.
    Added(Notification),
    Removed { id: u64, why: Closed },
    Dnd { until: DndUntil },
    /// Vysledek pozadavku bez dat (`Dismiss`, `Invoke`, ...).
    Result { op: u32, error: Option<Error> },
}

impl View {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            View::Subscribe => w = Writer::op(op::SUBSCRIBE),
            View::Dismiss { id } => {
                w = Writer::op(op::DISMISS);
                w.u64(*id);
            }
            View::Invoke { id, action } => {
                w = Writer::op(op::INVOKE);
                w.u64(*id).str(action);
            }
            View::DismissAll => w = Writer::op(op::DISMISS_ALL),
            View::SetDnd { until } => {
                w = Writer::op(op::SET_DND);
                w.u64(*until);
            }
            View::Seen { id } => {
                w = Writer::op(op::SEEN);
                w.u64(*id);
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        Ok(match r.u32()? {
            op::SUBSCRIBE => View::Subscribe,
            op::DISMISS => View::Dismiss { id: r.u64()? },
            op::INVOKE => View::Invoke { id: r.u64()?, action: r.str()? },
            op::DISMISS_ALL => View::DismissAll,
            op::SET_DND => View::SetDnd { until: r.u64()? },
            op::SEEN => View::Seen { id: r.u64()? },
            _ => return Err(Error::InvalidArgs),
        })
    }
}

impl ViewReply {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            ViewReply::Snapshot { list, last } => {
                w = Writer::op(op::SNAPSHOT);
                w.u8(*last as u8).u32(list.len() as u32);
                for n in list {
                    n.encode(&mut w);
                }
            }
            ViewReply::Added(n) => {
                w = Writer::op(op::ADDED);
                n.encode(&mut w);
            }
            ViewReply::Removed { id, why } => {
                w = Writer::op(op::REMOVED);
                w.u64(*id).u8(why.to_u8());
            }
            ViewReply::Dnd { until } => {
                w = Writer::op(op::DND);
                w.u64(*until);
            }
            ViewReply::Result { op: o, error } => {
                w = Writer::op(op::RESULT);
                w.u32(*o).i32(error.map_or(0, |e| e as i32));
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        Ok(match r.u32()? {
            op::SNAPSHOT => {
                let last = r.u8()? != 0;
                let n = r.u32()? as usize;
                if n > MAX_PER_MSG {
                    return Err(Error::InvalidArgs);
                }
                let mut list = Vec::with_capacity(n);
                for _ in 0..n {
                    list.push(Notification::decode(&mut r)?);
                }
                ViewReply::Snapshot { list, last }
            }
            op::ADDED => ViewReply::Added(Notification::decode(&mut r)?),
            op::REMOVED => ViewReply::Removed { id: r.u64()?, why: Closed::from_u8(r.u8()?)? },
            op::DND => ViewReply::Dnd { until: r.u64()? },
            op::RESULT => {
                let o = r.u32()?;
                let code = r.i32()?;
                ViewReply::Result { op: o, error: if code == 0 { None } else { Some(Error::from_code(code as i64).ok_or(Error::InvalidArgs)?) } }
            }
            _ => return Err(Error::InvalidArgs),
        })
    }
}

/// Text bez ridicich znaku (krome konce radku v tele), zkraceny na `max`
/// znaku (se `...`).
pub fn clean(s: &str, max: usize, newlines: bool) -> String {
    let chars: Vec<char> = s.chars().filter(|&c| !c.is_control() || (newlines && c == '\n')).collect();
    if chars.len() <= max {
        return chars.into_iter().collect();
    }
    let mut out: String = chars[..max.saturating_sub(3)].iter().collect();
    out.truncate(out.trim_end().len());
    out.push_str("...");
    out
}
