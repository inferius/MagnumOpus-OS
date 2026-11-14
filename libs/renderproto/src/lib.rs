//! Protokol tridy render: klient (aplikace, kompozitor, testovaci program)
//! <-> ovladac GPU. Nahrazuje `gpuproto` (jen Venus).
//!
//! Dva kanaly:
//! - **render uzel** ([`Request`], [`Reply`]): jeden kanal = jeden klient
//!   = nejvys jeden kontext GPU (jako soubor render uzlu DRM). Zavreni
//!   kanalu znici kontext i vsechny buffery klienta.
//! - **poskytovatel** ([`node`]): ovladac ho vyda jednou (na kanalu displeje
//!   `OPEN_PROVIDER`), drzi ho sluzba `gpud` a otevira na nem render uzly
//!   pro klienty.
//!
//! Spolecna obalka pro vsechny ovladace; data specificka pro ovladac (proud
//! Venusu, uAPI nouveau pro NVK) jdou uvnitr (`uapi`, `data`):
//!
//! | zprava | co | Venus |
//! |---|---|---|
//! | `INFO` | trida ovladace, uAPI (capsety), sdilena pamet | capsety virtio-gpu |
//! | `QUERY` | dotaz specificky pro uAPI | obsah capsetu |
//! | `CREATE_CONTEXT` | kontext s uAPI | kontext s capsetem |
//! | `CREATE_BUFFER` | buffer ovladace (s `MAPPABLE` + VMO) | blob `HOST3D` (`driver_id` = `VkDeviceMemory`) |
//! | `IMPORT_BUFFER` | buffer z VMO klienta + `bufdesc` | zatim `NOT_SUPPORTED` |
//! | `DESTROY` | znicit buffer | znicit blob |
//! | `BIND` | mapovani do VM GPU (`VM_BIND`) | nic (`DONE`) |
//! | `SUBMIT` | fronta, ploty k cekani a k signalizaci, data | proud prikazu na `ring_idx` = fronta |
//!
//! **Ploty** jsou objekty jadra (`docs/cs/ploty.md`). `SUBMIT` nese v handlech
//! zpravy nejdriv ploty k cekani (pravo `WAIT`), pak ploty k signalizaci
//! (pravo `SIGNAL`); v datech jen hodnoty. Ovladac odpovi `SUBMITTED` hned,
//! jak prikaz prijme; dokonceni hlasi jen ploty. Ovladac posle prikaz
//! zarizeni az po splneni vsech cekacich plotu. Chyba nebo opusteni
//! cekaciho plotu, chyba zarizeni nebo prekroceni [`HANG_TIMEOUT`] = vsechny
//! signalizacni ploty skonci chybou (`FENCE_FAILED`).
//!
//! Cisla little-endian, kodovani pres `wire`. Popis: `docs/cs/render.md`.

#![no_std]

extern crate alloc;

#[cfg(test)]
mod tests;

pub mod node;

use alloc::vec::Vec;
use core::time::Duration;

pub use abi::{Error, Result};
use bufdesc::BufDesc;
use wire::{Reader, Writer};

/// Jak dlouho smi prikaz cekat na vstupni ploty a bezet na zarizeni, nez ho
/// ovladac prohlasi za zaseknuty (vystupni ploty skonci `FENCE_FAILED`,
/// ovladac zaloguje duvod `hang`). Windows TDR ma 2 s, Linux (amdgpu, i915)
/// kolem 10 s; 10 s nechava rezervu pro dlouhe vypocty a pomale emulace
/// (lavapipe ve WSL) a porad zaruci, ze nikdo neceka do nekonecna.
pub const HANG_TIMEOUT: Duration = Duration::from_secs(10);

pub mod op {
    pub const INFO: u32 = 1;
    pub const QUERY: u32 = 2;
    pub const CREATE_CONTEXT: u32 = 3;
    pub const CREATE_BUFFER: u32 = 4;
    pub const IMPORT_BUFFER: u32 = 5;
    pub const DESTROY: u32 = 6;
    pub const BIND: u32 = 7;
    pub const SUBMIT: u32 = 8;

    pub const INFO_REPLY: u32 = 101;
    pub const QUERY_REPLY: u32 = 102;
    pub const CONTEXT: u32 = 103;
    pub const BUFFER: u32 = 104;
    pub const DONE: u32 = 105;
    pub const SUBMITTED: u32 = 106;
    pub const ERROR: u32 = 199;
}

