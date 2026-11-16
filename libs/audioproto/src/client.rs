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
    pub const DEVICES: u32 = 12;
    pub const SELECT: u32 = 13;
    /// Aplikace s proudy (identita z manifestu) a jejich hlasitost.
    pub const APPS: u32 = 14;
    /// Hlasitost aplikace (vsech jejich proudu, i z vic procesu).
    pub const APP_VOLUME: u32 = 15;
    /// Stav, zarizeni a aplikace v jedne odpovedi (panel Zvuk v liste).
    pub const OVERVIEW: u32 = 16;
    /// Zarizeni pro nahravani (jinak vstup aktivniho nebo prvni se vstupem).
    pub const SELECT_INPUT: u32 = 17;
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
    /// Seznam zvukovych zarizeni ([`DeviceEntry`]).
    Devices,
    /// Prepnout na zarizeni `index` (bezici streamy hraji dal na novem).
    Select { index: u8 },
    /// Seznam aplikaci s proudy ([`AppEntry`]).
    Apps,
    /// Hlasitost aplikace `app` (identita z manifestu) v procentech.
    AppVolume { app: String, percent: u8 },
    /// [`Overview`]: stav, zarizeni a aplikace.
    Overview,
    /// Nahravat ze zarizeni `index`; 255 = automaticky (vstup aktivniho
    /// zarizeni, jinak prvni se vstupem).
    SelectInput { index: u8 },
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
            Request::Devices => w = Writer::op(op::DEVICES),
            Request::Select { index } => {
                w = Writer::op(op::SELECT);
                w.u8(*index);
            }
            Request::Apps => w = Writer::op(op::APPS),
            Request::AppVolume { app, percent } => {
                w = Writer::op(op::APP_VOLUME);
                w.str(app).u8(*percent);
            }
            Request::Overview => w = Writer::op(op::OVERVIEW),
            Request::SelectInput { index } => {
                w = Writer::op(op::SELECT_INPUT);
                w.u8(*index);
            }
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
            op::DEVICES => Request::Devices,
            op::SELECT => Request::Select { index: r.u8()? },
            op::APPS => Request::Apps,
            op::APP_VOLUME => {
                let app = r.str()?;
                if app.is_empty() || app.len() > MAX_APP_ID {
                    return Err(Error::InvalidArgs);
                }
                Request::AppVolume { app, percent: percent(r.u8()?)? }
            }
            op::OVERVIEW => Request::Overview,
            op::SELECT_INPUT => Request::SelectInput { index: r.u8()? },
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
    /// Umi aktivni zarizeni nahravat?
    pub input: bool,
    /// Index aktivniho zarizeni (`DEVICES`).
    pub device_index: u8,
    /// Jmena aplikaci, ktere prave nahravaji (indikator v liste).
    pub capturing: Vec<String>,
}

impl Status {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        self.put(&mut w);
        w.finish()
    }

    pub fn decode(m: &[u8]) -> Result<Self> {
        Self::get(&mut Reader::new(m))
    }

    pub fn put(&self, w: &mut Writer) {
        w.str(&self.device).str(&self.outputs);
        self.format.encode(w);
        w.u8(self.master).u8(self.muted as u8).u32(self.streams).u32(self.running);
        w.u64(self.hw_pos).u64(self.irqs).u64(self.device_underruns).u32(self.latency_us).u32(self.period);
        w.u8(self.input as u8).u8(self.device_index);
        w.u32(self.capturing.len() as u32);
        for a in &self.capturing {
            w.str(a);
        }
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            device: r.str()?,
            outputs: r.str()?,
            format: Format::decode(r)?,
            master: r.u8()?,
            muted: r.u8()? != 0,
            streams: r.u32()?,
            running: r.u32()?,
            hw_pos: r.u64()?,
            irqs: r.u64()?,
            device_underruns: r.u64()?,
            latency_us: r.u32()?,
            period: r.u32()?,
            input: r.u8()? != 0,
            device_index: r.u8()?,
            capturing: {
                let n = r.u32()? as usize;
                // Nejvys tolik, kolik muze byt streamu (ochrana pred alokaci).
                if n > 256 {
                    return Err(Error::InvalidArgs);
                }
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(r.str()?);
                }
                v
            },
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

