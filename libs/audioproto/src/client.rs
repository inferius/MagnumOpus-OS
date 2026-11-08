//! Zpravy aplikace <-> `audiod`.
//!
//! Pozadavek = kod operace u32 + pole; odpoved = stav i32 + data
//! ([`crate::encode_reply`]). Kazdy klient ma vlastni kanal; zavreni
//! kanalu zavre vsechny jeho streamy.
//!
//! Prehravani:
//! 1. `OPEN` (smer, format, pozadovana latence) -> [`Opened`] a handly
//!    [VMO bufferu ([`crate::ring`]), udalost "misto v bufferu"],
//! 2. klient pise vzorky do bufferu a posouva `write_pos`,
//! 3. `START`; sluzba cte buffer v taktu zarizeni. Po kazde periode
//!    nastavi udalost (klient ceka na ni, ne na kanal),
//! 4. `DRAIN` odpovi, az vsechno zapsane zaznelo; `CLOSE` stream zrusi.
//!
//! Nahravani (`Direction::Input`) ma stejny tvar: sluzba pise, klient cte
//! a udalost znamena "nova data". Zarizeni ho zatim nemusi umet
//! (`NOT_SUPPORTED`).

use alloc::string::String;
use alloc::vec::Vec;

use wire::{Reader, Writer};

use crate::{Direction, Error, Format, Result};

pub mod op {
    pub const OPEN: u32 = 1;
    pub const START: u32 = 2;
    pub const STOP: u32 = 3;
    pub const CLOSE: u32 = 4;
    pub const DRAIN: u32 = 5;
    pub const STREAM_VOLUME: u32 = 6;
    pub const MASTER_VOLUME: u32 = 7;
    pub const MUTE: u32 = 8;
    pub const STATUS: u32 = 9;
    /// Novy kanal ke sluzbe (napr. termd ho preda kazde relaci shellu).
    pub const CONNECT: u32 = 10;
    pub const STREAM_INFO: u32 = 11;
}

/// Nejmensi a nejvetsi pozadovana latence bufferu klienta (us).
pub const MIN_LATENCY_US: u32 = 5_000;
pub const MAX_LATENCY_US: u32 = 2_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// `latency_us` = kolik zvuku ma buffer klienta pojmout (sluzba
    /// zaokrouhli na periody a omezi na meze).
    Open { dir: Direction, format: Format, latency_us: u32 },
    Start { stream: u32 },
    /// Zastavi stream a zahodi neprehrana data.
    Stop { stream: u32 },
    Close { stream: u32 },
    /// Odpoved prijde, az vsechno zapsane zaznelo (pak se stream zastavi).
    Drain { stream: u32 },
    StreamVolume { stream: u32, percent: u8 },
    MasterVolume { percent: u8 },
    Mute { on: bool },
    Status,
    Connect,
    StreamInfo { stream: u32 },
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            Request::Open { dir, format, latency_us } => {
                w = Writer::op(op::OPEN);
                w.u8(*dir as u8);
                format.encode(&mut w);
                w.u32(*latency_us);
            }
            Request::Start { stream } => w = stream_op(op::START, *stream),
            Request::Stop { stream } => w = stream_op(op::STOP, *stream),
            Request::Close { stream } => w = stream_op(op::CLOSE, *stream),
            Request::Drain { stream } => w = stream_op(op::DRAIN, *stream),
            Request::StreamInfo { stream } => w = stream_op(op::STREAM_INFO, *stream),
            Request::StreamVolume { stream, percent } => {
                w = stream_op(op::STREAM_VOLUME, *stream);
                w.u8(*percent);
            }
            Request::MasterVolume { percent } => {
                w = Writer::op(op::MASTER_VOLUME);
                w.u8(*percent);
            }
            Request::Mute { on } => {
                w = Writer::op(op::MUTE);
                w.u8(*on as u8);
            }
            Request::Status => w = Writer::op(op::STATUS),
            Request::Connect => w = Writer::op(op::CONNECT),
        }
        w.finish()
    }

    pub fn decode(m: &[u8]) -> Result<Self> {
        let mut r = Reader::new(m);
        let req = match r.u32()? {
            op::OPEN => {
                let dir = Direction::from_u8(r.u8()?).ok_or(Error::InvalidArgs)?;
                let format = Format::decode(&mut r)?;
                Request::Open { dir, format, latency_us: r.u32()? }
            }
            op::START => Request::Start { stream: r.u32()? },
            op::STOP => Request::Stop { stream: r.u32()? },
            op::CLOSE => Request::Close { stream: r.u32()? },
            op::DRAIN => Request::Drain { stream: r.u32()? },
            op::STREAM_INFO => Request::StreamInfo { stream: r.u32()? },
            op::STREAM_VOLUME => Request::StreamVolume { stream: r.u32()?, percent: percent(r.u8()?)? },
            op::MASTER_VOLUME => Request::MasterVolume { percent: percent(r.u8()?)? },
            op::MUTE => Request::Mute { on: r.u8()? != 0 },
            op::STATUS => Request::Status,
            op::CONNECT => Request::Connect,
            _ => return Err(Error::NotSupported),
        };
        Ok(req)
    }
}