/// Trida ovladace (`INFO`).
pub mod class {
    /// virtio-gpu (uAPI = capsety; Venus = Vulkan pres hostitele).
    pub const VIRTIO_GPU: u32 = 1;
}

/// Klice `QUERY`.
pub mod query {
    /// Obsah capsetu: `a` = id capsetu, `b` = verze (virtio-gpu).
    pub const CAPSET: u32 = 1;
}

/// Pamet bufferu (`CREATE_BUFFER`, u virtio-gpu hodnoty `BLOB_MEM_*`).
pub mod mem {
    /// Pamet hostitele vytvorena kontextem (`VkDeviceMemory` nebo nova
    /// sdilena pamet pro odpovedi a ring).
    pub const HOST3D: u32 = 2;
}

/// Priznaky bufferu (hodnoty virtio-gpu `BLOB_FLAG_*`).
pub mod flags {
    /// Namapovat do sdilene pameti a poslat klientovi VMO.
    pub const MAPPABLE: u32 = 1;
    pub const SHAREABLE: u32 = 2;
    pub const CROSS_DEVICE: u32 = 4;
    pub const ALL: u32 = MAPPABLE | SHAREABLE | CROSS_DEVICE;
}

/// Nejvetsi data v jedne zprave `SUBMIT` (zbytek limitu kanalu na hlavicku
/// a hodnoty plotu). Vetsi data patri do bufferu (u Venusu
/// `vkExecuteCommandStreamsMESA`).
pub const MAX_SUBMIT: usize = 60 * 1024;
/// Nejvetsi odpoved `QUERY`.
pub const MAX_QUERY: usize = 4096;
/// Nejvic uAPI v `INFO`.
pub const MAX_UAPIS: usize = 16;
/// Nejvic plotu k cekani a zvlast k signalizaci v jednom `SUBMIT`.
pub const MAX_FENCES: usize = 16;
/// Nejvetsi data `BIND`.
pub const MAX_BIND: usize = 4096;

/// uAPI ovladace (u virtio-gpu capset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Uapi {
    pub id: u32,
    pub version: u32,
    /// Velikost popisu (u capsetu bajty pro `QUERY`).
    pub size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Info,
    Query { key: u32, a: u32, b: u32 },
    CreateContext { uapi: u32 },
    /// `driver_id` = objekt kontextu (u Venusu id `VkDeviceMemory`), 0 = nova
    /// sdilena pamet. `size` po strankach.
    CreateBuffer { mem: u32, flags: u32, driver_id: u64, size: u64 },
    /// Buffer z VMO klienta (handle zpravy, pravo `DMA` a `READ`/`WRITE`
    /// podle pouziti); popis se overi proti velikosti VMO.
    ImportBuffer { desc: BufDesc },
    Destroy { buffer: u32 },
    /// Mapovani do adresniho prostoru GPU (data ovladace).
    Bind { data: Vec<u8> },
    /// Data pro frontu `queue`. Handly zpravy: `wait.len()` plotu
    /// k cekani, pak `signal.len()` plotu k signalizaci; tady jen hodnoty.
    Submit { queue: u32, wait: Vec<u64>, signal: Vec<u64>, data: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// Trida ovladace, uAPI a velikost sdilene pameti pro mapovane buffery
    /// (0 = zadna).
    Info { class: u32, uapis: Vec<Uapi>, shm_size: u64 },
    Query(Vec<u8>),
    Context,
    /// Buffer vytvoren; s `MAPPABLE` jde v handlech zpravy VMO (`size`
    /// bajtu) a `map_info` je typ cache od zarizeni.
    Buffer { buffer: u32, size: u64, map_info: u32 },
    /// Hotovo bez dat (`DESTROY`, `BIND`).
    Done { op: u32 },
    /// Prikaz prijat (dokonceni hlasi ploty).
    Submitted,
    Error { op: u32, code: Error },
}

