//! Protokol displeje: kompozitor <-> ovladac grafiky.
//!
//! Kompozitor kresli cely obraz do sve VMO (`ATTACH`, ovladac ji mapuje
//! jen pro cteni) a v `PRESENT` rekne, ktere obdelniky se zmenily.
//! Ovladac je zkopiruje do svych bufferu DMA, posle je zarizeni a po
//! dokonceni odpovi `PRESENTED` s casem (nahrada vsync u virtio-gpu 2D).
//!
//! Synchronizace po restartu kompozitoru (`docs/cs/obnova-sluzeb.md`):
//! konec kanalu k ovladaci drzi supervisor, nova instance dostane tentyz
//! konec, ve kterem mohou cekat odpovedi stare instance. Nova instance
//! posle `SYNC` s nahodnym tokenem a zahodi vse, co prijde pred `SYNCED`
//! se stejnym tokenem (ovladac, ktery `SYNC` nezna, odpovi `ERROR` s `op`
//! `SYNC` - i to je znacka). Pak znovu `INFO` a `ATTACH` (nova VMO nahradi
//! starou).
//!
//! `OPEN_PROVIDER` vyda **jednou** kanal poskytovatele render uzlu
//! (protokol `renderproto::node`, dalsi pokus `BAD_STATE`). Vyzada si ho
//! init pred predanim displeje kompozitoru a preda ho sluzbe `gpud`, ktera
//! render uzly rozdava; vlastnik displeje tak 3D nerozdava.
//! **Nastaveni obrazu** (`docs/cs/displej.md`): `CONNECTORS` vrati
//! konektory, monitory a jejich rezimy ([`crate::modes`]), `SET_MODE`
//! zmeni rezim vystupu (pak kompozitor posle nove `ATTACH` s novou
//! velikosti; do te doby vystup ukazuje cerny obraz). Ovladac s
//! `caps::HOTPLUG` posila bez vyzadani `HOTPLUG` pri zmene konektoru
//! (pripojeni, odpojeni monitoru) - klient znovu posle `CONNECTORS`.
//! `RESET` posle nova instance ovladace po padu predchozi (kanal drzi
//! init): klient znovu vybere vystup a posle `ATTACH`.
//! S `caps::VSYNC` ovladac kopiruje snimek do zadniho bufferu a prepne
//! ho ve svislem zatemneni (page flip); `PRESENTED` pak nese cas vsync.

use alloc::vec::Vec;

use crate::modes::{self, Connector};
use crate::{Error, ReadExt, Reader, Rect, Result, WriteExt, Writer, read_count};

pub mod op {
    pub const INFO: u32 = 1;
    pub const ATTACH: u32 = 2;
    pub const PRESENT: u32 = 3;
    pub const SYNC: u32 = 4;
    // 6..=15 a 106..=115: nastaveni obrazu (konektory, rezimy, hotplug).
    pub const CONNECTORS: u32 = 6;
    pub const SET_MODE: u32 = 7;
    pub const CONNECTORS_REPLY: u32 = 106;
    pub const MODE_SET: u32 = 107;
    /// Udalost bez pozadavku: zmena konektoru.
    pub const HOTPLUG: u32 = 108;
    /// Udalost bez pozadavku: ovladac bezi znovu (po padu, v rezimu
    /// firmwaru) - klient znovu vybere vystup a pripoji snimek.
    pub const RESET: u32 = 109;
    pub const MODES: u32 = 101;
    pub const ATTACHED: u32 = 102;
    pub const PRESENTED: u32 = 103;
    pub const SYNCED: u32 = 104;
    pub const OPEN_PROVIDER: u32 = 5;
    pub const PROVIDER: u32 = 105;
    pub const ERROR: u32 = 199;
}

