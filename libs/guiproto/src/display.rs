//! Protokol displeje: kompozitor <-> ovladac grafiky.
//!
//! Kompozitor kresli cely obraz do sve VMO (`ATTACH`, ovladac ji mapuje
//! jen pro cteni) a v `PRESENT` rekne, ktere obdelniky se zmenily.
//! Ovladac je zkopiruje do svych bufferu DMA, posle je zarizeni a po
//! dokonceni odpovi `PRESENTED` s casem (nahrada vsync u virtio-gpu 2D).

use alloc::vec::Vec;

use crate::{Error, ReadExt, Reader, Rect, Result, WriteExt, Writer, read_count};

pub mod op {
    pub const INFO: u32 = 1;
    pub const ATTACH: u32 = 2;
    pub const PRESENT: u32 = 3;
    pub const MODES: u32 = 101;
    pub const ATTACHED: u32 = 102;
    pub const PRESENTED: u32 = 103;
    pub const ERROR: u32 = 199;
}

/// Schopnosti ovladace (`MODES`).
pub mod caps {
    /// Zarizeni umi 3D (virgl).
    pub const VIRGL: u32 = 1 << 0;
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Modes { caps: u32, outputs: Vec<Output> },
    Attached { output: u32 },
    Presented { seq: u64, time: u64 },
    Error { op: u32, code: Error },
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
