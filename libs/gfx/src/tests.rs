//! Testy kresleni (na hostiteli: `cargo test -p gfx`).

use crate::damage::Damage;
use crate::*;

fn canvas(px: &mut [u32], w: u32, h: u32) -> Canvas<'_> {
    Canvas::new(px, w, h, w).unwrap()
}

#[test]
fn color_math() {
    assert_eq!(rgb(1, 2, 3), 0xFF01_0203);
    assert_eq!(rgba(255, 255, 255, 128), 0x8080_8080);
    assert_eq!(scale(0xFF80_4020, 255), 0xFF80_4020);
    assert_eq!(scale(0xFFFF_FFFF, 0), 0);
    assert_eq!(scale(0xFFFF_FFFF, 128), 0x8080_8080);
    // Nepruhledny zdroj prepise, pruhledny nic nezmeni.
    assert_eq!(over(0xFF00_0000, 0xFF11_2233), 0xFF11_2233);
    assert_eq!(over(0xFF11_2233, 0), 0xFF11_2233);
    // Polovicni bila pres cernou = seda.
    assert_eq!(over(0xFF00_0000, 0x8080_8080), 0xFF80_8080);
    // Vadny vstup (slozka > alfa) nepretece.
    let v = over(0xFFFF_FFFF, 0x10FF_FFFF);
    assert_eq!(v, 0xFEFF_FFFF, "{v:x}");
}

#[test]
fn canvas_rejects_small_memory() {
    let mut px = [0u32; 10];
    assert!(Canvas::new(&mut px, 4, 3, 4).is_none());
    assert!(Canvas::new(&mut px, 4, 2, 3).is_none());
    assert!(Image::new(&px, 2, 5, 2).is_some());
}

#[test]
fn fill_clips_to_canvas_and_clip() {
    let mut px = [0u32; 16];
    let mut c = canvas(&mut px, 4, 4);
    c.fill(Rect::new(-2, -2, 4, 4), 1);
    assert_eq!(c.get(0, 0), 1);
    assert_eq!(c.get(1, 1), 1);
    assert_eq!(c.get(2, 2), 0);
    c.set_clip(Rect::new(3, 3, 10, 10));
    c.fill(Rect::new(0, 0, 100, 100), 2);
    assert_eq!(c.get(3, 3), 2);
    assert_eq!(c.get(2, 3), 0);
    // Obri souradnice nic nerozbiji.
    c.reset_clip();
    c.fill(Rect::new(i32::MAX, i32::MIN, u32::MAX, u32::MAX), 3);
    c.fill(Rect::new(i32::MIN, i32::MIN, u32::MAX, u32::MAX), 4);
    assert_eq!(c.get(0, 0), 4);
}

#[test]
fn blend_and_outline() {
    let mut px = [0xFF00_0000u32; 25];
    let mut c = canvas(&mut px, 5, 5);
    c.blend(Rect::new(0, 0, 5, 5), 0x8080_8080);
    assert_eq!(c.get(2, 2), 0xFF80_8080);
    c.outline(Rect::new(0, 0, 5, 5), 1, rgb(255, 0, 0));
    assert_eq!(c.get(0, 0), rgb(255, 0, 0));
    assert_eq!(c.get(4, 4), rgb(255, 0, 0));
    assert_eq!(c.get(0, 2), rgb(255, 0, 0));
    assert_eq!(c.get(2, 2), 0xFF80_8080);
}

#[test]
fn rounded_corners_are_antialiased() {
    let mut px = [0u32; 400];
    let mut c = canvas(&mut px, 20, 20);
    c.rounded(Rect::new(0, 0, 20, 20), 6, rgb(255, 255, 255));
    assert_eq!(c.get(10, 10), rgb(255, 255, 255));
    assert_eq!(c.get(0, 0), 0, "roh je pruhledny");
    let edge = c.get(1, 1) >> 24;
    assert!(edge > 0 && edge < 255, "okraj rohu je castecne pokryty: {edge}");
    assert_eq!(c.get(0, 10), rgb(255, 255, 255), "rovna hrana je plna");
}

