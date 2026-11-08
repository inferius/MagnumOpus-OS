//! GPU backend kompozitoru: snimek slozi Vulkan pres Venus (`libs/vkcomp`).
//!
//! Zapina se ridicim prikazem `backend gpu` (vychozi je CPU, `backend cpu`
//! vrati CPU). Kompozitor si na kanalu displeje vyzada render uzel
//! (`OPEN_RENDER`), pripoji se k rendereru Venus a od te doby misto
//! kresleni do snimku vytvari seznam operaci (`comp::gpu`). Casti kreslene
//! CPU (plocha, ramy, panel...) a buffery aplikaci se nahravaji do
//! povrchu na GPU, operace slozi vypocetni shader a hotove poskozene
//! obdelniky se zkopiruji z GPU do VMO snimku, ktery dal jde ovladaci
//! displeje (`PRESENT` beze zmeny).
//!
//! RIZIKO (vykon): buffery aplikaci jsou v pameti hosta, kterou GPU
//! nevidi: kazdy pouzity buffer se pri kazdem snimku kopiruje do povrchu
//! a hotovy snimek zpet. S lavapipe (Vulkan na CPU, WSL) je to pomalejsi
//! nez CPU cesta; zrychleni prinese az fyzicka GPU, import pameti hosta
//! (blob GUEST) a zobrazeni bez kopie (`SET_SCANOUT_BLOB`).

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use comp::gpu::{GpuDraw, GpuSink, Src};
use comp::{ClientId, Comp};
use guiproto::Rect;
use libos::{Channel, println};
use venus::client::{Error, Result, Venus};
use vkcomp::gpu::Compositor;
use vkcomp::plan::Draw;

/// Povrch na GPU pro jeden zdroj seznamu.
struct Surf {
    id: u32,
    w: u32,
    h: u32,
    /// Snimek, kdy byl naposledy pouzit (uklid nepouzitych).
    used: u64,
}

/// Prijemce operaci (`comp::gpu::GpuSink`) nad skladacem.
struct Sink {
    c: Compositor,
    surfaces: BTreeMap<Src, Surf>,
    draws: Vec<Draw>,
    frame: u64,
    /// Prvni chyba GPU v tomto snimku.
    err: Option<Error>,
}

pub struct GpuBackend {
    sink: Sink,
    /// Odkladaci platno pro casti kreslene CPU (velikost snimku).
    scratch: Vec<u32>,
    w: u32,
    h: u32,
}

/// Povrchy nepouzite tolik snimku se uvolni (zavrena okna, stare buffery).
const KEEP_FRAMES: u64 = 300;

impl Sink {
    /// Povrch `src` velikosti `w` x `h` (pri jine velikosti novy).
    fn surface(&mut self, src: Src, w: u32, h: u32) -> Option<u32> {
        if let Some(s) = self.surfaces.get_mut(&src) {
            if s.w == w && s.h == h {
                s.used = self.frame;
                return Some(s.id);
            }
            let old = s.id;
            self.surfaces.remove(&src);
            let _ = self.c.destroy_surface(old);
        }
        match self.c.create_surface(w, h) {
            Ok(id) => {
                self.surfaces.insert(src, Surf { id, w, h, used: self.frame });
                Some(id)
            }
            Err(e) => {
                self.err.get_or_insert(e);
                None
            }
        }
    }

    /// Zapise radky `rows` (po `w` pixelech) do povrchu od radku `y0`, sloupce `x0`.
    fn write<'a>(&self, id: u32, x0: u32, y0: u32, w: u32, rows: impl Iterator<Item = &'a [u32]>) {
        let Some(s) = self.c.surface(id) else { return };
        let mut line = Vec::with_capacity(w as usize * 4);
        for (i, row) in rows.enumerate() {
            line.clear();
            line.extend(row.iter().flat_map(|p| p.to_le_bytes()));
            let off = ((y0 + i as u32) as u64 * s.stride as u64 + x0 as u64) * 4;
            s.map().write_bytes(off, &line);
        }
    }

    /// Uvolni povrchy, ktere se dlouho nepouzily.
    fn collect(&mut self) {
        let old: Vec<(Src, u32)> = self
            .surfaces
            .iter()
            .filter(|(_, s)| self.frame.saturating_sub(s.used) > KEEP_FRAMES)
            .map(|(k, s)| (*k, s.id))
            .collect();
        for (k, id) in old {
            self.surfaces.remove(&k);
            let _ = self.c.destroy_surface(id);
        }
    }
}

