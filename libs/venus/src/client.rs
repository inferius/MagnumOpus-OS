//! Spojeni s rendererem Venus pres render uzel ovladace GPU (`gpuproto`).
//!
//! Prubeh [`Venus::connect`]:
//! 1. `INFO` - zarizeni musi mit capset Venus (4),
//! 2. `GET_CAPSET` - stejny format proudu ([`capset::WIRE_FORMAT_VERSION`]),
//! 3. `CREATE_CONTEXT` s capsetem Venus,
//! 4. `CREATE_BLOB` - sdilena pamet pro odpovedi (`blob_id` 0,
//!    `MAPPABLE`); ovladac ji namapuje do BARu a posle VMO,
//! 5. `vkSetReplyCommandStreamMESA` - renderer bude odpovedi psat sem.
//!
//! [`Venus::call`] posle proud `vkSeekReplyCommandStreamMESA(0)` + prikazy
//! s plotem na ose 0 (osa CPU: plot se splni az po zpracovani proudu)
//! a po odpovedi ovladace precte odpovedi ze sdilene pameti.
//!
//! Sdilenou pamet ovladac posila jako fyzickou VMO s typem cache podle
//! `map_info` zarizeni (QEMU hlasi `CACHED`); po zniceni blobu nebo konci
//! spojeni ji odvola a pristup pres stare mapovani skonci vyjimkou.

use alloc::vec::Vec;
use core::time::Duration;

use gpuproto::{Reply, Request};
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Channel, Handle, Mapping, Vmo, time};

use crate::capset::VenusCapset;
use crate::cs::{self, CsError, Decoder, Encoder};
use crate::vk::{self, VkCommandStreamDescriptionMESA, VkDevice, VkFence, VkResult, cmd};

/// Capset Venusu ve virtio-gpu.
pub const CAPSET_VENUS: u32 = 4;
/// Velikost bloku pro odpovedi.
pub const REPLY_SIZE: u64 = 64 * 1024;
/// Jak dlouho cekat na ovladac (u `SUBMIT` s plotem az do zpracovani
/// proudu rendererem; ovladac sam nevyprsi). Po vyprseni je spojeni
/// rozjete (`Error::Broken`).
const TIMEOUT: Duration = Duration::from_secs(40);
/// Prvni kus odpovedi, ktery se cte ze sdilene pameti (pak 4x vic).
const REPLY_CHUNK: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Chyba systemu nebo ovladace (`NOT_SUPPORTED` = zarizeni nema Venus).
    Os(libos::Error),
    /// Chyba kodovani prikazu nebo cteni odpovedi.
    Cs(CsError),
    /// Renderer ma jiny format proudu nebo neumi blob s id 0.
    Incompatible,
    /// Zarizeni ma Venus, ale malo sdilene pameti (QEMU bez `hostmem=`).
    NoSharedMemory,
    /// Spojeni je rozjete (vyprsel cas cekani na odpoved): dalsi odpovedi
    /// by nepatrily k pozadavkum, spojeni uz nejde pouzit.
    Broken,
    /// Prikaz Vulkanu vratil chybu.
    Vk(VkResult),
}

impl From<libos::Error> for Error {
    fn from(e: libos::Error) -> Self {
        Error::Os(e)
    }
}

impl From<CsError> for Error {
    fn from(e: CsError) -> Self {
        Error::Cs(e)
    }
}

pub type Result<T> = core::result::Result<T, Error>;

/// Blob zdroj v kontextu (u `VkDeviceMemory` jeho pamet hostitele),
/// namapovany do procesu pro cteni a zapis.
pub struct Blob {
    pub resource: u32,
    pub size: u64,
    /// Typ cache od zarizeni (`MAP_CACHE_*`).
    pub map_info: u32,
    pub map: Mapping,
    _vmo: Vmo,
}

