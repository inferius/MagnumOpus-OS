//! Zpravy `audiod` <-> ovladac zvukove karty a stranka stavu DMA.
//!
//! Ovladac vytvori pro vystupni stream DMA buffer (kruh `periods`
//! period) a preda ho sluzbe jako VMO: sluzba mixuje primo do pameti,
//! kterou cte zarizeni (zadna dalsi kopie). Stav (pozice DMA) zapisuje
//! ovladac do vlastni stranky [`StatusPage`], po kazde periode nastavi
//! udalost. Sluzba BDL, CORB ani registry nevidi.
//!
//! `OPEN_OUTPUT` -> [`OpenStream`] (skutecne hodnoty), handly [DMA buffer, stranka stavu,
//! udalost periody]. `START`/`STOP` stream spusti a zastavi (pri `STOP`
//! se pozice nenuluje). `OPEN_INPUT` je v protokolu pro nahravani;
//! ovladac ho zatim muze odmitnout (`NOT_SUPPORTED`).

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use wire::{Reader, Writer};

use crate::{Error, Format, Result};

pub mod op {
    pub const INFO: u32 = 1;
    pub const OPEN_OUTPUT: u32 = 2;
    pub const START: u32 = 3;
    pub const STOP: u32 = 4;
    pub const CLOSE: u32 = 5;
    pub const OPEN_INPUT: u32 = 6;
}

/// Druh vystupu (podle vychozi konfigurace pinu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OutputKind {
    LineOut = 0,
    Speaker = 1,
    Headphone = 2,
    /// HDMI nebo DisplayPort (zatim jen pripraveno, ne aktivni).
    Digital = 3,
    Other = 4,
}

impl OutputKind {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::LineOut,
            1 => Self::Speaker,
            2 => Self::Headphone,
            3 => Self::Digital,
            _ => Self::Other,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::LineOut => "line-out",
            Self::Speaker => "speaker",
            Self::Headphone => "headphone",
            Self::Digital => "hdmi",
            Self::Other => "other",
        }
    }
}

/// Stav konektoru (detekce zasunuti).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Jack {
    /// Pin detekci nema (pevny reproduktor, nebo ji kodek neumi).
    Unknown = 0,
    Unplugged = 1,
    Plugged = 2,
}

impl Jack {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Unplugged,
            2 => Self::Plugged,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Output {
    pub kind: OutputKind,
    pub jack: Jack,
    /// Hraje (cesta nastavena a neztlumena).
    pub active: bool,
}

/// Odpoved na `INFO`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub name: String,
    pub outputs: Vec<Output>,
    /// Format, ktery `OPEN_OUTPUT` bez dalsiho prijme (48 kHz nebo 44,1 kHz,
    /// 16 bitu, stereo).
    pub format: Format,
    /// Umi zarizeni nahravani?
    pub input: bool,
}

impl Info {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.str(&self.name);
        self.format.encode(&mut w);
        w.u8(self.input as u8).u8(self.outputs.len().min(255) as u8);
        for o in self.outputs.iter().take(255) {
            w.u8(o.kind as u8).u8(o.jack as u8).u8(o.active as u8);
        }
        w.finish()
    }

    pub fn decode(m: &[u8]) -> Result<Self> {
        let mut r = Reader::new(m);
        let name = r.str()?;
        let format = Format::decode(&mut r)?;
        let input = r.u8()? != 0;
        let n = r.u8()?;
        let mut outputs = Vec::with_capacity(n as usize);
        for _ in 0..n {
            outputs.push(Output {
                kind: OutputKind::from_u8(r.u8()?),
                jack: Jack::from_u8(r.u8()?),
                active: r.u8()? != 0,
            });
        }
        Ok(Self { name, outputs, format, input })
    }

    /// Ma zarizeni aktivni analogovy vystup?
    pub fn has_analog(&self) -> bool {
        self.outputs.iter().any(|o| o.active && o.kind != OutputKind::Digital)
    }

    /// Aktivni vystupy jako text (`line-out,headphone`).
    pub fn active_names(&self) -> String {
        let mut s = String::new();
        for o in self.outputs.iter().filter(|o| o.active) {
            if !s.is_empty() {
                s.push(',');
            }
            s.push_str(o.kind.name());
        }
        s
    }
}

/// Pozadavek `OPEN_OUTPUT` / `OPEN_INPUT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenStream {
    pub format: Format,
    /// Snimku na periodu (ovladac zaokrouhli podle zarovnani zarizeni).
    pub period: u32,
    pub periods: u32,
}

/// Nejmene a nejvic period v DMA bufferu.
pub const MIN_PERIODS: u32 = 2;
pub const MAX_PERIODS: u32 = 32;
/// Nejvetsi DMA buffer (bajty).
pub const MAX_BUFFER: u32 = 256 << 10;

impl OpenStream {
    pub fn encode(&self, code: u32) -> Vec<u8> {
        let mut w = Writer::op(code);
        self.encode_fields(&mut w);
        w.finish()
    }

    /// Pole bez kodu operace (pozadavek i odpoved je maji stejna).
    pub fn encode_fields(&self, w: &mut Writer) {
        self.format.encode(w);
        w.u32(self.period).u32(self.periods);
    }