#[test]
fn mask_draws_coverage() {
    let mut px = [0xFF00_0000u32; 9];
    let mut c = canvas(&mut px, 3, 3);
    let mask = [0, 255, 0, 128, 255, 128, 0, 0, 0];
    c.mask(0, 0, 3, 3, &mask, rgb(255, 255, 255));
    assert_eq!(c.get(1, 0), rgb(255, 255, 255));
    assert_eq!(c.get(0, 0), 0xFF00_0000);
    assert_eq!(c.get(0, 1), 0xFF80_8080);
    // Kratka maska se ignoruje.
    c.mask(0, 0, 3, 3, &mask[..4], 0xFFFF_0000);
    assert_eq!(c.get(1, 0), rgb(255, 255, 255));
    // Maska z casti mimo plochu.
    c.mask(-1, -1, 3, 3, &mask, 0xFFFF_0000);
    assert_eq!(c.get(0, 0), 0xFFFF_0000);
}

#[test]
fn draw_image_copy_scale_opacity() {
    let src_px: Vec<u32> = (0..16).map(|i| 0xFF00_0000 | i).collect();
    let src = Image::new(&src_px, 4, 4, 4).unwrap();
    let mut px = [0u32; 64];
    let mut c = canvas(&mut px, 8, 8);
    // Kopie 1:1 s posunem.
    c.draw_image(&src, Rect::new(1, 1, 2, 2), Rect::new(5, 5, 2, 2), 255, true);
    assert_eq!(c.get(5, 5), 0xFF00_0005);
    assert_eq!(c.get(6, 6), 0xFF00_000A);
    // Zvetseni 2x.
    c.draw_image(&src, Rect::new(0, 0, 2, 2), Rect::new(0, 0, 4, 4), 255, true);
    assert_eq!(c.get(0, 0), 0xFF00_0000);
    assert_eq!(c.get(1, 1), 0xFF00_0000);
    assert_eq!(c.get(2, 0), 0xFF00_0001);
    assert_eq!(c.get(3, 3), 0xFF00_0005);
    // Pruhlednost 0 nic, polovina prolne.
    let white = [0xFFFF_FFFFu32; 4];
    let wimg = Image::new(&white, 2, 2, 2).unwrap();
    c.draw_image(&wimg, Rect::new(0, 0, 2, 2), Rect::new(0, 0, 2, 2), 0, true);
    assert_eq!(c.get(0, 0), 0xFF00_0000);
    c.draw_image(&wimg, Rect::new(0, 0, 2, 2), Rect::new(0, 0, 2, 2), 128, true);
    assert_eq!(c.get(0, 0), 0xFF80_8080);
    // Zdrojovy vyrez mimo obraz a cil mimo plochu nic nerozbiji.
    c.draw_image(&src, Rect::new(3, 3, 100, 100), Rect::new(-50, -50, 100, 100), 255, false);
    c.draw_image(&src, Rect::new(10, 10, 2, 2), Rect::new(0, 0, 2, 2), 255, false);
}

#[test]
fn damage_merges_and_limits() {
    let b = Rect::new(0, 0, 1000, 1000);
    let mut d = Damage::new();
    d.add(Rect::new(0, 0, 10, 10), b);
    d.add(Rect::new(5, 5, 10, 10), b);
    assert_eq!(d.rects(), &[Rect::new(0, 0, 15, 15)]);
    d.add(Rect::new(500, 500, 10, 10), b);
    assert_eq!(d.rects().len(), 2);
    d.add(Rect::new(-5, -5, 2, 2), b);
    assert_eq!(d.rects().len(), 2, "mimo hranice se ignoruje");
    for i in 0..40 {
        d.add(Rect::new(i * 24, 900, 2, 2), b);
    }
    assert!(d.rects().len() <= crate::damage::MAX);
    let all = d.bounding();
    assert!(all.contains(0, 0) && all.contains(509, 509));
    assert_eq!(d.take().len() > 0, true);
    assert!(d.is_empty());
}
