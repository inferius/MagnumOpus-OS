//! Render uzel: klienti s kontextem GPU (protokol `gpuproto`), u Venusu
//! Vulkan pres virtio-gpu.
//!
//! Jeden kanal = jeden klient = nejvys jeden kontext. Blob s `MAPPABLE`
//! ovladac namapuje do oblasti sdilene pameti zarizeni (BAR, id
//! `SHM_ID_HOST_VISIBLE`) a klientovi posle fyzickou VMO tohoto okna.
//! Zavreni kanalu znici bloby i kontext.
//!
//! RIZIKO (bezpecnost): VMO okna BARu nejde odvolat. Klient si ji muze
//! nechat i po `DESTROY_BLOB` nebo zavreni kanalu; az ovladac stejny
//! rozsah BARu da jinemu klientovi, puvodni klient uvidi jeho data. Dokud
//! jadro neumi odvolatelne mapovani, smi render uzel dostat jen duveryhodny
//! proces (init, kompozitor a jim povolene aplikace).
//!
//! RIZIKO (odezva): ovladac ceka na odpoved zarizeni synchronne, u
//! `SUBMIT` s plotem az do zpracovani proudu rendererem (nejvys 30 s).
//! Mezitim neobslouzi displej ani jine klienty. Kdyz plot nedobehne,
//! klient dostane `TIMED_OUT` a dalsi prikazy pockaji na jeho odpoved;
//! ovladac nekonci.

use alloc::vec::Vec;

use gpuproto::{Capset, Reply, Request, flags, mem};
use libos::{Channel, Error, Handle, Result};
use vgpu::blob;

use crate::device::Gpu;

/// Nejvetsi blob (zarizeni nema vic sdilene pameti nez par stovek MiB).
const MAX_BLOB: u64 = 256 << 20;

struct Blob {
    resource: u32,
    /// Offset ve sdilene pameti (namapovany blob).
    shm: Option<(u64, u64)>,
}

pub struct Client {
    pub ch: Channel,
    ctx: Option<u32>,
    blobs: Vec<Blob>,
}

impl Client {
    pub fn new(ch: Channel) -> Self {
        Self { ch, ctx: None, blobs: Vec::new() }
    }

    /// Obslouzi zpravu; odpoved a handly k ni.
    pub fn handle(&mut self, gpu: &mut Gpu, data: &[u8]) -> (Reply, Vec<Handle>) {
        let op = gpuproto::op_of(data);
        let r = Request::decode(data).and_then(|req| self.request(gpu, req));
        match r {
            Ok(x) => x,
            Err(code) => (Reply::Error { op, code }, Vec::new()),
        }
    }