    /// Precte pole za kodem operace.
    pub fn decode(r: &mut Reader) -> Result<Self> {
        let format = Format::decode(r)?;
        let s = Self { format, period: r.u32()?, periods: r.u32()? };
        let bytes = (s.period as u64) * (s.periods as u64) * (format.frame_bytes() as u64);
        if s.period == 0 || !(MIN_PERIODS..=MAX_PERIODS).contains(&s.periods) || bytes > MAX_BUFFER as u64 {
            return Err(Error::InvalidArgs);
        }
        Ok(s)
    }
}

/// Zakodovany pozadavek bez dat (INFO, START, STOP, CLOSE).
pub fn simple(code: u32) -> Vec<u8> {
    Writer::op(code).finish()
}

/// Odpoved na `OPEN_OUTPUT` (skutecne hodnoty po zaokrouhleni).
pub fn encode_opened(s: &OpenStream) -> Vec<u8> {
    let mut w = Writer::new();
    s.encode_fields(&mut w);
    w.finish()
}

pub fn decode_opened(m: &[u8]) -> Result<OpenStream> {
    OpenStream::decode(&mut Reader::new(m))
}

// ---- Stranka stavu (pise ovladac, cte sluzba) ----

pub const STATUS_MAGIC: u32 = u32::from_le_bytes(*b"AUS1");
pub const STATUS_SIZE: usize = 4096;

/// Priznaky stranky stavu.
pub mod flags {
    pub const RUNNING: u32 = 1;
    /// Zarizeni hlasilo chybu DMA (FIFO, deskriptor); stream zastaven.
    pub const ERROR: u32 = 2;
    /// Ovladac nema preruseni a pozici cte casovacem.
    pub const POLLED: u32 = 4;
}

const S_MAGIC: usize = 0;
const S_FLAGS: usize = 4;
const S_POS: usize = 8;
const S_IRQS: usize = 16;
const S_UPDATED: usize = 24;
const S_BUFFER: usize = 32;
const S_ERRORS: usize = 40;

/// Pohled na stranku stavu streamu.
pub struct StatusPage {
    base: *mut u8,
}

// SAFETY: jen atomicke pristupy do sdilene pameti.
unsafe impl Send for StatusPage {}

impl StatusPage {
    /// # Safety
    ///
    /// `base` ukazuje na zapisovatelnou pamet o delce [`STATUS_SIZE`],
    /// zarovnanou na 8 a platnou po celou dobu zivota vysledku.
    pub unsafe fn new(base: *mut u8) -> Self {
        Self { base }
    }

    fn u32(&self, off: usize) -> &AtomicU32 {
        // SAFETY: offset v prvnich 64 bajtech stranky, zarovnany.
        unsafe { &*(self.base.add(off) as *const AtomicU32) }
    }

    fn u64(&self, off: usize) -> &AtomicU64 {
        // SAFETY: jako u32.
        unsafe { &*(self.base.add(off) as *const AtomicU64) }
    }

    /// Zalozi stranku (ovladac): `buffer` = snimky DMA bufferu.
    pub fn init(&self, buffer: u32) {
        for off in [S_POS, S_IRQS, S_UPDATED, S_ERRORS] {
            self.u64(off).store(0, Ordering::Relaxed);
        }
        self.u64(S_BUFFER).store(buffer as u64, Ordering::Relaxed);
        self.u32(S_FLAGS).store(0, Ordering::Relaxed);
        self.u32(S_MAGIC).store(STATUS_MAGIC, Ordering::Release);
    }

    pub fn valid(&self) -> bool {
        self.u32(S_MAGIC).load(Ordering::Acquire) == STATUS_MAGIC
    }

    /// Pozice DMA: snimky prectene zarizenim od otevreni (monotonni).
    pub fn position(&self) -> u64 {
        self.u64(S_POS).load(Ordering::Acquire)
    }

    pub fn set_position(&self, frames: u64, now_ns: u64) {
        self.u64(S_UPDATED).store(now_ns, Ordering::Relaxed);
        self.u64(S_POS).store(frames, Ordering::Release);
    }

    /// Cas posledni aktualizace pozice (ns od startu systemu).
    pub fn updated_ns(&self) -> u64 {
        self.u64(S_UPDATED).load(Ordering::Relaxed)
    }

    pub fn irqs(&self) -> u64 {
        self.u64(S_IRQS).load(Ordering::Relaxed)
    }

    pub fn add_irq(&self) {
        self.u64(S_IRQS).fetch_add(1, Ordering::Relaxed);
    }

    pub fn errors(&self) -> u64 {
        self.u64(S_ERRORS).load(Ordering::Relaxed)
    }

    pub fn add_error(&self) {
        self.u64(S_ERRORS).fetch_add(1, Ordering::Relaxed);
    }

    pub fn buffer_frames(&self) -> u64 {
        self.u64(S_BUFFER).load(Ordering::Relaxed)
    }

    pub fn flags(&self) -> u32 {
        self.u32(S_FLAGS).load(Ordering::Acquire)
    }

    pub fn set_flags(&self, set: u32, clear: u32) {
        let f = self.u32(S_FLAGS);
        let old = f.load(Ordering::Relaxed);
        f.store((old & !clear) | set, Ordering::Release);
    }
}