/// Schopnosti ovladace (`MODES`).
pub mod caps {
    /// Zarizeni umi 3D (virgl).
    pub const VIRGL: u32 = 1 << 0;
    /// Ovladac ma render uzel s capsetem Venus (Vulkan, render uzly pres `OPEN_PROVIDER` a `gpud`).
    pub const VENUS: u32 = 1 << 1;
    // Bity 2..=7: nastaveni obrazu.
    /// Ovladac umi `SET_MODE` (vic nez jeden rezim).
    pub const MODESET: u32 = 1 << 2;
    /// Ovladac posila udalost `HOTPLUG`.
    pub const HOTPLUG: u32 = 1 << 3;
    /// `PRESENT` se zobrazi celym snimkem ve vsync (page flip), `PRESENTED`
    /// nese cas vsync.
    pub const VSYNC: u32 = 1 << 4;
    /// Vic vystupu (monitoru).
    pub const MULTI: u32 = 1 << 5;
}

/// Jeden vystup (scanout).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Output {
    pub w: u32,
    pub h: u32,
    pub enabled: bool,
}

/// Nejvic obdelniku v jednom `PRESENT` (vic = jeden obalujici).
pub const MAX_RECTS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Info,
    /// Handle VMO (B8G8R8A8, `stride * h` bajtu) jde v handlech zpravy.
    Attach { output: u32, w: u32, h: u32, stride: u32 },
    Present { seq: u64, output: u32, rects: Vec<Rect> },
    /// Znacka v proudu odpovedi: ovladac odpovi `SYNCED` se stejnym
    /// tokenem (vse pred ni patri predchozi instanci kompozitoru).
    Sync { token: u64 },
    /// Kanal poskytovatele render uzlu (odpoved `Provider`; jen jednou).
    OpenProvider,
    /// Vycet konektoru a rezimu (odpoved `Connectors`).
    Connectors,
    /// Nastavi rezim vystupu; `refresh_mhz` 0 = nejvyssi obnova pro rozmery.
    SetMode { output: u32, w: u32, h: u32, refresh_mhz: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Modes { caps: u32, outputs: Vec<Output> },
    Attached { output: u32 },
    Presented { seq: u64, time: u64 },
    Synced { token: u64 },
    /// Kanal poskytovatele render uzlu jde v handlech zpravy.
    Provider,
    Connectors { connectors: Vec<Connector> },
    /// Rezim nastaven (skutecna obnova); nasleduje `ATTACH` nove velikosti.
    ModeSet { output: u32, w: u32, h: u32, refresh_mhz: u32 },
    /// Konektor `output` se zmenil (bez pozadavku).
    Hotplug { output: u32 },
    /// Ovladac zacal znovu (bez pozadavku); predchozi `ATTACH` neplati.
    Reset,
    Error { op: u32, code: Error },
}

impl Reply {
    /// Udalost bez pozadavku (ne odpoved): klient ji pri cekani na
    /// odpoved preskoci a zpracuje zvlast.
    pub fn is_event(&self) -> bool {
        matches!(self, Reply::Hotplug { .. } | Reply::Reset)
    }

