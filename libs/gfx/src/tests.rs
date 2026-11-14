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
    // Vadny vstup (slozka > alfa) nepretece; nad nepruhlednym pixelem
    // zustane alfa 255.
    let v = over(0xFFFF_FFFF, 0x10FF_FFFF);
    assert_eq!(v, 0xFFFF_FFFF, "{v:x}");
    // Text (castecne pokryti) nad nepruhlednou vyplni: alfa 255, ne 254.
    assert_eq!(over(0xFF31_3740, 0x8040_4040) >> 24, 255);
    assert_eq!(over(0x8000_0000, 0x4000_0000) >> 24, 0x40 + 0x60);
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

#[test]
fn text_draws_and_measures() {
    let mut t = crate::text::Text::system();
    let mut px = vec![0xFF00_0000u32; 200 * 40];
    let mut c = Canvas::new(&mut px, 200, 40, 200).unwrap();
    let (lh, base) = t.line(16.0);
    assert!(lh >= 16 && base > 8);
    let end = t.draw(&mut c, 4, base, "Žluťoučký", 16.0, rgb(255, 255, 255));
    let w = t.width("Žluťoučký", 16.0) as i32;
    assert!((end - 4 - w).abs() <= 2, "{end} {w}");
    let lit = (0..200 * 40).filter(|&i| c.get(i % 200, i / 200) != 0xFF00_0000).count();
    assert!(lit > 50, "text neco nakreslil: {lit}");
    let pos = t.positions("ab", 16.0);
    assert_eq!(pos.len(), 3);
    assert!(pos[0] == 0 && pos[1] > 0 && pos[2] > pos[1]);
}

// ---- Vzhled (tvary, stin, ikony, plocha, pisma) ----

fn canvas_buf(w: u32, h: u32, c: u32) -> alloc::vec::Vec<u32> {
    alloc::vec![c; (w * h) as usize]
}

#[test]
fn round_fill_corners_are_smooth() {
    let mut buf = canvas_buf(40, 40, 0xFF00_0000);
    let mut c = Canvas::new(&mut buf, 40, 40, 40).unwrap();
    c.round_fill(Rect::new(0, 0, 40, 40), 18, 0xFFFF_FFFF);
    // Stred plny, roh prazdny, na hrane mezihodnota.
    assert_eq!(c.get(20, 20), 0xFFFF_FFFF);
    assert_eq!(c.get(0, 0), 0xFF00_0000);
    let edge: alloc::vec::Vec<u32> = (0..18).map(|x| c.get(x, 5) & 0xFF).collect();
    assert!(edge.iter().any(|&v| v > 10 && v < 245), "{edge:?}");
    // Symetrie.
    assert_eq!(c.get(3, 7), c.get(36, 7));
    assert_eq!(c.get(3, 7), c.get(3, 32));
}

#[test]
fn round_stroke_is_ring() {
    let mut buf = canvas_buf(50, 50, 0);
    let mut c = Canvas::new(&mut buf, 50, 50, 50).unwrap();
    c.round_stroke(Rect::new(5, 5, 40, 40), 10, 1, 0xFFFF_FFFF);
    assert_eq!(c.get(25, 25), 0, "stred prazdny");
    assert_eq!(c.get(25, 5), 0xFFFF_FFFF, "horni hrana");
    assert_eq!(c.get(25, 6), 0);
}

#[test]
fn corners_restore_rounds_content() {
    let mut buf = canvas_buf(60, 60, 0xFF11_2233);
    let mut c = Canvas::new(&mut buf, 60, 60, 60).unwrap();
    let r = Rect::new(10, 10, 40, 40);
    let saved = c.save_corners(r, 12);
    c.fill(r, 0xFFFF_FFFF);
    c.restore_corners(r, 12, &saved);
    assert_eq!(c.get(10, 10), 0xFF11_2233, "roh je zase pozadi");
    assert_eq!(c.get(30, 30), 0xFFFF_FFFF);
    assert_eq!(c.get(10, 30), 0xFFFF_FFFF, "rovna hrana zustane");
}

