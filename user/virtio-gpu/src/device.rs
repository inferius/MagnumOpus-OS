//! Zarizeni virtio-gpu: fronta prikazu, vystupy a jejich zdroje 2D;
//! kontexty 3D, bloby a sdilena pamet pro render uzel (`render.rs`).

use alloc::vec::Vec;
use core::time::Duration;

use guiproto::Rect;
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::hw::Resource;
use libos::{Error, Mapping, Result, Vmo};
use pci::{Address, Ecam};
use vgpu::cmd;
use virtio::F_VERSION_1;

use crate::cmdq::{CmdQueue, Completion};

/// Vlastnost: DMA pres IOMMU (musi se prijmout, kdyz ji zarizeni nabidne).
const F_ACCESS_PLATFORM: u64 = 1 << 33;
/// Jak dlouho cekat na odpoved zarizeni.
const TIMEOUT: Duration = Duration::from_secs(2);
/// Velikost fronty prikazu (2 deskriptory na slot).
const QUEUE_SIZE: u16 = 64;
/// Zarovnani blobu ve sdilene pameti (stranka hostitele muze byt 64 KiB).
const SHM_ALIGN: u64 = 64 * 1024;
/// Nejvetsi kus souvisle DMA pameti (limit jadra).
const CHUNK_MAX: u64 = 4 << 20;
/// Nejvetsi buffer vystupu (16384 x 1024 x 4).
const MAX_BUFFER: u64 = 64 << 20;

/// Vystup podle `GET_DISPLAY_INFO`.
#[derive(Debug, Clone, Copy)]
pub struct Out {
    pub id: u32,
    pub w: u32,
    pub h: u32,
}

/// Kus pameti zdroje (cele radky).
struct Chunk {
    _vmo: Vmo,
    map: Mapping,
    first_row: u32,
    rows: u32,
}

/// Zobrazovany zdroj jednoho vystupu.
struct Scanout {
    output: u32,
    resource: u32,
    w: u32,
    h: u32,
    /// Krok radku zdroje kompozitoru.
    stride: u32,
    /// VMO kompozitoru (jen cteni).
    src: Mapping,
    _src_vmo: Vmo,
    chunks: Vec<Chunk>,
}

pub struct Gpu {
    /// Fronta prikazu (zarizeni, preruseni, sloty).
    pub q: CmdQueue,
    _msix: pci::msix::MsixTable,
    resource: Resource,
    outputs: Vec<Out>,
    scanouts: Vec<Scanout>,
    next_resource: u32,
    pub virgl: bool,
    /// Capsety (id, nejvyssi verze, velikost), napr. venus = 4.
    pub capsets: Vec<(u32, u32, u32)>,
    pub blob: bool,
    pub context_init: bool,
    /// Oblast sdilene pameti pro mapovane bloby (QEMU `hostmem=`).
    pub shm: Option<virtio::SharedMemory>,
    pub shm_alloc: Option<vgpu::shm::RangeAlloc>,
    next_ctx: u32,
    next_fence: u64,
}

impl Gpu {
    pub fn open(ecam: &Ecam, a: Address, resource: Resource) -> Result<Self> {
        let wanted = F_VERSION_1 | F_ACCESS_PLATFORM | vgpu::F_VIRGL | vgpu::F_RESOURCE_BLOB | vgpu::F_CONTEXT_INIT;
        let s = virtio::start(ecam, a, &resource, wanted, QUEUE_SIZE)?;
        let num_scanouts: u32 = s.device.config(vgpu::config::NUM_SCANOUTS);
        let shm = s.device.shared_memory(vgpu::blob::SHM_ID_HOST_VISIBLE);
        let mut gpu = Self {
            virgl: s.features & vgpu::F_VIRGL != 0,
            capsets: Vec::new(),
            blob: s.features & vgpu::F_RESOURCE_BLOB != 0,
            context_init: s.features & vgpu::F_CONTEXT_INIT != 0,
            shm,
            shm_alloc: shm.map(|m| vgpu::shm::RangeAlloc::new(m.len, SHM_ALIGN)),
            next_ctx: 1,
            next_fence: 1,
            q: CmdQueue::new(s.device, s.queue, s.irq, &resource)?,
            _msix: s.msix,
            resource,
            outputs: Vec::new(),
            scanouts: Vec::new(),
            next_resource: 1,
        };
        let num_capsets: u32 = gpu.q.device.config(vgpu::config::NUM_CAPSETS);
        for i in 0..num_capsets.min(16) {
            if let Ok(r) = gpu.command(&vgpu::get_capset_info(i), vgpu::HEADER_SIZE + 16)
                && let Some(c) = vgpu::parse_capset_info(&r)
            {
                gpu.capsets.push(c);
            }
        }
        let resp = gpu.command(&vgpu::get_display_info(), vgpu::DISPLAY_INFO_SIZE)?;
        let scanouts = vgpu::parse_display_info(&resp, num_scanouts.max(1)).ok_or(Error::NotSupported)?;
        for (i, so) in scanouts.iter().enumerate() {
            if so.enabled && so.rect.w > 0 && so.rect.h > 0 {
                gpu.outputs.push(Out {
                    id: i as u32,
                    w: so.rect.w.min(guiproto::MAX_DIM),
                    h: so.rect.h.min(guiproto::MAX_DIM),
                });
            }
        }
        Ok(gpu)
    }

