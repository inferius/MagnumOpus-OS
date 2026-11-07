//! Testy pisma (na hostiteli: `cargo test -p font`).

use crate::math::{ceil, floor, sqrt};
use crate::*;

fn tuffy() -> Font<'static> {
    Font::parse(TUFFY).expect("Tuffy se musi precist")
}

#[test]
fn math_helpers() {
    assert_eq!(floor(1.5), 1.0);
    assert_eq!(floor(-1.5), -2.0);
    assert_eq!(floor(-2.0), -2.0);
    assert_eq!(ceil(1.2), 2.0);
    assert_eq!(ceil(-1.2), -1.0);
    assert!((sqrt(2.0) - 1.414_213_5).abs() < 1e-5);
    assert_eq!(sqrt(-1.0), 0.0);
    assert!((sqrt(1e6) - 1000.0).abs() < 1e-2);
}

#[test]
fn parses_tuffy_metrics_and_cmap() {
    let f = tuffy();
    assert!(f.units_per_em() >= 1000);
    assert!(f.glyph_count() > 200);
    for ch in "AaZz09 .,ěščřžýáíéůúťďňĚŠČŘŽÝÁÍÉŮÚŤĎŇ".chars() {
        assert!(f.has_glyph(ch), "chybi glyf {ch:?}");
    }
    assert_eq!(f.glyph_index('\u{10FFFF}'), 0);
    let a = f.glyph_index('A');
    assert!(f.advance_units(a) > 0);
    let m = f.line_metrics(16.0);
    assert!(m.ascent > 10.0 && m.ascent < 20.0, "{m:?}");
    assert!(m.descent > 1.0 && m.descent < 8.0, "{m:?}");
    assert!(m.height() >= 16);
}

#[test]
fn kerning_pairs_when_present() {
    let f = tuffy();
    let (a, v) = (f.glyph_index('A'), f.glyph_index('V'));
    // Tuffy ma tabulku kern; AV se typicky stahuje k sobe.
    let k = f.kern_units(a, v);
    assert!(k <= 0, "{k}");
    assert_eq!(f.kern_units(0, 0), 0);
}

#[test]
fn rasterize_letter_i_is_vertical_bar() {
    let f = tuffy();
    let g = f.rasterize(f.glyph_index('l'), 32.0);
    assert!(g.w >= 2 && g.h >= 20, "{}x{}", g.w, g.h);
    assert!(g.top < 0, "glyf je nad zakladnou");
    assert!(g.advance > 3.0);
    // Uprostred vysky je souvisly plny pruh.
    let row = g.h as usize / 2;
    let line = &g.coverage[row * g.w as usize..(row + 1) * g.w as usize];
    assert!(line.iter().any(|&c| c == 255), "{line:?}");
    // Nad glyfem (mimo masku) nic, horni a dolni radek masky neco.
    let full: u32 = g.coverage.iter().map(|&c| c as u32).sum();
    assert!(full > 0);
}

#[test]
fn rasterize_with_diacritics_has_more_ink_above() {
    let f = tuffy();
    let c = f.rasterize(f.glyph_index('c'), 24.0);
    let cc = f.rasterize(f.glyph_index('č'), 24.0);
    assert!(cc.top < c.top, "hacek je nad pismenem: {} {}", cc.top, c.top);
    let ink = |g: &Glyph| g.coverage.iter().map(|&v| v as u32).sum::<u32>();
    assert!(ink(&cc) > ink(&c));
}

#[test]
fn space_is_empty_but_advances() {
    let f = tuffy();
    let g = f.rasterize(f.glyph_index(' '), 16.0);
    assert_eq!(g.w * g.h, 0);
    assert!(g.advance > 2.0);
}

#[test]
fn text_width_grows_with_text() {
    let f = tuffy();
    let a = f.text_width("Ahoj", 16.0);
    let b = f.text_width("Ahoj světe", 16.0);
    assert!(a > 20 && b > a, "{a} {b}");
    assert_eq!(f.text_width("", 16.0), 0);
}

#[test]
fn square_coverage_exact() {
    // Ctverec 4x4 posunuty o pul pixelu: vnitrek plny, okraje polovicni.
    let mut r = Rasterizer::new(6, 6);
    let p = |x: f32, y: f32| Point { x, y };
    let (a, b, c, d) = (p(0.5, 0.5), p(4.5, 0.5), p(4.5, 4.5), p(0.5, 4.5));
    r.line(a, b);
    r.line(b, c);
    r.line(c, d);
    r.line(d, a);
    let m = r.finish();
    let at = |x: usize, y: usize| m[y * 6 + x];
    assert_eq!(at(2, 2), 255);
    assert_eq!(at(0, 2), 128);
    assert_eq!(at(4, 2), 128);
    assert_eq!(at(2, 0), 128);
    assert_eq!(at(0, 0), 64);
    assert_eq!(at(5, 5), 0);
}

#[test]
fn rasterizer_survives_wild_coordinates() {
    let mut r = Rasterizer::new(4, 4);
    let p = |x: f32, y: f32| Point { x, y };
    r.line(p(-1e9, -1e9), p(1e9, 1e9));
    r.line(p(f32::NAN, 0.0), p(1.0, 3.0));
    r.line(p(2.0, -10.0), p(2.0, 100.0));
    r.quad(p(0.0, 0.0), p(1e30, 1e30), p(3.0, 3.0));
    let _ = r.finish();
}

#[test]
fn not_truetype_and_corrupt() {
    assert_eq!(Font::parse(b"").err(), Some(FontError::NotTrueType));
    assert_eq!(Font::parse(b"OTTO\0\0\0\0\0\0\0\0").err(), Some(FontError::Unsupported));
    assert_eq!(Font::parse(&[0, 1, 0, 0, 0, 0]).err(), Some(FontError::NotTrueType));
}

/// Jednoduchy generator (bez zavislosti).
fn lcg(s: &mut u64) -> u64 {
    *s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *s >> 33
}

#[test]
fn garbage_never_panics() {
    // Zkraceny soubor.
    for cut in (0..TUFFY.len()).step_by(997) {
        if let Ok(f) = Font::parse(&TUFFY[..cut]) {
            for ch in "Aač ".chars() {
                let _ = f.rasterize(f.glyph_index(ch), 12.0);
            }
        }
    }
    // Nahodne prepsane bajty (vcetne tabulek glyf a loca).
    let mut seed = 42u64;
    for _ in 0..60 {
        let mut d = TUFFY.to_vec();
        for _ in 0..200 {
            let i = lcg(&mut seed) as usize % d.len();
            d[i] = lcg(&mut seed) as u8;
        }
        if let Ok(f) = Font::parse(&d) {
            for gid in (0..f.glyph_count()).step_by(7) {
                let _ = f.rasterize(gid, 14.0);
            }
            let _ = f.text_width("Příliš žluťoučký kůň", 14.0);
        }
    }
}
