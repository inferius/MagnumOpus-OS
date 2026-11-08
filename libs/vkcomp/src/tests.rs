//! Testy planovani operaci na hostiteli (`cargo test -p vkcomp`): vysledek
//! referencniho provedeni shaderu = `gfx::Canvas::draw_image`.

use alloc::vec;

use guiproto::Rect;

use crate::plan::{Draw, plan, run_reference};

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + (self.next() % (hi - lo + 1) as u32) as i32
    }
    fn pixel(&mut self) -> u32 {
        let a = self.next() & 255;
        a << 24 | (self.next() % (a + 1)) << 16 | (self.next() % (a + 1)) << 8 | self.next() % (a + 1)
    }
}

#[test]
fn plan_matches_gfx_draw_image() {
    let (fw, fh) = (64u32, 48u32);
    let (sw, sh, stride) = (40u32, 30u32, 44u32);
    let mut r = Rng(0xC0FF_EE11);
    let mut drawn = 0;
    for _ in 0..3000 {
        let src: alloc::vec::Vec<u32> = (0..stride * sh).map(|_| r.pixel()).collect();
        let base: alloc::vec::Vec<u32> = (0..fw * fh).map(|_| r.pixel()).collect();
        let scaled = r.next() % 3 == 0;
        let src_rect = if scaled {
            Rect::new(r.range(0, 10), r.range(0, 10), r.range(1, 30) as u32, r.range(1, 20) as u32)
        } else {
            Rect::new(r.range(-8, 30), r.range(-8, 20), r.range(1, 40) as u32, r.range(1, 30) as u32)
        };
        let dst = if scaled {
            Rect::new(r.range(-20, 50), r.range(-20, 40), r.range(1, 60) as u32, r.range(1, 50) as u32)
        } else {
            Rect::new(r.range(-20, 50), r.range(-20, 40), src_rect.w, src_rect.h)
        };
        let clip = Rect::new(r.range(-5, 40), r.range(-5, 30), r.range(0, 64) as u32, r.range(0, 48) as u32);
        let opacity = [255u8, 0, 128, r.next() as u8][(r.next() % 4) as usize];
        let opaque = r.next() % 2 == 0;
        let d = Draw { src: 0, src_rect, dst, clip, opacity, opaque };
        // gfx (CPU kompozitoru): u nepruhledneho zdroje opravdu nepruhledne pixely.
        let src_used: alloc::vec::Vec<u32> = if opaque { src.iter().map(|p| p | 0xFF00_0000).collect() } else { src.clone() };
        let mut expect = base.clone();
        {
            let mut c = gfx::Canvas::new(&mut expect, fw, fh, fw).unwrap();
            c.set_clip(clip);
            let img = gfx::Image::new(&src_used, sw, sh, stride).unwrap();
            c.draw_image(&img, src_rect, dst, opacity, opaque);
        }
        let mut got = base.clone();
        match plan(&d, fw, fh, sw, sh, stride) {
            Some(p) => {
                run_reference(&p.push, &mut got, &src_used);
                drawn += 1;
            }
            None => {
                // Bez dispatche se nesmi nic zmenit (az na skalovani s vyrezem
                // mimo obraz, ktere planovac odmita).
                let partial = scaled && src_rect.intersect(&Rect::new(0, 0, sw, sh)) != src_rect;
                if partial {
                    continue;
                }
            }
        }
        assert_eq!(got, expect, "draw {d:?}");
    }
    assert!(drawn > 500, "malo nakreslenych operaci: {drawn}");
}

#[test]
fn groups_cover_clip() {
    let d = Draw {
        src: 0,
        src_rect: Rect::new(0, 0, 20, 17),
        dst: Rect::new(3, 4, 20, 17),
        clip: Rect::new(0, 0, 100, 100),
        opacity: 255,
        opaque: true,
    };
    let p = plan(&d, 100, 100, 20, 17, 20).unwrap();
    assert_eq!(p.groups, (3, 3));
    assert_eq!(p.push.copy, 1);
    let _ = vec![0u8];
}
