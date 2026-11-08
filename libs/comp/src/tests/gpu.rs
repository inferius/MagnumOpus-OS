//! GPU backend kompozitoru (`gpu.rs`): seznam operaci provedeny
//! referencnim shaderem (`vkcomp::plan`) dava stejny snimek jako CPU cesta.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use gfx::Canvas;
use guiproto::Rect;
use guiproto::window::*;

use super::{T, W, H};
use crate::gpu::{GpuDraw, GpuSink, Src};
use crate::*;

/// Povrchy a snimek v pameti; operace provadi referencni shader.
struct RefSink {
    surfaces: BTreeMap<Src, (Vec<u32>, u32, u32)>,
    frame: Vec<u32>,
    draws: usize,
}

impl GpuSink for RefSink {
    fn upload(&mut self, src: Src, b: Rect, region: Rect, scratch: &[u32], stride: u32) {
        let s = self.surfaces.entry(src).or_insert_with(|| (vec![0; (b.w * b.h) as usize], b.w, b.h));
        if (s.1, s.2) != (b.w, b.h) {
            *s = (vec![0; (b.w * b.h) as usize], b.w, b.h);
        }
        for y in region.y..region.y + region.h as i32 {
            for x in region.x..region.x + region.w as i32 {
                s.0[((y - b.y) as u32 * b.w + (x - b.x) as u32) as usize] = scratch[(y as u32 * stride + x as u32) as usize];
            }
        }
    }

    fn buffer(&mut self, client: ClientId, id: u32, px: &[u32], w: u32, h: u32, stride: u32) {
        let mut v = vec![0; (w * h) as usize];
        for y in 0..h {
            v[(y * w) as usize..((y + 1) * w) as usize].copy_from_slice(&px[(y * stride) as usize..(y * stride + w) as usize]);
        }
        self.surfaces.insert(Src::Buffer(client, id), (v, w, h));
    }

    fn draw(&mut self, d: GpuDraw) {
        if let Src::Color(c) = d.src {
            self.surfaces.insert(d.src, (vec![c], 1, 1));
        }
        let Some((px, w, h)) = self.surfaces.get(&d.src) else { panic!("povrch {:?} nebyl poslan", d.src) };
        let v = vkcomp::plan::Draw { src: 0, src_rect: d.src_rect, dst: d.dst, clip: d.clip, opacity: d.opacity, opaque: d.opaque };
        if let Some(p) = vkcomp::plan::plan(&v, W, H, *w, *h, *w) {
            vkcomp::plan::run_reference(&p.push, &mut self.frame, px);
            self.draws += 1;
        }
    }
}

fn gpu_frame(t: &mut T, sink: &mut RefSink, scratch: &mut Vec<u32>) -> Option<Vec<Rect>> {
    t.now += PERIOD;
    let mut canvas = Canvas::new(scratch, W, H, W).unwrap();
    let r = t.c.tick_gpu(t.now, &mut canvas, sink);
    if r.is_some() {
        t.c.presented(t.now);
    }
    r
}