    fn request(&mut self, gpu: &mut Gpu, req: Request) -> Result<(Reply, Vec<Handle>)> {
        let none = |r: Reply| Ok((r, Vec::new()));
        match req {
            Request::Info => none(Reply::Info {
                capsets: gpu
                    .capsets
                    .iter()
                    .take(gpuproto::MAX_CAPSETS)
                    .map(|&(id, version, size)| Capset { id, version, size })
                    .collect(),
                shm_size: gpu.shm.map_or(0, |s| s.len),
            }),
            Request::GetCapset { id, version } => {
                let &(_, max, size) = gpu.capsets.iter().find(|c| c.0 == id).ok_or(Error::NotFound)?;
                if version > max || size as usize > gpuproto::MAX_CAPSET {
                    return Err(Error::InvalidArgs);
                }
                none(Reply::Capset(gpu.get_capset(id, version, size)?))
            }
            Request::CreateContext { capset } => {
                if self.ctx.is_some() {
                    return Err(Error::AlreadyExists);
                }
                if !gpu.context_init || !gpu.capsets.iter().any(|c| c.0 == capset) {
                    return Err(Error::NotSupported);
                }
                // Venus bez blobu a sdilene pameti nema kam psat odpovedi.
                if capset == vgpu::CAPSET_VENUS && (!gpu.blob || gpu.shm.is_none()) {
                    return Err(Error::NotSupported);
                }
                self.ctx = Some(gpu.create_context(capset)?);
                none(Reply::Context)
            }
            Request::CreateBlob { mem: m, flags: f, blob_id, size } => {
                let ctx = self.ctx.ok_or(Error::BadState)?;
                if m != mem::HOST3D || !gpu.blob {
                    return Err(Error::NotSupported);
                }
                if size == 0 || size % 4096 != 0 || size > MAX_BLOB {
                    return Err(Error::InvalidArgs);
                }
                self.create_blob(gpu, ctx, f, blob_id, size)
            }
            Request::DestroyBlob { resource } => {
                let ctx = self.ctx.ok_or(Error::BadState)?;
                let i = self.blobs.iter().position(|b| b.resource == resource).ok_or(Error::NotFound)?;
                let b = self.blobs.remove(i);
                destroy_blob(gpu, ctx, &b);
                none(Reply::Done { op: gpuproto::op::DESTROY_BLOB })
            }
            Request::Submit { ring_idx, fence, data } => {
                let ctx = self.ctx.ok_or(Error::BadState)?;
                // Jen osa 0 (CPU): plot na ose, kterou renderer nema, by se
                // nikdy nesplnil. Dalsi osy az s frontami (VkQueue s ring_idx).
                if ring_idx != 0 {
                    return Err(Error::NotSupported);
                }
                gpu.submit(ctx, ring_idx, fence, &data)?;
                none(Reply::Submitted)
            }
        }
    }

    fn create_blob(&mut self, gpu: &mut Gpu, ctx: u32, f: u32, blob_id: u64, size: u64) -> Result<(Reply, Vec<Handle>)> {
        let mappable = f & flags::MAPPABLE != 0;
        let shm = gpu.shm.ok_or(Error::NotSupported);
        let offset = if mappable {
            shm?;
            Some(gpu.shm_alloc.as_mut().and_then(|a| a.alloc(size)).ok_or(Error::NoMemory)?)
        } else {
            None
        };
        let resource = gpu.new_resource();
        let info = blob::BlobInfo { resource, mem: blob::BLOB_MEM_HOST3D, flags: f, blob_id, size };
        if let Err(e) = gpu.simple_cmd(&blob::resource_create_blob(ctx, &info, &[])) {
            if let (Some(o), Some(a)) = (offset, gpu.shm_alloc.as_mut()) {
                a.release(o, size);
            }
            return Err(e);
        }
        let mut b = Blob { resource, shm: None };
        let mut handles = Vec::new();
        let mut map_info = 0;
        if let Some(off) = offset {
            let mapped = gpu.map_blob(ctx, resource, off).and_then(|mi| {
                map_info = mi;
                b.shm = Some((off, size));
                gpu.shm_vmo(off, size)
            });
            match mapped {
                Ok(vmo) => handles.push(vmo.into_handle()),
                Err(e) => {
                    if b.shm.is_none()
                        && let Some(a) = gpu.shm_alloc.as_mut()
                    {
                        a.release(off, size);
                    }
                    destroy_blob(gpu, ctx, &b);
                    return Err(e);
                }
            }
        }
        self.blobs.push(b);
        Ok((Reply::Blob { resource, size, map_info }, handles))
    }

    /// Konec klienta: bloby a kontext pryc.
    pub fn close(&mut self, gpu: &mut Gpu) {
        let Some(ctx) = self.ctx.take() else { return };
        for b in core::mem::take(&mut self.blobs) {
            destroy_blob(gpu, ctx, &b);
        }
        gpu.destroy_context(ctx);
    }
}

fn destroy_blob(gpu: &mut Gpu, ctx: u32, b: &Blob) {
    if let Some((off, size)) = b.shm {
        let _ = gpu.simple_cmd(&blob::resource_unmap_blob(ctx, b.resource));
        if let Some(a) = gpu.shm_alloc.as_mut() {
            a.release(off, size);
        }
    }
    let _ = gpu.simple_cmd(&vgpu::resource_unref(b.resource));
}