    pub fn outputs(&self) -> &[Out] {
        &self.outputs
    }

    pub fn reset(&self) -> Result<()> {
        self.q.device.reset()
    }

    /// Zarizeni neodpovedelo nebo vratilo neplatnou frontu: dal ho
    /// nepouzivat.
    pub fn broken(&self) -> bool {
        self.q.broken
    }

    /// Posle prikaz a pocka na odpoved (`resp_len` bajtu). Chyba fronty
    /// nebo vyprseni = zarizeni je vadne (`broken`).
    fn command(&mut self, c: &[u8], resp_len: usize) -> Result<Vec<u8>> {
        self.q.command(c, resp_len, TIMEOUT)
    }

    /// Prikaz bez dat v odpovedi.
    fn simple(&mut self, c: &[u8]) -> Result<()> {
        let r = self.command(c, vgpu::HEADER_SIZE)?;
        resp_result(&r)
    }

    /// Prikaz bez dat v odpovedi (pro render uzel).
    pub fn simple_cmd(&mut self, c: &[u8]) -> Result<()> {
        self.simple(c)
    }

    pub fn new_resource(&mut self) -> u32 {
        let r = self.next_resource;
        self.next_resource += 1;
        r
    }

    /// Obsah capsetu (`size` bajtu podle `GET_CAPSET_INFO`).
    pub fn get_capset(&mut self, id: u32, version: u32, size: u32) -> Result<Vec<u8>> {
        let r = self.command(&vgpu::blob::get_capset(id, version), vgpu::HEADER_SIZE + size as usize)?;
        match vgpu::blob::parse_capset(&r) {
            Some(d) => Ok(d.into()),
            None => resp_result(&r).and(Err(Error::IoError)),
        }
    }

    /// Novy kontext s capsetem; vraci id kontextu.
    pub fn create_context(&mut self, capset: u32) -> Result<u32> {
        let ctx = self.next_ctx;
        self.next_ctx += 1;
        self.simple(&vgpu::blob::ctx_create_init(ctx, capset, "render"))?;
        Ok(ctx)
    }

    pub fn destroy_context(&mut self, ctx: u32) {
        let _ = self.simple(&vgpu::blob::ctx_destroy(ctx));
    }

    /// Namapuje blob na offset sdilene pameti; vraci `map_info`.
    pub fn map_blob(&mut self, ctx: u32, resource: u32, offset: u64) -> Result<u32> {
        let r = self.command(&vgpu::blob::resource_map_blob(ctx, resource, offset), vgpu::blob::MAP_INFO_SIZE)?;
        match vgpu::blob::parse_map_info(&r) {
            Some(mi) => Ok(mi),
            None => resp_result(&r).and(Err(Error::IoError)),
        }
    }

    /// Fyzicka VMO rozsahu sdilene pameti (pro klienta).
    /// Typ cache podle `map_info` zarizeni (`MAP_CACHE_*`); `NONE` a
    /// nezname = bez cache.
    pub fn shm_vmo(&self, offset: u64, size: u64, map_info: u32) -> Result<Vmo> {
        let shm = self.shm.ok_or(Error::NotSupported)?;
        if offset.checked_add(size).is_none_or(|e| e > shm.len) {
            return Err(Error::InvalidArgs);
        }
        let cache = match map_info & vgpu::blob::MAP_CACHE_MASK {
            vgpu::blob::MAP_CACHE_CACHED => libos::abi::vm::cache::CACHED,
            vgpu::blob::MAP_CACHE_WC => libos::abi::vm::cache::WRITE_COMBINING,
            _ => libos::abi::vm::cache::UNCACHED,
        };
        self.resource.device_memory_cached(shm.phys + offset, size, cache)
    }

    /// Odvola VMO okna sdilene pameti (mapovani zmizi i u klienta).
    pub fn revoke(&self, vmo: &Vmo) -> Result<()> {
        self.resource.revoke(vmo)
    }

