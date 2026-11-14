//! Protokol medii: aplikace <-> sluzba `mediad` <-> proces dekoderu
//! (`ffdec`, zatim testovaci `mediatest`). Popis: `docs/cs/multimedia.md`.
//!
//! - jeden kanal klienta = jedna relace prehravani; dalsi relace = dalsi
//!   kanal (`CONNECT` na ridicim kanalu sluzby),
//! - [`Request`]: pozadavky aplikace; `mediad` je po kontrole posila
//!   dekoderu ve stejnem kodovani (plus [`op::PROBE`]),
//! - zpravy od sluzby (a od dekoderu ke sluzbe) zacinaji znackou u32:
//!   [`TAG_REPLY`] = odpoved na posledni pozadavek (status i32 + data),
//!   jinak udalost [`Event`] (`ev::*`); udalosti chodi kdykoli,
//! - [`source`]: protokol zdroje bajtu (cteni po kusech), kterym dekoder
//!   cte vstup - nikdy nedostane handle souboru,
//! - [`clock`]: hodiny zvuku a vyber snimku pro A/V synchronizaci,
//! - [`testmedia`]: testovaci format a generator obrazu a tonu pro
//!   dekoder bez FFmpeg,
//! - [`conn`] (feature `client`): klient nad kanaly.
//!
//! Cteni neplatne zpravy vraci `INVALID_ARGS`, nikdy paniku: sluzba i
//! aplikace ctou zpravy od procesu, ktery zpracovava neduveryhodna data.

#![no_std]

extern crate alloc;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod c_header_tests;

pub mod clock;
pub mod source;
pub mod testmedia;

#[cfg(feature = "client")]
pub mod conn;

use alloc::string::String;
use alloc::vec::Vec;

pub use abi::{Error, Result};
use bufdesc::BufDesc;
use wire::{Reader, Writer};

/// Nejvic bufferu v poolu videa.
pub const MAX_POOL: usize = 16;
/// Nejvic proudu v jednom souboru.
pub const MAX_STREAMS: usize = 32;
/// Znacka zpravy od sluzby: odpoved na pozadavek.
pub const TAG_REPLY: u32 = 0;

/// Kody pozadavku (aplikace -> `mediad`, `mediad` -> dekoder).
pub mod op {
    /// Jen na ridicim kanalu sluzby: novy kanal klienta (handle odpovedi).
    pub const CONNECT: u32 = 1;
    /// Druh zdroje u8 ([`super::source_kind`]), velikost u64 (u VMO) ->
    /// [`super::Info`]. Handly: [VMO jen pro cteni | kanal zdroje].
    pub const OPEN: u32 = 2;
    /// Video i32, zvuk i32, titulky i32 (index proudu, -1 = zadny).
    pub const SELECT: u32 = 3;
    /// Pocet u8 + `BufDesc` kazdeho bufferu. Handly: VMO bufferu (v poradi)
    /// a nakonec plot snimku (pravo SIGNAL; hodnota = poradove cislo snimku).
    pub const POOL: u32 = 4;
    /// Frekvence u32, kanaly u8, vzorek u8 (`audioproto::Sample`), kapacita
    /// u32 (ramce). Handly: [VMO kruhu audiod, udalost periody].
    pub const AUDIO: u32 = 5;
    pub const PLAY: u32 = 6;
    pub const PAUSE: u32 = 7;
    /// Cas us u64, presne u8 -> skutecny cas u64 (klicovy snimek nebo presne).
    pub const SEEK: u32 = 8;
    /// Buffer u8: aplikace ho uz nepotrebuje (kompozitor ho uvolnil).
    pub const RELEASE: u32 = 9;
    /// Zavrit relaci (dekoder skonci).
    pub const CLOSE: u32 = 10;
    /// Po ztrate dekoderu: novy dekoder se stejnym zdrojem, poolem a kruhem
    /// -> [`super::Info`]; pozice se obnovi `SEEK`.
    pub const REOPEN: u32 = 11;
    /// Jen `mediad` -> dekoder: velikost zdroje u64 -> [`super::Info`].
    /// Handly: [kanal zdroje].
    pub const PROBE: u32 = 50;
}

