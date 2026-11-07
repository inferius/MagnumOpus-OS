//! Protokol render uzlu: klient (aplikace, kompozitor, testovaci program)
//! <-> ovladac GPU (`user/virtio-gpu`).
//!
//! Jeden kanal = jeden klient = nejvys jeden kontext GPU (jako soubor
//! render uzlu DRM v Linuxu). Kanal otevre vlastnik displeje zadosti
//! `OPEN_RENDER` (`guiproto::display`) a preda ho dal. Zavreni kanalu
//! znici kontext i vsechny bloby klienta.
//!
//! Prubeh u Venusu (Vulkan pres virtio-gpu):
//! 1. `INFO` - capsety zarizeni (venus = 4) a velikost sdilene pameti,
//! 2. `GET_CAPSET` - obsah capsetu (verze protokolu Venus),
//! 3. `CREATE_CONTEXT` s capsetem,
//! 4. `CREATE_BLOB` - pamet hostitele; s `MAPPABLE` ji ovladac namapuje
//!    do sdilene pameti zarizeni a posle klientovi VMO tohoto okna,
//! 5. `SUBMIT` - proud prikazu kontextu (u Venusu serializovane prikazy
//!    Vulkanu); s `fence` odpovi az po zpracovani proudu.
//!
//! Cisla little-endian, kodovani pres `wire`.

#![no_std]

extern crate alloc;

#[cfg(test)]
mod tests;

use alloc::vec::Vec;

pub use abi::{Error, Result};
use wire::{Reader, Writer};

pub mod op {
    pub const INFO: u32 = 1;
    pub const GET_CAPSET: u32 = 2;
    pub const CREATE_CONTEXT: u32 = 3;
    pub const CREATE_BLOB: u32 = 4;
    pub const DESTROY_BLOB: u32 = 5;
    pub const SUBMIT: u32 = 6;
    pub const SCANOUT_BLOB: u32 = 7;

    pub const INFO_REPLY: u32 = 101;
    pub const CAPSET: u32 = 102;
    pub const CONTEXT: u32 = 103;
    pub const BLOB: u32 = 104;
    pub const DONE: u32 = 105;
    pub const SUBMITTED: u32 = 106;
    pub const ERROR: u32 = 199;
}

/// Pamet blobu (hodnoty virtio-gpu `BLOB_MEM_*`).
pub mod mem {
    /// Pamet hostitele vytvorena kontextem (`VkDeviceMemory` nebo nova
    /// sdilena pamet pro odpovedi a ring).
    pub const HOST3D: u32 = 2;
}

/// Priznaky blobu (hodnoty virtio-gpu `BLOB_FLAG_*`).
pub mod flags {
    /// Namapovat do sdilene pameti a poslat klientovi VMO.
    pub const MAPPABLE: u32 = 1;
    pub const SHAREABLE: u32 = 2;
    pub const CROSS_DEVICE: u32 = 4;
    pub const ALL: u32 = MAPPABLE | SHAREABLE | CROSS_DEVICE;
}

/// Nejvetsi proud prikazu v jedne zprave `SUBMIT` (zbytek limitu kanalu
/// na hlavicku). Vetsi data patri do blobu (u Venusu
/// `vkExecuteCommandStreamsMESA`).
pub const MAX_SUBMIT: usize = 60 * 1024;
/// Nejvetsi obsah capsetu.
pub const MAX_CAPSET: usize = 4096;
/// Nejvic capsetu v `INFO`.
pub const MAX_CAPSETS: usize = 16;

/// Capset zarizeni (`GET_CAPSET_INFO`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capset {
    pub id: u32,
    pub version: u32,
    pub size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Info,
    GetCapset { id: u32, version: u32 },
    CreateContext { capset: u32 },
    /// `blob_id` = objekt kontextu (u Venusu id `VkDeviceMemory`), 0 = nova
    /// sdilena pamet. `size` po strankach.
    CreateBlob { mem: u32, flags: u32, blob_id: u64, size: u64 },
    DestroyBlob { resource: u32 },
    /// Proud prikazu kontextu. `fence`: odpoved az po splneni plotu na
    /// casove ose `ring_idx` (u Venusu osa 0 = po zpracovani proudu).
    Submit { ring_idx: u8, fence: bool, data: Vec<u8> },
    /// Pokus (krok 4 Venusu): vystup 0 zobrazi blob klienta (`w` x `h`,
    /// format virtio-gpu, krok radku). `resource` 0 = vratit puvodni obraz.
    /// Blob musi byt podlozeny dmabuf.
    ScanoutBlob { resource: u32, w: u32, h: u32, format: u32, stride: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// Capsety a velikost sdilene pameti pro mapovane bloby (0 = zadna).
    Info { capsets: Vec<Capset>, shm_size: u64 },
    Capset(Vec<u8>),
    Context,
    /// Blob vytvoren; s `MAPPABLE` jde v handlech zpravy VMO okna
    /// (`size` bajtu) a `map_info` je typ cache od zarizeni.
    Blob { resource: u32, size: u64, map_info: u32 },
    /// Hotovo bez dat (`DESTROY_BLOB`).
    Done { op: u32 },
    Submitted,
    Error { op: u32, code: Error },
}