    /// `SUBMIT_3D` proudu `data` v kontextu `ctx` bez plotu (odpoved hned
    /// po prevzeti prikazu).
    pub fn submit(&mut self, ctx: u32, data: &[u8]) -> Result<()> {
        self.simple(&vgpu::submit_3d(ctx, data))
    }

    /// `SUBMIT_3D` s plotem na ose `ring_idx`: jen odeslat, dokonceni
    /// (po splneni plotu) prijde jako `Completion` s `tag`.
    pub fn submit_fenced(&mut self, ctx: u32, ring_idx: u8, data: &[u8], tag: u64) -> Result<()> {
        let id = self.next_fence;
        self.next_fence += 1;
        self.q.submit_async(&vgpu::blob::submit_3d_fenced(ctx, ring_idx, id, data), tag)
    }

    /// Dokoncene asynchronni prikazy (po preruseni nebo synchronnim prikazu).
    pub fn completions(&mut self) -> Vec<Completion> {
        let _ = self.q.poll();
        self.q.take_completions()
    }
}

/// Vysledek dokonceneho prikazu podle typu odpovedi.
pub fn completion_result(resp_type: u32) -> Result<()> {
    let mut h = [0u8; vgpu::HEADER_SIZE];
    h[..4].copy_from_slice(&resp_type.to_le_bytes());
    resp_result(&h)
}

/// Odpoved bez dat -> vysledek.
fn resp_result(r: &[u8]) -> Result<()> {
    match vgpu::response_type(r) {
        Some(cmd::RESP_OK_NODATA) => Ok(()),
        Some(cmd::RESP_ERR_OUT_OF_MEMORY) => Err(Error::NoMemory),
        Some(cmd::RESP_ERR_INVALID_PARAMETER | cmd::RESP_ERR_INVALID_SCANOUT_ID) => Err(Error::InvalidArgs),
        Some(cmd::RESP_ERR_INVALID_CONTEXT_ID | cmd::RESP_ERR_INVALID_RESOURCE_ID) => Err(Error::NotFound),
        _ => Err(Error::IoError),
    }
}

impl Gpu {
    /// Pripoji VMO kompozitoru jako obraz vystupu `output`.
    pub fn attach(&mut self, output: u32, w: u32, h: u32, stride: u32, vmo: Vmo) -> Result<()> {
        if !self.outputs.iter().any(|o| o.id == output) {
            return Err(Error::NotFound);
        }
        if w == 0 || h == 0 || w > guiproto::MAX_DIM || h > guiproto::MAX_DIM || stride % 4 != 0 || stride < w * 4 {
            return Err(Error::InvalidArgs);
        }
        let size = stride as u64 * h as u64;
        if size > MAX_BUFFER {
            return Err(Error::InvalidArgs);
        }
        // Mapovani overi, ze je VMO dost velka.
        let src = Mapping::new(&vmo, 0, size, MAP_READ)?;
        // Stejna velikost: jen vymenit zdroj kompozitoru. Kusy DMA se znovu
        // nealokuji (limit DMA pameti plati za dobu zivota ovladace).
        if let Some(s) = self.scanouts.iter_mut().find(|s| s.output == output && s.w == w && s.h == h) {
            s.src = src;
            s._src_vmo = vmo;
            s.stride = stride;
            return self.present(output, &[Rect::new(0, 0, w, h)]);
        }
        let row = w * 4;
        let rpc = vgpu::rows_per_chunk(row, CHUNK_MAX).max(1);
        let page = libos::vm::page_size();
        let mut chunks = Vec::new();
        let mut entries = Vec::new();
        let mut y = 0;
        while y < h {
            let rows = rpc.min(h - y);
            let bytes = rows as u64 * row as u64;
            let cvmo = self.resource.dma_memory(bytes.next_multiple_of(page))?;
            let phys = self.resource.phys_addr(&cvmo, 0)?;
            let map = Mapping::new(&cvmo, 0, bytes, MAP_READ | MAP_WRITE)?;
            entries.push((phys, bytes as u32));
            chunks.push(Chunk {
                _vmo: cvmo,
                map,
                first_row: y,
                rows,
            });
            y += rows;
        }
        let res = self.new_resource();
        self.simple(&vgpu::resource_create_2d(res, vgpu::FORMAT_B8G8R8X8_UNORM, w, h))?;
        if let Err(e) = self
            .simple(&vgpu::resource_attach_backing(res, &entries))
            .and_then(|_| self.simple(&vgpu::set_scanout(output, res, vgpu::Rect { x: 0, y: 0, w, h })))
        {
            let _ = self.simple(&vgpu::resource_unref(res));
            return Err(e);
        }
        // Novy zdroj uz vystup ukazuje: stary (pokud byl) uvolnit az ted,
        // aby pri chybe vyse zustal obraz.
        if let Some(i) = self.scanouts.iter().position(|s| s.output == output) {
            let old = self.scanouts.remove(i);
            let _ = self.simple(&vgpu::resource_detach_backing(old.resource));
            let _ = self.simple(&vgpu::resource_unref(old.resource));
        }
        self.scanouts.push(Scanout {
            output,
            resource: res,
            w,
            h,
            stride,
            src,
            _src_vmo: vmo,
            chunks,
        });
        self.present(output, &[Rect::new(0, 0, w, h)])
    }