pub struct Venus {
    ch: Channel,
    pub capset: VenusCapset,
    /// Velikost sdilene pameti zarizeni.
    pub shm_size: u64,
    reply_resource: u32,
    reply: Mapping,
    _reply_vmo: Vmo,
    next_id: u64,
    broken: bool,
}

impl Venus {
    /// Pripoji se k render uzlu `ch` a pripravi kontext Venus.
    pub fn connect(ch: Channel) -> Result<Self> {
        let (info, _) = request(&ch, &Request::Info)?;
        let Reply::Info { capsets, shm_size } = info else {
            return Err(Error::Os(libos::Error::Corrupted));
        };
        if !capsets.iter().any(|c| c.id == CAPSET_VENUS) {
            return Err(Error::Os(libos::Error::NotSupported));
        }
        if shm_size < REPLY_SIZE {
            return Err(Error::NoSharedMemory);
        }
        let (data, _) = request(&ch, &Request::GetCapset { id: CAPSET_VENUS, version: 0 })?;
        let Reply::Capset(data) = data else {
            return Err(Error::Os(libos::Error::Corrupted));
        };
        let capset = VenusCapset::parse(&data).ok_or(Error::Incompatible)?;
        if !capset.compatible() {
            return Err(Error::Incompatible);
        }
        expect(request(&ch, &Request::CreateContext { capset: CAPSET_VENUS })?.0, |r| matches!(r, Reply::Context))?;
        let (blob, mut handles) = request(
            &ch,
            &Request::CreateBlob {
                mem: gpuproto::mem::HOST3D,
                flags: gpuproto::flags::MAPPABLE,
                blob_id: 0,
                size: REPLY_SIZE,
            },
        )?;
        let Reply::Blob { resource, size, .. } = blob else {
            return Err(Error::Os(libos::Error::Corrupted));
        };
        if handles.len() != 1 || size < REPLY_SIZE {
            return Err(Error::Os(libos::Error::Corrupted));
        }
        let vmo = Vmo::from_handle(handles.remove(0));
        let reply = Mapping::new(&vmo, 0, REPLY_SIZE, MAP_READ)?;
        let mut v = Self { ch, capset, shm_size, reply_resource: resource, reply, _reply_vmo: vmo, next_id: 1, broken: false };
        let mut e = Encoder::new();
        let stream = VkCommandStreamDescriptionMESA { resource_id: resource, offset: 0, size: REPLY_SIZE };
        cmd::set_reply_command_stream_mesa(&mut e, 0, &stream);
        v.submit(e.finish()?, true)?;
        Ok(v)
    }

    /// Nove id objektu Vulkanu (handle prideluje klient).
    pub fn new_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Id bloku s odpovedmi.
    pub fn reply_resource(&self) -> u32 {
        self.reply_resource
    }

    /// Posle proud prikazu; s `fence` pocka, az ho renderer zpracuje.
    pub fn submit(&mut self, stream: Vec<u8>, fence: bool) -> Result<()> {
        if stream.len() > gpuproto::MAX_SUBMIT {
            return Err(Error::Os(libos::Error::InvalidArgs));
        }
        if self.broken {
            return Err(Error::Broken);
        }
        let r = request(&self.ch, &Request::Submit { ring_idx: 0, fence, data: stream });
        if matches!(r, Err(Error::Os(libos::Error::TimedOut))) {
            self.broken = true;
        }
        expect(r?.0, |r| matches!(r, Reply::Submitted))
    }

    /// Zakoduje prikazy (`encode`, s priznakem [`crate::REPLY`]), pocka na
    /// jejich zpracovani a precte odpovedi (`decode`, ve stejnem poradi).
    ///
    /// Odpovedi se ze sdilene pameti ctou po kusech (4 KiB, pak 4x vic):
    /// kdyz dekoder narazi na konec dat, zkusi se znovu s vetsim kusem.
    pub fn call<T>(
        &mut self,
        encode: impl FnOnce(&mut Encoder),
        decode: impl Fn(&mut Decoder) -> cs::Result<T>,
    ) -> Result<T> {
        let mut e = Encoder::new();
        cmd::seek_reply_command_stream_mesa(&mut e, 0, 0);
        encode(&mut e);
        self.submit(e.finish()?, true)?;
        let mut n = REPLY_CHUNK;
        let mut buf = Vec::new();
        loop {
            buf.resize(n, 0);
            self.reply.read_bytes(0, &mut buf);
            match decode(&mut Decoder::new(&buf)) {
                Err(CsError::Truncated) if n < REPLY_SIZE as usize => n = (n * 4).min(REPLY_SIZE as usize),
                r => return Ok(r?),
            }
        }
    }