impl GpuSink for Sink {
    fn upload(&mut self, src: Src, b: Rect, region: Rect, scratch: &[u32], stride: u32) {
        let Some(id) = self.surface(src, b.w, b.h) else { return };
        let rows = (region.y..region.y + region.h as i32).map(|y| {
            let s = (y as u32 * stride + region.x as u32) as usize;
            &scratch[s..s + region.w as usize]
        });
        self.write(id, (region.x - b.x) as u32, (region.y - b.y) as u32, region.w, rows);
    }

    fn buffer(&mut self, client: ClientId, buf: u32, px: &[u32], w: u32, h: u32, stride: u32) {
        let src = Src::Buffer(client, buf);
        // Buffer muze vrstva pouzit vickrat: nahrat jednou za snimek.
        if self.surfaces.get(&src).is_some_and(|s| s.used == self.frame && s.w == w && s.h == h) {
            return;
        }
        let Some(id) = self.surface(src, w, h) else { return };
        let rows = (0..h).map(|y| &px[(y * stride) as usize..(y * stride + w) as usize]);
        self.write(id, 0, 0, w, rows);
    }

    fn draw(&mut self, d: GpuDraw) {
        let id = match d.src {
            Src::Color(c) => {
                let new = !self.surfaces.contains_key(&d.src);
                let id = self.surface(d.src, 1, 1);
                if new && let Some(id) = id {
                    self.write(id, 0, 0, 1, core::iter::once(&[c][..]));
                }
                id
            }
            src => self.surfaces.get(&src).map(|s| s.id),
        };
        let Some(src) = id else { return };
        self.draws.push(Draw { src, src_rect: d.src_rect, dst: d.dst, clip: d.clip, opacity: d.opacity, opaque: d.opaque });
    }
}

impl GpuBackend {
    /// Render uzel z kanalu displeje (odpoved `RENDER` uz prisla) a skladac
    /// `w` x `h`.
    pub fn new(render: Channel, w: u32, h: u32) -> Result<Self> {
        let v = Venus::connect(render)?;
        let c = Compositor::new(v, w, h)?;
        println!("compd: skladani na GPU ({})", c.device_name);
        Ok(Self {
            sink: Sink { c, surfaces: BTreeMap::new(), draws: Vec::new(), frame: 0, err: None },
            scratch: alloc::vec![0; (w * h) as usize],
            w,
            h,
        })
    }

    /// Tik kompozitoru se skladanim na GPU; hotove obdelniky do `screen`
    /// (pixely snimku `w * h`). Chyba GPU = backend dal nepouzivat.
    pub fn tick(&mut self, comp: &mut Comp, now: u64, screen: &mut [u32]) -> Result<Option<Vec<Rect>>> {
        let (w, h) = (self.w, self.h);
        self.sink.frame += 1;
        self.sink.draws.clear();
        let mut canvas = gfx::Canvas::new(&mut self.scratch, w, h, w).ok_or(Error::Os(libos::Error::InvalidArgs))?;
        let Some(rects) = comp.tick_gpu(now, &mut canvas, &mut self.sink) else {
            return Ok(None);
        };
        if let Some(e) = self.sink.err.take() {
            return Err(e);
        }
        let draws = core::mem::take(&mut self.sink.draws);
        self.sink.c.compose(&draws)?;
        self.sink.draws = draws;
        // Hotove obdelniky z GPU do snimku.
        let frame = self.sink.c.frame().ok_or(Error::Os(libos::Error::BadState))?;
        let mut row = Vec::new();
        for r in &rects {
            let r = r.intersect(&Rect::new(0, 0, w, h));
            for y in r.y..r.y + r.h as i32 {
                row.resize(r.w as usize * 4, 0);
                let off = (y as u32 * w + r.x as u32) as usize;
                frame.read_bytes(off as u64 * 4, &mut row);
                for (i, px) in row.chunks(4).enumerate() {
                    screen[off + i] = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
                }
            }
        }
        if self.sink.frame % 60 == 0 {
            self.sink.collect();
        }
        Ok(Some(rects))
    }

    /// Uvolni GPU (pri prepnuti zpet na CPU).
    pub fn destroy(self) {
        let Sink { mut c, surfaces, .. } = self.sink;
        for (_, s) in surfaces {
            let _ = c.destroy_surface(s.id);
        }
        let _ = c.destroy();
    }
}