/// Kody udalosti (dekoder -> `mediad` -> aplikace).
pub mod ev {
    /// Buffer u8, pts us u64, trvani us u32, poradi u64 (hodnota plotu).
    pub const FRAME: u32 = 100;
    /// Pozice v kruhu zvuku u64 (ramce), pts us u64: prvni vzorek od teto
    /// pozice ma tento cas (po startu a po `SEEK`).
    pub const AUDIO_BASE: u32 = 101;
    /// Konec vsech vybranych proudu.
    pub const END: u32 = 102;
    /// Kod chyby i32, dekoder ztracen u8 (relace jde obnovit `REOPEN`).
    pub const ERROR: u32 = 103;
}

/// Druh zdroje v `OPEN`.
pub mod source_kind {
    /// VMO s celym souborem (jen pro cteni).
    pub const VMO: u8 = 1;
    /// Kanal protokolu [`crate::source`] (soubor, sit, pamet aplikace).
    pub const CHANNEL: u8 = 2;
}

/// Druh proudu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Video = 1,
    Audio = 2,
    Subtitle = 3,
}

impl Kind {
    fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            1 => Kind::Video,
            2 => Kind::Audio,
            3 => Kind::Subtitle,
            _ => return Err(Error::InvalidArgs),
        })
    }
}

/// Popis jednoho proudu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stream {
    pub kind: Kind,
    /// Jmeno kodeku (jako ve FFmpeg: "av1", "opus", ...).
    pub codec: String,
    pub width: u32,
    pub height: u32,
    /// Snimkova frekvence jako zlomek (0/0 = promenna).
    pub fps_num: u32,
    pub fps_den: u32,
    pub rate: u32,
    pub channels: u8,
    /// Jazyk (BCP 47 nebo prazdny).
    pub lang: String,
    /// Kodek je v tomto obrazu systemu dostupny (jinak jde jen popsat).
    pub available: bool,
}

/// Popis otevreneho zdroje.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Info {
    /// Delka v us (0 = nezname, napr. zive vysilani).
    pub duration_us: u64,
    pub streams: Vec<Stream>,
}

impl Info {
    pub fn write(&self, w: &mut Writer) {
        w.u64(self.duration_us).u8(self.streams.len() as u8);
        for s in &self.streams {
            w.u8(s.kind as u8).str(&s.codec).u32(s.width).u32(s.height);
            w.u32(s.fps_num).u32(s.fps_den).u32(s.rate).u8(s.channels);
            w.str(&s.lang).u8(s.available as u8);
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        self.write(&mut w);
        w.finish()
    }

    pub fn read(r: &mut Reader) -> Result<Self> {
        let duration_us = r.u64()?;
        let n = r.u8()? as usize;
        if n > MAX_STREAMS {
            return Err(Error::InvalidArgs);
        }
        let mut streams = Vec::with_capacity(n);
        for _ in 0..n {
            streams.push(Stream {
                kind: Kind::from_u8(r.u8()?)?,
                codec: r.str()?,
                width: r.u32()?,
                height: r.u32()?,
                fps_num: r.u32()?,
                fps_den: r.u32()?,
                rate: r.u32()?,
                channels: r.u8()?,
                lang: r.str()?,
                available: bool_of(r.u8()?)?,
            });
        }
        Ok(Self { duration_us, streams })
    }

    pub fn decode(m: &[u8]) -> Result<Self> {
        let mut r = Reader::new(m);
        let i = Self::read(&mut r)?;
        end(&r)?;
        Ok(i)
    }

    /// Index prvniho proudu daneho druhu.
    pub fn first(&self, kind: Kind) -> Option<usize> {
        self.streams.iter().position(|s| s.kind == kind)
    }
}

/// Format kruhu zvuku (kopie potrebnych udaju z `audioproto::Opened`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioRing {
    pub rate: u32,
    pub channels: u8,
    /// `audioproto::Sample` jako u8.
    pub sample: u8,
    /// Kapacita kruhu v ramcich.
    pub capacity: u32,
}