/// Kod operace zpravy (pro odpoved `Error` na zpravu, ktera nejde
/// rozebrat).
pub fn op_of(data: &[u8]) -> u32 {
    data.get(..4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
}

fn finish(r: Reader) -> Result<()> {
    if r.is_empty() { Ok(()) } else { Err(Error::InvalidArgs) }
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            Request::Info => w = Writer::op(op::INFO),
            Request::GetCapset { id, version } => {
                w = Writer::op(op::GET_CAPSET);
                w.u32(*id).u32(*version);
            }
            Request::CreateContext { capset } => {
                w = Writer::op(op::CREATE_CONTEXT);
                w.u32(*capset);
            }
            Request::CreateBlob { mem, flags, blob_id, size } => {
                w = Writer::op(op::CREATE_BLOB);
                w.u32(*mem).u32(*flags).u64(*blob_id).u64(*size);
            }
            Request::DestroyBlob { resource } => {
                w = Writer::op(op::DESTROY_BLOB);
                w.u32(*resource);
            }
            Request::Submit { ring_idx, fence, data } => {
                w = Writer::op(op::SUBMIT);
                w.u8(*ring_idx).u8(*fence as u8).bytes(data);
            }
            Request::ScanoutBlob { resource, w: bw, h, format, stride } => {
                w = Writer::op(op::SCANOUT_BLOB);
                w.u32(*resource).u32(*bw).u32(*h).u32(*format).u32(*stride);
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let m = match r.u32()? {
            op::INFO => Request::Info,
            op::GET_CAPSET => Request::GetCapset {
                id: r.u32()?,
                version: r.u32()?,
            },
            op::CREATE_CONTEXT => Request::CreateContext { capset: r.u32()? },
            op::CREATE_BLOB => {
                let mem = r.u32()?;
                let f = r.u32()?;
                if f & !flags::ALL != 0 {
                    return Err(Error::InvalidArgs);
                }
                Request::CreateBlob { mem, flags: f, blob_id: r.u64()?, size: r.u64()? }
            }
            op::DESTROY_BLOB => Request::DestroyBlob { resource: r.u32()? },
            op::SUBMIT => {
                let ring_idx = r.u8()?;
                let fence = match r.u8()? {
                    0 => false,
                    1 => true,
                    _ => return Err(Error::InvalidArgs),
                };
                let data = r.rest();
                if data.is_empty() || data.len() > MAX_SUBMIT || data.len() % 4 != 0 {
                    return Err(Error::InvalidArgs);
                }
                Request::Submit { ring_idx, fence, data: data.into() }
            }
            op::SCANOUT_BLOB => Request::ScanoutBlob {
                resource: r.u32()?,
                w: r.u32()?,
                h: r.u32()?,
                format: r.u32()?,
                stride: r.u32()?,
            },
            _ => return Err(Error::NotSupported),
        };
        finish(r)?;
        Ok(m)
    }
}

impl Reply {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            Reply::Info { capsets, shm_size } => {
                w = Writer::op(op::INFO_REPLY);
                w.u64(*shm_size).u32(capsets.len() as u32);
                for c in capsets {
                    w.u32(c.id).u32(c.version).u32(c.size);
                }
            }
            Reply::Capset(d) => {
                w = Writer::op(op::CAPSET);
                w.bytes(d);
            }
            Reply::Context => w = Writer::op(op::CONTEXT),
            Reply::Blob { resource, size, map_info } => {
                w = Writer::op(op::BLOB);
                w.u32(*resource).u64(*size).u32(*map_info);
            }
            Reply::Done { op: o } => {
                w = Writer::op(op::DONE);
                w.u32(*o);
            }
            Reply::Submitted => w = Writer::op(op::SUBMITTED),
            Reply::Error { op: o, code } => {
                w = Writer::op(op::ERROR);
                w.u32(*o).i32(code.code() as i32);
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let m = match r.u32()? {
            op::INFO_REPLY => {
                let shm_size = r.u64()?;
                let n = r.u32()? as usize;
                if n > MAX_CAPSETS {
                    return Err(Error::InvalidArgs);
                }
                let mut capsets = Vec::with_capacity(n);
                for _ in 0..n {
                    capsets.push(Capset {
                        id: r.u32()?,
                        version: r.u32()?,
                        size: r.u32()?,
                    });
                }
                Reply::Info { capsets, shm_size }
            }
            op::CAPSET => {
                let d = r.rest();
                if d.len() > MAX_CAPSET {
                    return Err(Error::InvalidArgs);
                }
                Reply::Capset(d.into())
            }
            op::CONTEXT => Reply::Context,
            op::BLOB => Reply::Blob {
                resource: r.u32()?,
                size: r.u64()?,
                map_info: r.u32()?,
            },
            op::DONE => Reply::Done { op: r.u32()? },
            op::SUBMITTED => Reply::Submitted,
            op::ERROR => Reply::Error {
                op: r.u32()?,
                code: Error::from_code(r.i32()? as i64).ok_or(Error::InvalidArgs)?,
            },
            _ => return Err(Error::NotSupported),
        };
        finish(r)?;
        Ok(m)
    }
}