#[test]
fn shadow_mask_is_soft_and_symmetric() {
    let m = shadow::ShadowMask::new(18, 32);
    let mut buf = canvas_buf(400, 300, 0xFFFF_FFFF);
    let mut c = Canvas::new(&mut buf, 400, 300, 400).unwrap();
    let shape = Rect::new(100, 80, 200, 140);
    c.shadow(&m, shape, 0xFF00_0000, Rect::default());
    let v = |x, y| 255 - (c.get(x, y) & 0xFF);
    // Pod tvarem tma, daleko svetlo, mezi tim prechod.
    assert!(v(200, 150) > 200);
    assert_eq!(v(10, 10), 0);
    let mid = v(200, 80 - 10);
    assert!(mid > 5 && mid < 200, "{mid}");
    assert_eq!(v(100 - 8, 150), v(300 + 7, 150));
    // Monotonni pokles smerem ven.
    let mut prev = 256;
    for x in (60..100).rev() {
        let a = v(x, 150);
        assert!(a <= prev);
        prev = a;
    }
}

#[test]
fn icons_from_prototype() {
    let set = icon::parse_set(icon::SOURCE);
    assert!(set.len() >= 60, "{}", set.len());
    for name in ["close", "min", "max", "shield", "apps", "tasks", "terminal", "folder", "agent", "bell", "wifi", "volume"] {
        let i = set.iter().find(|i| i.name == name).unwrap_or_else(|| panic!("{name}"));
        let m = i.render(24, 1.75);
        let ink: u32 = m.coverage.iter().map(|&v| v as u32).sum();
        assert!(ink > 255 * 10, "{name}: {ink}");
    }
    // Kazda ikona jde vykreslit ve 4 velikostech.
    for i in &set {
        for s in [16, 20, 24, 32] {
            let m = i.render(s, 1.75);
            assert_eq!(m.coverage.len(), (s * s) as usize);
            assert!(m.coverage.iter().any(|&v| v > 128), "{} {s}", i.name);
        }
    }
    // Krizek: uhlopricky, stred plny, roh prazdny.
    let close = set.iter().find(|i| i.name == "close").unwrap().render(24, 2.0);
    assert!(close.coverage[12 * 24 + 12] > 200);
    assert_eq!(close.coverage[0], 0);
    // Poskozena cesta nespadne.
    for bad in ["<path d=\"M1 2 L\"/>", "<path d=\"Q 1 2 3\"/>", "<path d=\"A 1 1 0 2 1 3 3\"/>", "<rect/>", "<path d=\"Z Z M\"/>"] {
        let p = icon::parse_svg(bad);
        let _ = icon::Icon { name: "x".into(), parts: p }.render(16, 1.75);
    }
}

#[test]
fn backdrop_gradients() {
    let img = backdrop::render(
        100,
        80,
        0xFF00_0000,
        &[backdrop::Radial { cx: 0.2, cy: 0.2, stop: 0.5, color: 0xFFFF_0000 }],
    );
    let red = |x: u32, y: u32| img[(y * 100 + x) as usize] >> 16 & 0xFF;
    assert!(red(20, 16) > 240);
    assert_eq!(red(99, 79), 0);
    assert!(img.iter().all(|p| p >> 24 == 0xFF));
}

#[test]
fn family_picks_weights_and_subpixel_text() {
    let mut f = text::Family::new(
        &[(400, include_bytes!("../../font/fonts/Inter-Regular.ttf")), (600, include_bytes!("../../font/fonts/Inter-SemiBold.ttf"))],
        &[*b"tnum"],
    );
    assert_eq!(f.weights(), alloc::vec![400, 600]);
    let w400 = f.face(400).width("Okna a plocha", 14.0);
    let w600 = f.face(650).width("Okna a plocha", 14.0);
    assert!(w600 > w400, "{w400} {w600}");
    // Kerning z GPOS se projevi i v sirce pres Text.
    assert!(f.face(400).width("Te", 28.0) < f.face(400).width("T", 28.0) + f.face(400).width("e", 28.0));
    let mut buf = canvas_buf(200, 40, 0xFFFF_FFFF);
    let mut c = Canvas::new(&mut buf, 200, 40, 200).unwrap();
    let end = f.face(400).draw_ellipsized(&mut c, 2, 28, "Velmi dlouhy titulek okna, ktery se nevejde", 14.0, 0xFF2A_1F2D, 120);
    assert!(end <= 2 + 120 + 1, "{end}");
    assert!(c.get(10, 22) != 0xFFFF_FFFF || c.get(11, 22) != 0xFFFF_FFFF || c.get(12, 22) != 0xFFFF_FFFF);
    // Bez rezu zalozni pismo.
    let mut e = text::Family::new(&[], &[]);
    assert!(e.face(700).width("a", 14.0) > 0);
}