    /// Je odpoved znackou synchronizace s tokenem `token`? (`ERROR` na
    /// `SYNC` = ovladac synchronizaci nezna, ale odpovida po poradi.)
    pub fn ends_sync(&self, token: u64) -> bool {
        match self {
            Reply::Synced { token: t } => *t == token,
            Reply::Error { op: o, .. } => *o == op::SYNC,
            _ => false,
        }
    }
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            Request::Info => w = Writer::op(op::INFO),
            Request::Attach { output, w: aw, h, stride } => {
                w = Writer::op(op::ATTACH);
                w.u32(*output).u32(*aw).u32(*h).u32(*stride);
            }
            Request::Present { seq, output, rects } => {
                w = Writer::op(op::PRESENT);
                w.u64(*seq).u32(*output).u32(rects.len() as u32);
                for r in rects {
                    w.rect(r);
                }
            }
            Request::Sync { token } => {
                w = Writer::op(op::SYNC);
                w.u64(*token);
            }
            Request::OpenProvider => w = Writer::op(op::OPEN_PROVIDER),
            Request::Connectors => w = Writer::op(op::CONNECTORS),
            Request::SetMode { output, w: mw, h, refresh_mhz } => {
                w = Writer::op(op::SET_MODE);
                w.u32(*output).u32(*mw).u32(*h).u32(*refresh_mhz);
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let m = match r.u32()? {
            op::INFO => Request::Info,
            op::ATTACH => Request::Attach {
                output: r.u32()?,
                w: r.u32()?,
                h: r.u32()?,
                stride: r.u32()?,
            },
            op::PRESENT => {
                let seq = r.u64()?;
                let output = r.u32()?;
                let n = read_count(&mut r, 16, data.len())?;
                let mut rects = Vec::with_capacity(n);
                for _ in 0..n {
                    rects.push(r.rect()?);
                }
                Request::Present { seq, output, rects }
            }
            op::SYNC => Request::Sync { token: r.u64()? },
            op::OPEN_PROVIDER => Request::OpenProvider,
            op::CONNECTORS => Request::Connectors,
            op::SET_MODE => Request::SetMode {
                output: r.u32()?,
                w: r.u32()?,
                h: r.u32()?,
                refresh_mhz: r.u32()?,
            },
            _ => return Err(Error::NotSupported),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(m)
    }
}

impl Reply {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            Reply::Modes { caps, outputs } => {
                w = Writer::op(op::MODES);
                w.u32(*caps).u32(outputs.len() as u32);
                for o in outputs {
                    w.u32(o.w).u32(o.h).bool(o.enabled);
                }
            }
            Reply::Attached { output } => {
                w = Writer::op(op::ATTACHED);
                w.u32(*output);
            }
            Reply::Presented { seq, time } => {
                w = Writer::op(op::PRESENTED);
                w.u64(*seq).u64(*time);
            }
            Reply::Synced { token } => {
                w = Writer::op(op::SYNCED);
                w.u64(*token);
            }
            Reply::Provider => w = Writer::op(op::PROVIDER),
            Reply::Connectors { connectors } => {
                w = Writer::op(op::CONNECTORS_REPLY);
                modes::encode_connectors(&mut w, connectors);
            }
            Reply::ModeSet { output, w: mw, h, refresh_mhz } => {
                w = Writer::op(op::MODE_SET);
                w.u32(*output).u32(*mw).u32(*h).u32(*refresh_mhz);
            }
            Reply::Hotplug { output } => {
                w = Writer::op(op::HOTPLUG);
                w.u32(*output);
            }
            Reply::Reset => w = Writer::op(op::RESET),
            Reply::Error { op: o, code } => {
                w = Writer::op(op::ERROR);
                w.u32(*o).i32(code.code());
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let m = match r.u32()? {
            op::MODES => {
                let caps = r.u32()?;
                let n = read_count(&mut r, 9, data.len())?;
                let mut outputs = Vec::with_capacity(n);
                for _ in 0..n {
                    outputs.push(Output {
                        w: r.u32()?,
                        h: r.u32()?,
                        enabled: r.bool()?,
                    });
                }
                Reply::Modes { caps, outputs }
            }
            op::ATTACHED => Reply::Attached { output: r.u32()? },
            op::PRESENTED => Reply::Presented {
                seq: r.u64()?,
                time: r.u64()?,
            },
            op::SYNCED => Reply::Synced { token: r.u64()? },
            op::PROVIDER => Reply::Provider,
            op::CONNECTORS_REPLY => Reply::Connectors {
                connectors: modes::decode_connectors(&mut r, data.len())?,
            },
            op::MODE_SET => Reply::ModeSet {
                output: r.u32()?,
                w: r.u32()?,
                h: r.u32()?,
                refresh_mhz: r.u32()?,
            },
            op::HOTPLUG => Reply::Hotplug { output: r.u32()? },
            op::RESET => Reply::Reset,
            op::ERROR => Reply::Error {
                op: r.u32()?,
                code: Error::from_code(r.i32()? as i64).ok_or(Error::InvalidArgs)?,
            },
            _ => return Err(Error::NotSupported),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(m)
    }
}

/// Slouci obdelniky, aby jich nebylo vic nez [`MAX_RECTS`] (presah se
/// slije do jednoho obalujiciho); prazdne vynecha a orizne na vystup.
pub fn normalize_rects(rects: &[Rect], screen: Rect) -> Vec<Rect> {
    let mut out: Vec<Rect> = rects
        .iter()
        .map(|r| r.intersect(&screen))
        .filter(|r| !r.is_empty())
        .collect();
    if out.len() > MAX_RECTS {
        let all = out.iter().fold(Rect::default(), |a, r| a.union(r));
        out.clear();
        out.push(all);
    }
    out
}