/// Polozka seznamu zarizeni (`DEVICES`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceEntry {
    pub index: u8,
    pub name: String,
    /// Aktivni vystupy (`line-out,headphone`), prazdne = zadny.
    pub outputs: String,
    pub input: bool,
    /// Prave pouzivane zarizeni.
    pub active: bool,
}

impl DeviceEntry {
    pub fn encode_list(list: &[DeviceEntry]) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(list.len().min(255) as u8);
        for d in list.iter().take(255) {
            w.u8(d.index).str(&d.name).str(&d.outputs).u8(d.input as u8).u8(d.active as u8);
        }
        w.finish()
    }

    pub fn decode_list(m: &[u8]) -> Result<Vec<DeviceEntry>> {
        let mut r = Reader::new(m);
        let n = r.u8()?;
        let mut v = Vec::with_capacity(n as usize);
        for _ in 0..n {
            v.push(DeviceEntry {
                index: r.u8()?,
                name: r.str()?,
                outputs: r.str()?,
                input: r.u8()? != 0,
                active: r.u8()? != 0,
            });
        }
        Ok(v)
    }
}

/// Nejdelsi identita aplikace (`AppVolume`, [`AppEntry`]).
pub const MAX_APP_ID: usize = 128;
/// Nejvic aplikaci a zarizeni v [`Overview`].
pub const MAX_LIST: usize = 64;

/// Aplikace s proudy (`APPS`): identita z manifestu, jmeno, hlasitost
/// aplikace (pro vsechny jeji proudy, i z vic procesu: proud Wine je jedna
/// polozka `org.winehq.wine`), zda hraje a zda nahrava.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEntry {
    pub app_id: String,
    pub name: String,
    pub volume: u8,
    pub playing: bool,
    pub capturing: bool,
}

impl AppEntry {
    pub fn put(&self, w: &mut Writer) {
        w.str(&self.app_id).str(&self.name).u8(self.volume).u8(self.playing as u8).u8(self.capturing as u8);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        Ok(Self { app_id: r.str()?, name: r.str()?, volume: r.u8()?, playing: r.u8()? != 0, capturing: r.u8()? != 0 })
    }

    pub fn encode_list(list: &[AppEntry]) -> Vec<u8> {
        let mut w = Writer::new();
        put_apps(&mut w, list);
        w.finish()
    }

    pub fn decode_list(m: &[u8]) -> Result<Vec<AppEntry>> {
        get_apps(&mut Reader::new(m))
    }
}

fn put_apps(w: &mut Writer, list: &[AppEntry]) {
    let n = list.len().min(MAX_LIST);
    w.u8(n as u8);
    for a in &list[..n] {
        a.put(w);
    }
}

fn get_apps(r: &mut Reader) -> Result<Vec<AppEntry>> {
    let n = r.u8()? as usize;
    if n > MAX_LIST {
        return Err(Error::InvalidArgs);
    }
    (0..n).map(|_| AppEntry::get(r)).collect()
}

/// Odpoved `OVERVIEW`: vse pro panel Zvuk v jedne zprave (kompozitor se
/// pta pravidelne a nesmi na sluzbu cekat).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overview {
    pub status: Status,
    pub devices: Vec<DeviceEntry>,
    pub apps: Vec<AppEntry>,
    /// Zarizeni, ze ktereho se nahrava (`None` = zadny vstup).
    pub input: Option<u8>,
}

impl Overview {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        self.status.put(&mut w);
        let n = self.devices.len().min(MAX_LIST);
        w.u8(n as u8);
        for d in &self.devices[..n] {
            w.u8(d.index).str(&d.name).str(&d.outputs).u8(d.input as u8).u8(d.active as u8);
        }
        put_apps(&mut w, &self.apps);
        w.u8(self.input.unwrap_or(u8::MAX));
        w.finish()
    }

    pub fn decode(m: &[u8]) -> Result<Self> {
        let mut r = Reader::new(m);
        let status = Status::get(&mut r)?;
        let n = r.u8()? as usize;
        if n > MAX_LIST {
            return Err(Error::InvalidArgs);
        }
        let mut devices = Vec::with_capacity(n);
        for _ in 0..n {
            devices.push(DeviceEntry { index: r.u8()?, name: r.str()?, outputs: r.str()?, input: r.u8()? != 0, active: r.u8()? != 0 });
        }
        let apps = get_apps(&mut r)?;
        let input = Some(r.u8()?).filter(|&i| i != u8::MAX);
        Ok(Self { status, devices, apps, input })
    }
}