    /// Vystup 0 zobrazi blob `resource` (pokus kroku 4 Venusu): `SET_SCANOUT_BLOB`
    /// a `RESOURCE_FLUSH`. Zarizeni odmitne blob bez dmabuf (`IO_ERROR`).
    pub fn scanout_blob(&mut self, resource: u32, w: u32, h: u32, format: u32, stride: u32) -> Result<()> {
        self.simple(&vgpu::blob::set_scanout_blob(0, resource, w, h, format, stride, 0))?;
        self.simple(&vgpu::resource_flush(resource, vgpu::Rect { x: 0, y: 0, w, h }))
    }

    /// Vypne zobrazeni blobu na vystupu 0 a vrati puvodni obraz 2D
    /// (kdyz ho displej mel).
    pub fn restore_scanout(&mut self) -> Result<()> {
        self.simple(&vgpu::blob::set_scanout_blob(0, 0, 16, 16, vgpu::FORMAT_B8G8R8X8_UNORM, 64, 0))?;
        if let Some(s) = self.scanouts.iter().find(|s| s.output == 0) {
            let (res, w, h) = (s.resource, s.w, s.h);
            self.simple(&vgpu::set_scanout(0, res, vgpu::Rect { x: 0, y: 0, w, h }))?;
            return self.present(0, &[Rect::new(0, 0, w, h)]);
        }
        Ok(())
    }

    /// Zkopiruje obdelniky do bufferu DMA a preda je zarizeni.
    pub fn present(&mut self, output: u32, rects: &[Rect]) -> Result<()> {
        let i = self.scanouts.iter().position(|s| s.output == output).ok_or(Error::NotFound)?;
        let (w, h, res) = (self.scanouts[i].w, self.scanouts[i].h, self.scanouts[i].resource);
        let screen = Rect::new(0, 0, w, h);
        let rects = guiproto::display::normalize_rects(rects, screen);
        let mut all = Rect::default();
        for r in &rects {
            self.copy(i, r);
            let vr = vgpu::Rect {
                x: r.x as u32,
                y: r.y as u32,
                w: r.w,
                h: r.h,
            };
            let offset = r.y as u64 * w as u64 * 4 + r.x as u64 * 4;
            self.simple(&vgpu::transfer_to_host_2d(res, vr, offset))?;
            all = all.union(r);
        }
        if !all.is_empty() {
            let vr = vgpu::Rect {
                x: all.x as u32,
                y: all.y as u32,
                w: all.w,
                h: all.h,
            };
            self.simple(&vgpu::resource_flush(res, vr))?;
        }
        Ok(())
    }

    /// Kopie obdelniku (uz orezaneho na vystup) z VMO kompozitoru do kusu DMA.
    fn copy(&self, i: usize, r: &Rect) {
        let s = &self.scanouts[i];
        let row = s.w as u64 * 4;
        let len = r.w as usize * 4;
        for y in r.y as u32..r.y as u32 + r.h {
            let Some(c) = s.chunks.iter().find(|c| y >= c.first_row && y < c.first_row + c.rows) else {
                continue;
            };
            let src = s.src.addr() + y as u64 * s.stride as u64 + r.x as u64 * 4;
            let dst = c.map.addr() + (y - c.first_row) as u64 * row + r.x as u64 * 4;
            // SAFETY: obdelnik je orezany na vystup, takze zdroj lezi v mapovani
            // VMO kompozitoru (`stride * h` bajtu) a cil v kusu DMA (cele radky
            // po `w * 4` bajtech). Obe mapovani ziji s `Scanout`. Kompozitor
            // muze zdroj soucasne menit - cteme surove bajty (bez referenci),
            // nejhorsi vysledek je roztrzeny snimek.
            unsafe { core::ptr::copy_nonoverlapping(src as *const u8, dst as *mut u8, len) };
        }
    }
}