/// Kod operace zpravy (pro odpoved `Error` na zpravu, ktera nejde
/// rozebrat).
pub fn op_of(data: &[u8]) -> u32 {
    data.get(..4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
}

pub(crate) fn finish(r: Reader) -> Result<()> {
    if r.is_empty() { Ok(()) } else { Err(Error::InvalidArgs) }
}

fn values(r: &mut Reader) -> Result<Vec<u64>> {
    let n = r.u8()? as usize;
    if n > MAX_FENCES {
        return Err(Error::InvalidArgs);
    }
    (0..n).map(|_| r.u64()).collect()
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            Request::Info => w = Writer::op(op::INFO),
            Request::Query { key, a, b } => {
                w = Writer::op(op::QUERY);
                w.u32(*key).u32(*a).u32(*b);
            }
            Request::CreateContext { uapi } => {
                w = Writer::op(op::CREATE_CONTEXT);
                w.u32(*uapi);
            }
            Request::CreateBuffer { mem, flags, driver_id, size } => {
                w = Writer::op(op::CREATE_BUFFER);
                w.u32(*mem).u32(*flags).u64(*driver_id).u64(*size);
            }
            Request::ImportBuffer { desc } => {
                w = Writer::op(op::IMPORT_BUFFER);
                desc.write(&mut w);
            }
            Request::Destroy { buffer } => {
                w = Writer::op(op::DESTROY);
                w.u32(*buffer);
            }
            Request::Bind { data } => {
                w = Writer::op(op::BIND);
                w.bytes(data);
            }
            Request::Submit { queue, wait, signal, data } => {
                w = Writer::op(op::SUBMIT);
                w.u32(*queue).u8(wait.len() as u8);
                for v in wait {
                    w.u64(*v);
                }
                w.u8(signal.len() as u8);
                for v in signal {
                    w.u64(*v);
                }
                w.bytes(data);
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let m = match r.u32()? {
            op::INFO => Request::Info,
            op::QUERY => Request::Query { key: r.u32()?, a: r.u32()?, b: r.u32()? },
            op::CREATE_CONTEXT => Request::CreateContext { uapi: r.u32()? },
            op::CREATE_BUFFER => {
                let mem = r.u32()?;
                let f = r.u32()?;
                if f & !flags::ALL != 0 {
                    return Err(Error::InvalidArgs);
                }
                Request::CreateBuffer { mem, flags: f, driver_id: r.u64()?, size: r.u64()? }
            }
            op::IMPORT_BUFFER => Request::ImportBuffer {
                desc: BufDesc::read(&mut r).map_err(|_| Error::InvalidArgs)?,
            },
            op::DESTROY => Request::Destroy { buffer: r.u32()? },
            op::BIND => {
                let d = r.rest();
                if d.len() > MAX_BIND {
                    return Err(Error::InvalidArgs);
                }
                Request::Bind { data: d.into() }
            }
            op::SUBMIT => {
                let queue = r.u32()?;
                let wait = values(&mut r)?;
                let signal = values(&mut r)?;
                let data = r.rest();
                // Prazdna data jdou: jen cekani a signalizace (synchronizace
                // mezi frontami bez prace).
                if data.len() > MAX_SUBMIT || data.len() % 4 != 0 {
                    return Err(Error::InvalidArgs);
                }
                Request::Submit { queue, wait, signal, data: data.into() }
            }
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
            Reply::Info { class, uapis, shm_size } => {
                w = Writer::op(op::INFO_REPLY);
                w.u32(*class).u64(*shm_size).u32(uapis.len() as u32);
                for c in uapis {
                    w.u32(c.id).u32(c.version).u32(c.size);
                }
            }
            Reply::Query(d) => {
                w = Writer::op(op::QUERY_REPLY);
                w.bytes(d);
            }
            Reply::Context => w = Writer::op(op::CONTEXT),
            Reply::Buffer { buffer, size, map_info } => {
                w = Writer::op(op::BUFFER);
                w.u32(*buffer).u64(*size).u32(*map_info);
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
                let class = r.u32()?;
                let shm_size = r.u64()?;
                let n = r.u32()? as usize;
                if n > MAX_UAPIS {
                    return Err(Error::InvalidArgs);
                }
                let mut uapis = Vec::with_capacity(n);
                for _ in 0..n {
                    uapis.push(Uapi { id: r.u32()?, version: r.u32()?, size: r.u32()? });
                }
                Reply::Info { class, uapis, shm_size }
            }
            op::QUERY_REPLY => {
                let d = r.rest();
                if d.len() > MAX_QUERY {
                    return Err(Error::InvalidArgs);
                }
                Reply::Query(d.into())
            }
            op::CONTEXT => Reply::Context,
            op::BUFFER => Reply::Buffer { buffer: r.u32()?, size: r.u64()?, map_info: r.u32()? },
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