/// Scena: okno s titulkem a barvou, okno s vlastnim titulkem a buffer
/// vrstvou s pruhlednosti (vnorena skupina s orezem a pruhlednosti),
/// okno bez ramu, panel.
fn scene(t: &mut T) {
    let a = t.client("org.magnumopus.a");
    t.window(a, Mode::Standard, 0xFF2A_3340);
    let b = t.client("org.magnumopus.b");
    t.msg(b, ClientMsg::WindowCreate { window: 1, mode: Mode::CustomTitle, w: 180, h: 120, title: "B".into() });
    let mut x: u32 = 0x9E37_79B9;
    let px: Vec<u32> = (0..64 * 48)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let a = x & 255;
            a << 24 | ((x >> 8) % (a + 1)) << 16 | ((x >> 16) % (a + 1)) << 8 | (x >> 24) % (a + 1)
        })
        .collect();
    t.c.client_msg(b, ClientMsg::BufferCreate { buffer: 1, w: 64, h: 48, stride: 64 }, Some(Box::new(px)), t.now);
    t.msg(b, ClientMsg::LayerCreate { layer: 1, window: 1, parent: 0, kind: LayerKind::Group });
    t.msg(
        b,
        ClientMsg::LayerSet {
            layer: 1,
            props: LayerProps { mask: prop::RECT | prop::CLIP | prop::OPACITY, rect: Rect::new(10, 10, 150, 90), clip: true, opacity: 200, ..Default::default() },
        },
    );
    t.msg(b, ClientMsg::LayerCreate { layer: 2, window: 1, parent: 1, kind: LayerKind::Buffer });
    t.msg(
        b,
        ClientMsg::LayerSet {
            layer: 2,
            props: LayerProps { mask: prop::RECT | prop::BUFFER, rect: Rect::new(5, 5, 64, 48), buffer: 1, ..Default::default() },
        },
    );
    // Zvetsena kopie (skalovani nejblizsim sousedem) a polopruhledna barva.
    t.msg(b, ClientMsg::LayerCreate { layer: 3, window: 1, parent: 1, kind: LayerKind::Buffer });
    t.msg(
        b,
        ClientMsg::LayerSet {
            layer: 3,
            props: LayerProps { mask: prop::RECT | prop::BUFFER, rect: Rect::new(60, 20, 100, 70), buffer: 1, ..Default::default() },
        },
    );
    t.msg(b, ClientMsg::LayerCreate { layer: 4, window: 1, parent: 0, kind: LayerKind::Solid });
    t.msg(
        b,
        ClientMsg::LayerSet {
            layer: 4,
            props: LayerProps { mask: prop::RECT | prop::COLOR, rect: Rect::new(0, 80, 120, 30), color: gfx::rgba(40, 200, 90, 120), ..Default::default() },
        },
    );
    t.commit(b, 1, 0, 0, 0);
    let c = t.client("org.magnumopus.c");
    t.window(c, Mode::Frameless, 0xFF60_2030);
}

#[test]
fn gpu_list_matches_cpu_frame() {
    let mut cpu = T::new();
    let mut gpu = T::new();
    scene(&mut cpu);
    scene(&mut gpu);
    let mut sink = RefSink { surfaces: BTreeMap::new(), frame: vec![0; (W * H) as usize], draws: 0 };
    let mut scratch = vec![0u32; (W * H) as usize];
    let r1 = cpu.frame();
    let r2 = gpu_frame(&mut gpu, &mut sink, &mut scratch);
    assert_eq!(r1, r2, "stejne poskozene oblasti");
    // Displej alfu snimku ignoruje (XRGB); porovnava se RGB.
    let diffs: Vec<usize> = (0..cpu.frame.len()).filter(|&i| (cpu.frame[i] ^ sink.frame[i]) & 0x00FF_FFFF != 0).collect();
    if !diffs.is_empty() {
        let (mut x0, mut y0, mut x1, mut y1) = (W, H, 0, 0);
        for &i in &diffs {
            let (x, y) = (i as u32 % W, i as u32 / W);
            x0 = x0.min(x); y0 = y0.min(y); x1 = x1.max(x); y1 = y1.max(y);
        }
        let i = diffs[0];
        std::println!("rozdil v [{x0},{y0}]..[{x1},{y1}], prvni [{},{}] cpu {:#010x} gpu {:#010x}", i as u32 % W, i as u32 / W, cpu.frame[i], sink.frame[i]);
        for (k, w) in &cpu.c.windows { std::println!("okno {k}: outer {:?} rect {:?} mode {:?}", w.outer(), w.rect, w.mode); }
    }
    let diff = diffs.len();
    assert_eq!(diff, 0, "prvni snimek: {diff} pixelu se lisi");
    assert!(sink.draws > 6, "malo operaci: {}", sink.draws);
    // Presun okna (cela obrazovka znovu), znovu porovnat.
    for t in [&mut cpu, &mut gpu] {
        let g = t.c.clients[&1].windows[&1];
        let w = t.c.windows.get_mut(&g).unwrap();
        w.rect = Rect::new(w.rect.x + 37, w.rect.y + 21, w.rect.w, w.rect.h);
        t.c.damage_all();
    }
    cpu.frame();
    gpu_frame(&mut gpu, &mut sink, &mut scratch);
    let diff = (0..cpu.frame.len()).filter(|&i| (cpu.frame[i] ^ sink.frame[i]) & 0x00FF_FFFF != 0).count();
    assert_eq!(diff, 0, "po presunu: {diff} pixelu se lisi");
}