/// Pozadavek aplikace (a `mediad` -> dekoder).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Connect,
    Open { kind: u8, size: u64 },
    Select { video: i32, audio: i32, subtitle: i32 },
    Pool { descs: Vec<BufDesc> },
    Audio(AudioRing),
    Play,
    Pause,
    Seek { pts_us: u64, exact: bool },
    Release { buffer: u8 },
    Close,
    Reopen,
    Probe { size: u64 },
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            Request::Connect => w = Writer::op(op::CONNECT),
            Request::Open { kind, size } => {
                w = Writer::op(op::OPEN);
                w.u8(*kind).u64(*size);
            }
            Request::Select { video, audio, subtitle } => {
                w = Writer::op(op::SELECT);
                w.i32(*video).i32(*audio).i32(*subtitle);
            }
            Request::Pool { descs } => {
                w = Writer::op(op::POOL);
                w.u8(descs.len() as u8);
                for d in descs {
                    d.write(&mut w);
                }
            }
            Request::Audio(a) => {
                w = Writer::op(op::AUDIO);
                w.u32(a.rate).u8(a.channels).u8(a.sample).u32(a.capacity);
            }
            Request::Play => w = Writer::op(op::PLAY),
            Request::Pause => w = Writer::op(op::PAUSE),
            Request::Seek { pts_us, exact } => {
                w = Writer::op(op::SEEK);
                w.u64(*pts_us).u8(*exact as u8);
            }
            Request::Release { buffer } => {
                w = Writer::op(op::RELEASE);
                w.u8(*buffer);
            }
            Request::Close => w = Writer::op(op::CLOSE),
            Request::Reopen => w = Writer::op(op::REOPEN),
            Request::Probe { size } => {
                w = Writer::op(op::PROBE);
                w.u64(*size);
            }
        }
        w.finish()
    }

    pub fn decode(m: &[u8]) -> Result<Self> {
        let mut r = Reader::new(m);
        let req = match r.u32()? {
            op::CONNECT => Request::Connect,
            op::OPEN => {
                let kind = r.u8()?;
                if kind != source_kind::VMO && kind != source_kind::CHANNEL {
                    return Err(Error::InvalidArgs);
                }
                Request::Open { kind, size: r.u64()? }
            }
            op::SELECT => Request::Select {
                video: stream_index(r.i32()?)?,
                audio: stream_index(r.i32()?)?,
                subtitle: stream_index(r.i32()?)?,
            },
            op::POOL => {
                let n = r.u8()? as usize;
                if n == 0 || n > MAX_POOL {
                    return Err(Error::InvalidArgs);
                }
                let mut descs = Vec::with_capacity(n);
                for _ in 0..n {
                    descs.push(BufDesc::read(&mut r).map_err(|_| Error::InvalidArgs)?);
                }
                Request::Pool { descs }
            }
            op::AUDIO => {
                let a = AudioRing { rate: r.u32()?, channels: r.u8()?, sample: r.u8()?, capacity: r.u32()? };
                if a.rate == 0 || a.channels == 0 || a.capacity == 0 || !(1..=5).contains(&a.sample) {
                    return Err(Error::InvalidArgs);
                }
                Request::Audio(a)
            }
            op::PLAY => Request::Play,
            op::PAUSE => Request::Pause,
            op::SEEK => Request::Seek { pts_us: r.u64()?, exact: bool_of(r.u8()?)? },
            op::RELEASE => {
                let buffer = r.u8()?;
                if buffer as usize >= MAX_POOL {
                    return Err(Error::InvalidArgs);
                }
                Request::Release { buffer }
            }
            op::CLOSE => Request::Close,
            op::REOPEN => Request::Reopen,
            op::PROBE => Request::Probe { size: r.u64()? },
            _ => return Err(Error::NotSupported),
        };
        end(&r)?;
        Ok(req)
    }
}