    /// Blob z objektu kontextu `blob_id` (u Venusu id `VkDeviceMemory`
    /// s typem pameti HOST_VISIBLE), namapovany do procesu. Velikost po
    /// strankach, nejvys velikost alokace.
    pub fn create_blob(&mut self, blob_id: u64, size: u64) -> Result<Blob> {
        let (r, mut handles) = request(
            &self.ch,
            &Request::CreateBlob { mem: gpuproto::mem::HOST3D, flags: gpuproto::flags::MAPPABLE, blob_id, size },
        )?;
        let Reply::Blob { resource, size, map_info } = r else {
            return Err(Error::Os(libos::Error::Corrupted));
        };
        if handles.len() != 1 {
            return Err(Error::Os(libos::Error::Corrupted));
        }
        let vmo = Vmo::from_handle(handles.remove(0));
        let map = Mapping::new(&vmo, 0, size, MAP_READ | MAP_WRITE)?;
        Ok(Blob { resource, size, map_info, map, _vmo: vmo })
    }

    /// Znici blob (ovladac odvola jeho mapovani).
    pub fn destroy_blob(&mut self, b: Blob) -> Result<()> {
        let resource = b.resource;
        drop(b);
        let (r, _) = request(&self.ch, &Request::DestroyBlob { resource })?;
        expect(r, |r| matches!(r, Reply::Done { .. }))
    }

    /// Pocka na plot (`vkGetFenceStatus` po kratkych pauzach; renderer
    /// blokujici cekani nepovoluje, jen `vkWaitForFences` s casem).
    pub fn wait_fence(&mut self, device: VkDevice, fence: VkFence, timeout: Duration) -> Result<()> {
        let deadline = time::deadline_after(timeout);
        loop {
            let r = self.call(|e| cmd::get_fence_status(e, crate::REPLY, device, fence), cmd::decode_get_fence_status_reply)?;
            match r.ret {
                vk::VK_SUCCESS => return Ok(()),
                vk::VK_NOT_READY if time::now() < deadline => time::sleep(Duration::from_millis(1)),
                vk::VK_NOT_READY => return Err(Error::Vk(vk::VK_TIMEOUT)),
                e => return Err(Error::Vk(e)),
            }
        }
    }

    /// Prikazy bez odpovedi (napr. vkDestroy*); pocka na zpracovani.
    pub fn send(&mut self, encode: impl FnOnce(&mut Encoder)) -> Result<()> {
        let mut e = Encoder::new();
        encode(&mut e);
        self.submit(e.finish()?, true)
    }
}

/// `VK_SUCCESS` nebo chyba.
pub fn check(r: VkResult) -> Result<()> {
    if r == vk::VK_SUCCESS { Ok(()) } else { Err(Error::Vk(r)) }
}

fn request(ch: &Channel, r: &Request) -> Result<(Reply, Vec<Handle>)> {
    ch.write(&r.encode(), Vec::new())?;
    let m = ch.read(Some(time::deadline_after(TIMEOUT)))?;
    let reply = Reply::decode(&m.data)?;
    if let Reply::Error { code, .. } = reply {
        return Err(Error::Os(code));
    }
    Ok((reply, m.handles))
}

fn expect(r: Reply, ok: impl Fn(&Reply) -> bool) -> Result<()> {
    if ok(&r) { Ok(()) } else { Err(Error::Os(libos::Error::Corrupted)) }
}
