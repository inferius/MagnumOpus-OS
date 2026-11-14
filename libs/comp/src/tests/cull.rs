//! Vynechani zakrytych oblasti pri skladani na CPU: stejne pixely jako
//! kresleni vseho odspodu nahoru.

use alloc::vec::Vec;

use super::*;
use crate::render::subtract;

/// Snimek cele obrazovky s vynechanim a bez nej.
fn both(t: &mut T) -> (Vec<u32>, Vec<u32>) {
    t.c.prof.no_cull = false;
    t.c.redraw_all();
    t.frame();
    let a = t.frame.clone();
    t.c.prof.no_cull = true;
    t.c.redraw_all();
    t.frame();
    let b = t.frame.clone();
    t.c.prof.no_cull = false;
    (a, b)
}

#[test]
fn culling_keeps_pixels() {
    for mode in [theme::Mode::Light, theme::Mode::Dark, theme::Mode::Contrast] {
        let mut t = T::new();
        t.c.set_mode(mode);
        let a = t.client("a");
        t.window(a, Mode::Standard, RED);
        let b = t.client("b");
        t.window(b, Mode::Frameless, 0x8000_4000);
        let c = t.client("c");
        t.window(c, Mode::CustomTitle, 0xFF00_00FF);
        let d = t.client("d");
        t.window(d, Mode::Standard, 0x4020_2020);
        t.frame();
        // Okna se prekryvaji (jinak by test nic neoveril).
        let (ra, rd) = (t.rect_of(a), t.rect_of(d));
        assert!(!ra.intersect(&rd).is_empty(), "{ra:?} {rd:?}");
        let (x, y) = both(&mut t);
        let bad = x.iter().zip(&y).filter(|(p, q)| p != q).count();
        assert_eq!(bad, 0, "{mode:?}: lisi se {bad} pixelu");
    }
}

#[test]
fn subtract_covers_rest() {
    let r = Rect::new(10, 20, 100, 50);
    for k in [Rect::new(0, 0, 5, 5), Rect::new(30, 30, 10, 10), Rect::new(0, 0, 500, 500), Rect::new(50, 10, 200, 20), Rect::new(10, 20, 100, 1)] {
        let parts = subtract(r, k);
        for y in 0..100 {
            for x in 0..150 {
                let n = parts.iter().filter(|p| p.contains(x, y)).count();
                let want = r.contains(x, y) && !k.contains(x, y);
                assert_eq!(n, want as usize, "{k:?} ({x},{y})");
            }
        }
    }
}