/// Udalost od dekoderu (preposlana aplikaci).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Frame { buffer: u8, pts_us: u64, duration_us: u32, serial: u64 },
    AudioBase { ring_pos: u64, pts_us: u64 },
    End,
    Error { code: i32, decoder_lost: bool },
}

impl Event {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match *self {
            Event::Frame { buffer, pts_us, duration_us, serial } => {
                w = Writer::op(ev::FRAME);
                w.u8(buffer).u64(pts_us).u32(duration_us).u64(serial);
            }
            Event::AudioBase { ring_pos, pts_us } => {
                w = Writer::op(ev::AUDIO_BASE);
                w.u64(ring_pos).u64(pts_us);
            }
            Event::End => w = Writer::op(ev::END),
            Event::Error { code, decoder_lost } => {
                w = Writer::op(ev::ERROR);
                w.i32(code).u8(decoder_lost as u8);
            }
        }
        w.finish()
    }

    pub fn decode(m: &[u8]) -> Result<Self> {
        let mut r = Reader::new(m);
        let e = match r.u32()? {
            ev::FRAME => {
                let buffer = r.u8()?;
                if buffer as usize >= MAX_POOL {
                    return Err(Error::InvalidArgs);
                }
                Event::Frame { buffer, pts_us: r.u64()?, duration_us: r.u32()?, serial: r.u64()? }
            }
            ev::AUDIO_BASE => Event::AudioBase { ring_pos: r.u64()?, pts_us: r.u64()? },
            ev::END => Event::End,
            ev::ERROR => Event::Error { code: r.i32()?, decoder_lost: bool_of(r.u8()?)? },
            _ => return Err(Error::NotSupported),
        };
        end(&r)?;
        Ok(e)
    }
}

/// Zprava od sluzby (nebo od dekoderu ke sluzbe).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming<'a> {
    /// Odpoved: vysledek a data.
    Reply(Result<&'a [u8]>),
    Event(Event),
}

/// Zakoduje odpoved (znacka [`TAG_REPLY`], status, data).
pub fn encode_reply(r: Result<Vec<u8>>) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(TAG_REPLY);
    match r {
        Ok(d) => {
            w.i32(0).bytes(&d);
        }
        Err(e) => {
            w.i32(e.code());
        }
    }
    w.finish()
}

/// Rozebere zpravu od sluzby: odpoved nebo udalost.
pub fn decode_incoming(m: &[u8]) -> Result<Incoming<'_>> {
    let mut r = Reader::new(m);
    if r.u32()? != TAG_REPLY {
        return Ok(Incoming::Event(Event::decode(m)?));
    }
    let status = r.i32()?;
    if status != 0 {
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        return Ok(Incoming::Reply(Err(Error::from_code(status as i64).unwrap_or(Error::Corrupted))));
    }
    Ok(Incoming::Reply(Ok(r.rest())))
}

/// Odpoved na `SEEK`: skutecny cas.
pub fn encode_u64(v: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(v);
    w.finish()
}

pub fn decode_u64(m: &[u8]) -> Result<u64> {
    let mut r = Reader::new(m);
    let v = r.u64()?;
    end(&r)?;
    Ok(v)
}

fn stream_index(v: i32) -> Result<i32> {
    if v < -1 || v >= MAX_STREAMS as i32 {
        return Err(Error::InvalidArgs);
    }
    Ok(v)
}

fn bool_of(v: u8) -> Result<bool> {
    match v {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Error::InvalidArgs),
    }
}

fn end(r: &Reader) -> Result<()> {
    if r.is_empty() { Ok(()) } else { Err(Error::InvalidArgs) }
}
