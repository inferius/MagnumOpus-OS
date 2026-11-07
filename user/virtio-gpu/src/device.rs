//! Zarizeni virtio-gpu: fronta prikazu, vystupy a jejich zdroje 2D.

use alloc::vec::Vec;
use core::time::Duration;

use guiproto::Rect;
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::hw::Resource;
use libos::{Error, Mapping, Result, Vmo};
use pci::{Address, Ecam};
use vgpu::cmd;
use virtio::{Buffer, F_VERSION_1};

/// Vlastnost: DMA pres IOMMU (musi se prijmout, kdyz ji zarizeni nabidne).
const F_ACCESS_PLATFORM: u64 = 1 << 33;
/// Jak dlouho cekat na odpoved zarizeni.
const TIMEOUT: Duration = Duration::from_secs(2);
/// Stranka pro prikaz (prvni pulka) a odpoved (druha pulka).
const CMD_PAGE: u64 = 4096;
const RESP_OFF: u64 = 2048;
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
    device: virtio::Device,
    queue: virtio::Virtqueue,
    irq: libos::hw::Interrupt,
    _msix: pci::msix::MsixTable,
    resource: Resource,
    _cmd_vmo: Vmo,
    cmd: Mapping,
    cmd_phys: u64,
    outputs: Vec<Out>,
    scanouts: Vec<Scanout>,
    next_resource: u32,
    pub virgl: bool,
    /// Zarizeni neodpovedelo nebo vratilo neplatnou frontu: dal ho
    /// nepouzivat.
    pub broken: bool,
}

impl Gpu {
    pub fn open(ecam: &Ecam, a: Address, resource: Resource) -> Result<Self> {
        let s = virtio::start(ecam, a, &resource, F_VERSION_1 | F_ACCESS_PLATFORM | vgpu::F_VIRGL, 16)?;
        let cmd_vmo = resource.dma_memory(CMD_PAGE)?;
        let cmd_phys = resource.phys_addr(&cmd_vmo, 0)?;
        let cmd = Mapping::new(&cmd_vmo, 0, CMD_PAGE, MAP_READ | MAP_WRITE)?;
        let num_scanouts: u32 = s.device.config(vgpu::config::NUM_SCANOUTS);
        let mut gpu = Self {
            virgl: s.features & vgpu::F_VIRGL != 0,
            device: s.device,
            queue: s.queue,
            irq: s.irq,
            _msix: s.msix,
            resource,
            _cmd_vmo: cmd_vmo,
            cmd,
            cmd_phys,
            outputs: Vec::new(),
            scanouts: Vec::new(),
            next_resource: 1,
            broken: false,
        };
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
        self.device.reset()
    }

    /// Posle prikaz a pocka na odpoved (`resp_len` bajtu). Chyba fronty
    /// nebo vyprseni = zarizeni je vadne (`broken`).
    fn command(&mut self, c: &[u8], resp_len: usize) -> Result<Vec<u8>> {
        if self.broken {
            return Err(Error::IoError);
        }
        if c.len() > RESP_OFF as usize || resp_len > (CMD_PAGE - RESP_OFF) as usize {
            return Err(Error::InvalidArgs);
        }
        self.cmd.write_bytes(0, c);
        self.cmd.write_bytes(RESP_OFF, &[0u8; vgpu::HEADER_SIZE]);
        self.queue.submit(&[
            Buffer {
                phys: self.cmd_phys,
                len: c.len() as u32,
                device_writes: false,
            },
            Buffer {
                phys: self.cmd_phys + RESP_OFF,
                len: resp_len as u32,
                device_writes: true,
            },
        ])?;
        self.device.notify(&self.queue);
        let deadline = libos::time::deadline_after(TIMEOUT);
        loop {
            match self.queue.take_used() {
                Ok(Some(_)) => break,
                Ok(None) => {}
                Err(e) => {
                    self.broken = true;
                    return Err(e);
                }
            }
            if let Err(e) = self.irq.wait(Some(deadline)) {
                self.broken = true;
                return Err(e);
            }
        }
        let mut resp = alloc::vec![0u8; resp_len];
        self.cmd.read_bytes(RESP_OFF, &mut resp);
        Ok(resp)
    }

    /// Prikaz bez dat v odpovedi.
    fn simple(&mut self, c: &[u8]) -> Result<()> {
        let r = self.command(c, vgpu::HEADER_SIZE)?;
        match vgpu::response_type(&r) {
            Some(cmd::RESP_OK_NODATA) => Ok(()),
            Some(cmd::RESP_ERR_OUT_OF_MEMORY) => Err(Error::NoMemory),
            Some(cmd::RESP_ERR_INVALID_PARAMETER | cmd::RESP_ERR_INVALID_SCANOUT_ID) => Err(Error::InvalidArgs),
            _ => Err(Error::IoError),
        }
    }

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
        let res = self.next_resource;
        self.next_resource += 1;
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