fn stream_op(code: u32, stream: u32) -> Writer {
    let mut w = Writer::op(code);
    w.u32(stream);
    w
}

fn percent(p: u8) -> Result<u8> {
    if p > 100 { Err(Error::InvalidArgs) } else { Ok(p) }
}

/// Odpoved na `OPEN` (handly: VMO bufferu, udalost).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Opened {
    pub stream: u32,
    /// Format bufferu klienta (stejny jako pozadovany; prevod resi sluzba).
    pub format: Format,
    /// Kapacita bufferu klienta (snimky jeho formatu).
    pub capacity: u32,
    /// Perioda zarizeni ve snimcich formatu klienta (takt udalosti).
    pub period: u32,
    /// Odhad latence od zapisu do bufferu po reproduktor pri plnem
    /// bufferu (us).
    pub latency_us: u32,
}

impl Opened {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(self.stream);
        self.format.encode(&mut w);
        w.u32(self.capacity).u32(self.period).u32(self.latency_us);
        w.finish()
    }

    pub fn decode(m: &[u8]) -> Result<Self> {
        let mut r = Reader::new(m);
        Ok(Self {
            stream: r.u32()?,
            format: Format::decode(&mut r)?,
            capacity: r.u32()?,
            period: r.u32()?,
            latency_us: r.u32()?,
        })
    }
}

/// Stav sluzby (`STATUS`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// Jmeno zarizeni (prazdne = zadne zarizeni).
    pub device: String,
    /// Aktivni vystupy zarizeni oddelene carkou (`line-out,headphone`).
    pub outputs: String,
    /// Format zarizeni (mix).
    pub format: Format,
    pub master: u8,
    pub muted: bool,
    /// Otevrene a bezici streamy.
    pub streams: u32,
    pub running: u32,
    /// Pozice DMA zarizeni (snimky od otevreni).
    pub hw_pos: u64,
    /// Preruseni od zarizeni (0 = ovladac se dotazuje casovacem).
    pub irqs: u64,
    /// Kolikrat sluzba nestihla smichat periodu vcas.
    pub device_underruns: u64,
    /// Latence od mixu po reproduktor (us).
    pub latency_us: u32,
    /// Perioda zarizeni (snimky).
    pub period: u32,
}

impl Status {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.str(&self.device).str(&self.outputs);
        self.format.encode(&mut w);
        w.u8(self.master).u8(self.muted as u8).u32(self.streams).u32(self.running);
        w.u64(self.hw_pos).u64(self.irqs).u64(self.device_underruns).u32(self.latency_us).u32(self.period);
        w.finish()
    }

    pub fn decode(m: &[u8]) -> Result<Self> {
        let mut r = Reader::new(m);
        Ok(Self {
            device: r.str()?,
            outputs: r.str()?,
            format: Format::decode(&mut r)?,
            master: r.u8()?,
            muted: r.u8()? != 0,
            streams: r.u32()?,
            running: r.u32()?,
            hw_pos: r.u64()?,
            irqs: r.u64()?,
            device_underruns: r.u64()?,
            latency_us: r.u32()?,
            period: r.u32()?,
        })
    }
}

/// Stav streamu (`STREAM_INFO`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamInfo {
    pub running: bool,
    /// Snimky formatu klienta, ktere uz zazneli.
    pub played: u64,
    pub underruns: u64,
    pub volume: u8,
}

impl StreamInfo {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(self.running as u8).u64(self.played).u64(self.underruns).u8(self.volume);
        w.finish()
    }

    pub fn decode(m: &[u8]) -> Result<Self> {
        let mut r = Reader::new(m);
        Ok(Self { running: r.u8()? != 0, played: r.u64()?, underruns: r.u64()?, volume: r.u8()? })
    }
}