/// PNG s kompresi: zapis a cteni zpet beze ztraty; plochy obrazku UI se
/// zmensi desitkykrat; vadna data nepanikari.
#[test]
fn png_roundtrip_and_compression() {
    let (w, h) = (320u32, 120u32);
    let mut px = vec![0xFFF0_F0F0u32; (w * h) as usize];
    for y in 20..60 {
        for x in 10..300 {
            px[(y * w + x) as usize] = if (x / 7 + y) % 5 == 0 { 0xFF20_50C4 } else { 0xFFFF_FFFF };
        }
    }
    let png = crate::png::encode(&px, w, h, w);
    assert!(png.len() < (w * h * 3 / 20) as usize, "komprese: {} B", png.len());
    let (dw, dh, back) = crate::png::decode(&png).unwrap();
    assert_eq!((dw, dh), (w, h));
    assert_eq!(back, px);
    // Nahodna data (spatne komprimovatelna) taky beze ztraty.
    let mut s = 12345u32;
    let noise: Vec<u32> = (0..64 * 64)
        .map(|_| {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12345);
            0xFF00_0000 | (s >> 8)
        })
        .collect();
    assert_eq!(crate::png::decode(&crate::png::encode(&noise, 64, 64, 64)).unwrap().2, noise);
    // Poskozeny soubor: None, ne panika.
    let mut bad = png.clone();
    for i in (60..bad.len()).step_by(7) {
        bad[i] ^= 0x5A;
    }
    let _ = crate::png::decode(&bad);
    assert!(crate::png::decode(&png[..40]).is_none());
}

/// Pokryti zaobleneho obdelniku bez `f32` mimo rohy dava presne stejne
/// hodnoty jako vzdalenost v `f32` pro kazdy pixel (snimky se nezmeni).
#[test]
fn round_rect_coverage_matches_float() {
    use crate::shape::{coverage, round_rect_coverage, round_rect_distance};
    for &(x, y, w, h) in &[(0, 0, 10, 10), (3, -2, 17, 9), (100, 700, 1264, 56), (5, 5, 1, 1), (-7, 4, 31, 30), (2, 3, 0, 5)] {
        let r = Rect::new(x, y, w, h);
        for radius in [0u32, 1, 3, 8, 12, 15, 40] {
            let rad = radius.min(r.w / 2).min(r.h / 2);
            for py in y - 2..y + h as i32 + 2 {
                for px in x - 2..x + w as i32 + 2 {
                    let f = if r.is_empty() { 0 } else { coverage(round_rect_distance(px as f32 + 0.5, py as f32 + 0.5, r, rad as f32)) };
                    assert_eq!(round_rect_coverage(px, py, r, rad), f, "{r:?} r={rad} ({px},{py})");
                }
            }
        }
    }
}

/// Puvodni rozmazani v `f32` jako reference celociselneho.
mod blur_ref {
    use alloc::vec;
    use alloc::vec::Vec;

    use crate::Rect;
    use crate::shadow::box_sizes;

