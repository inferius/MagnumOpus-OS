//! Krok 4: skladani na GPU (`libs/vkcomp`) proti `gfx` na CPU.
//!
//! Snimek 96x64: pozadi (kopie), vrstva s pruhlednosti, zvetsena vrstva
//! (nejblizsi soused) a vrstva castecne mimo snimek s orezem. Vsechny
//! pixely snimku z GPU musi byt stejne jako `gfx::Canvas::draw_image`.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use guiproto::Rect;
use libos::time;
use venus::client::{Error, Venus};
use vkcomp::gpu::Compositor;
use vkcomp::plan::Draw;

const FW: u32 = 96;
const FH: u32 = 64;

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
    /// Premultiplikovany pixel.
    fn pixel(&mut self) -> u32 {
        let a = self.next() & 255;
        a << 24 | (self.next() % (a + 1)) << 16 | (self.next() % (a + 1)) << 8 | self.next() % (a + 1)
    }
}

fn fill(c: &Compositor, id: u32, px: &[u32]) {
    let bytes: Vec<u8> = px.iter().flat_map(|p| p.to_le_bytes()).collect();
    if let Some(s) = c.surface(id) {
        s.map().write_bytes(0, &bytes);
    }
}

/// Slozi snimek na GPU a porovna s CPU. Vraci radek vysledku a spojeni.
pub fn run(v: Venus) -> (String, Option<Venus>) {
    let mut c = match Compositor::new(v, FW, FH) {
        Ok(c) => c,
        Err(e) => return (format!("err init {e:?}"), None),
    };
    let r = test(&mut c);
    let name = c.device_name.clone();
    let v = c.destroy().ok();
    match r {
        Ok(s) => (format!("ok {s} ({name})"), v),
        Err(e) => (format!("err {e:?}"), v),
    }
}

fn test(c: &mut Compositor) -> Result<String, Error> {
    let mut rng = Rng(0x5EED_1234);
    let bg: Vec<u32> = (0..FW * FH).map(|i| 0xFF00_0000 | i.wrapping_mul(2654435761) >> 8 & 0xFF_FFFF).collect();
    let a: Vec<u32> = (0..40 * 30).map(|_| rng.pixel()).collect();
    let b: Vec<u32> = (0..16 * 16).map(|_| rng.pixel()).collect();
    let sbg = c.create_surface(FW, FH)?;
    let sa = c.create_surface(40, 30)?;
    let sb = c.create_surface(16, 16)?;
    fill(c, sbg, &bg);
    fill(c, sa, &a);
    fill(c, sb, &b);
    let full = Rect::new(0, 0, FW, FH);
    let draws = [
        Draw { src: sbg, src_rect: full, dst: full, clip: full, opacity: 255, opaque: true },
        Draw { src: sa, src_rect: Rect::new(0, 0, 40, 30), dst: Rect::new(10, 8, 40, 30), clip: full, opacity: 200, opaque: false },
        Draw { src: sb, src_rect: Rect::new(0, 0, 16, 16), dst: Rect::new(60, 30, 32, 24), clip: full, opacity: 255, opaque: false },
        Draw {
            src: sa,
            src_rect: Rect::new(4, 2, 30, 20),
            dst: Rect::new(-10, 50, 30, 20),
            clip: Rect::new(0, 0, 70, 60),
            opacity: 128,
            opaque: false,
        },
    ];
    let t = time::now();
    let n = c.compose(&draws)?;
    let us = (time::now() - t) / 1000;
    // Stejne na CPU.
    let mut expect = alloc::vec![0u32; (FW * FH) as usize];
    {
        let mut cv = gfx::Canvas::new(&mut expect, FW, FH, FW).ok_or(Error::Os(libos::Error::InvalidArgs))?;
        for d in &draws {
            let (px, w, h): (&[u32], u32, u32) = match d.src {
                x if x == sbg => (&bg, FW, FH),
                x if x == sa => (&a, 40, 30),
                _ => (&b, 16, 16),
            };
            let img = gfx::Image::new(px, w, h, w).ok_or(Error::Os(libos::Error::InvalidArgs))?;
            cv.set_clip(d.clip);
            cv.draw_image(&img, d.src_rect, d.dst, d.opacity, d.opaque);
        }
    }
    let mut got = alloc::vec![0u8; (FW * FH * 4) as usize];
    c.frame().ok_or(Error::Os(libos::Error::BadState))?.read_bytes(0, &mut got);
    let got: Vec<u32> = got.chunks(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    let diff: Vec<usize> = (0..expect.len()).filter(|&i| got[i] != expect[i]).collect();
    for id in [sbg, sa, sb] {
        c.destroy_surface(id)?;
    }
    if let Some(&i) = diff.first() {
        libos::println!(
            "vk-probe: skladani: {} pixelu se lisi, prvni [{},{}] gpu {:#010x} cpu {:#010x}",
            diff.len(),
            i as u32 % FW,
            i as u32 / FW,
            got[i],
            expect[i]
        );
        return Err(Error::Vk(venus::vk::VK_ERROR_UNKNOWN));
    }
    Ok(format!("{FW}x{FH} {n} dispatchu, shoda s gfx, {us} us"))
}