    pub(super) fn blurred_f32(src: &[u32], stride: u32, area: Rect, sigma: f32) -> Vec<u32> {
        let (w, h) = (area.w as usize, area.h as usize);
        if w == 0 || h == 0 {
            return Vec::new();
        }
        let (hw, hh) = (w.div_ceil(2), h.div_ceil(2));
        // Polovicni rozliseni: prumer 2x2, ctyri kanaly v f32.
        let mut ch = vec![[0f32; 4]; hw * hh];
        for y in 0..hh {
            for x in 0..hw {
                let mut acc = [0f32; 4];
                let mut n = 0.0;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let (sx, sy) = (2 * x + dx, 2 * y + dy);
                        if sx >= w || sy >= h {
                            continue;
                        }
                        let p = src[(area.y as usize + sy) * stride as usize + area.x as usize + sx];
                        for (k, a) in acc.iter_mut().enumerate() {
                            *a += (p >> (24 - 8 * k) & 0xFF) as f32;
                        }
                        n += 1.0;
                    }
                }
                ch[y * hw + x] = acc.map(|v| v / n);
            }
        }
        for bw in box_sizes((sigma / 2.0).max(0.5)) {
            box_pass_f32(&mut ch, hw, hh, bw, true);
            box_pass_f32(&mut ch, hw, hh, bw, false);
        }
        // Zpet do plneho rozliseni (bilinearne).
        let mut out = Vec::with_capacity(w * h);
        for y in 0..h {
            let fy = (y as f32 - 0.5) / 2.0;
            let y0 = (fy.max(0.0) as usize).min(hh - 1);
            let y1 = (y0 + 1).min(hh - 1);
            let ty = (fy - y0 as f32).clamp(0.0, 1.0);
            for x in 0..w {
                let fx = (x as f32 - 0.5) / 2.0;
                let x0 = (fx.max(0.0) as usize).min(hw - 1);
                let x1 = (x0 + 1).min(hw - 1);
                let tx = (fx - x0 as f32).clamp(0.0, 1.0);
                let (a, b, c, d) = (ch[y0 * hw + x0], ch[y0 * hw + x1], ch[y1 * hw + x0], ch[y1 * hw + x1]);
                let mut px = 0u32;
                for k in 0..4 {
                    let top = a[k] + (b[k] - a[k]) * tx;
                    let bot = c[k] + (d[k] - c[k]) * tx;
                    let v = (top + (bot - top) * ty + 0.5).clamp(0.0, 255.0) as u32;
                    px |= v << (24 - 8 * k);
                }
                out.push(px);
            }
        }
        out
    }
    
    /// Krabickovy filtr sirky `bw` po radcich (`rows`) nebo sloupcich, okraje
    /// prodlouzene.
    fn box_pass_f32(a: &mut [[f32; 4]], w: usize, h: usize, bw: u32, rows: bool) {
        let r = (bw / 2) as isize;
        let (n, lines) = if rows { (w, h) } else { (h, w) };
        let mut line = vec![[0f32; 4]; n];
        let inv = 1.0 / bw as f32;
        for l in 0..lines {
            let idx = |i: usize| if rows { l * w + i } else { i * w + l };
            for (i, v) in line.iter_mut().enumerate() {
                *v = a[idx(i)];
            }
            let at = |i: isize| line[i.clamp(0, n as isize - 1) as usize];
            let mut acc = [0f32; 4];
            for i in -r..=r {
                let v = at(i);
                for k in 0..4 {
                    acc[k] += v[k];
                }
            }
            for i in 0..n as isize {
                a[idx(i as usize)] = acc.map(|v| v * inv);
                let (add, sub) = (at(i + r + 1), at(i - r));
                for k in 0..4 {
                    acc[k] += add[k] - sub[k];
                }
            }
        }
    }
}

#[test]
fn blur_int_matches_float() {
    let (w, h) = (97u32, 61u32);
    let mut src = alloc::vec![0u32; (w * h) as usize];
    let mut s = 12345u32;
    for (i, p) in src.iter_mut().enumerate() {
        s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        // Nahodne pruhledne pixely (premultiplied) a ostre hrany.
        let a = if i % 7 == 0 { 255 } else { s >> 24 };
        let c = |sh: u32| ((s >> sh) & 0xFF) * a / 255;
        *p = a << 24 | c(16) << 16 | c(8) << 8 | c(0);
    }
    for (area, sigma) in [(Rect::new(0, 0, w, h), 16.0f32), (Rect::new(3, 5, 40, 17), 4.0), (Rect::new(10, 10, 1, 1), 16.0), (Rect::new(0, 0, w, h), 1.0)] {
        let a = crate::blur::blurred_region(&src, w, area, sigma);
        let b = blur_ref::blurred_f32(&src, w, area, sigma);
        assert_eq!(a.len(), b.len());
        for (i, (x, y)) in a.iter().zip(&b).enumerate() {
            for sh in [24, 16, 8, 0] {
                let d = ((x >> sh) & 0xFF).abs_diff((y >> sh) & 0xFF);
                assert!(d <= 1, "{area:?} pixel {i}: {x:08x} vs {y:08x}");
            }
        }
    }
}
