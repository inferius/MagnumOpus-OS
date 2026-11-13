//! Snimkove testy systemovych pohledu nad knihovnou ui (paleta, spoustec)
//! ve stejnem ramci jako komponenty (`libs/ui/src/tests/shots.rs`):
//! reference v `libs/comp/tests/shots/`, tolerance, rozdil do
//! `target/shots/`, prepis jen `cargo xtask test --update-shots`.

extern crate std;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use std::path::PathBuf;

use super::*;

const TOL: u32 = 24;
const MAX_BAD: usize = 2;

fn ch(p: u32, s: u32) -> u32 {
    p >> s & 0xFF
}

/// Vyrez snimku obrazovky.
fn crop(t: &T, r: Rect) -> (Vec<u32>, u32, u32) {
    let r = r.intersect(&Rect::new(0, 0, W, H));
    let mut out = Vec::with_capacity((r.w * r.h) as usize);
    for y in r.y..r.y + r.h as i32 {
        for x in r.x..r.x + r.w as i32 {
            out.push(t.px(x, y) | 0xFF00_0000);
        }
    }
    (out, r.w, r.h)
}

fn compare(name: &str, px: &[u32], w: u32, h: u32) -> Option<String> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("shots");
    let file = dir.join(format!("{name}.png"));
    if std::env::var("UPDATE_SHOTS").is_ok_and(|v| v == "1") {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&file, gfx::png::encode(px, w, h, w)).unwrap();
        return None;
    }
    let Ok(data) = std::fs::read(&file) else { return Some(format!("{name}: chybi reference (cargo xtask test --update-shots)")) };
    let Some((rw, rh, refpx)) = gfx::png::decode(&data) else { return Some(format!("{name}: vadne PNG")) };
    let bad = if (rw, rh) != (w, h) { px.len() } else { px.iter().zip(&refpx).filter(|(a, b)| [16, 8, 0].iter().any(|&s| ch(**a, s).abs_diff(ch(**b, s)) > TOL)).count() };
    if bad * 1000 > px.len() * MAX_BAD {
        let o = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
        let _ = std::fs::create_dir_all(&o);
        let _ = std::fs::write(o.join(format!("{name}-actual.png")), gfx::png::encode(px, w, h, w));
        return Some(format!("{name}: lisi se {bad} z {} pixelu", px.len()));
    }
    None
}

/// Paleta a spoustec ve svetlem, tmavem a vysokem kontrastu.
#[test]
fn shot_palette_and_launcher() {
    let mut errors = Vec::new();
    for (mode, mname) in [(theme::Mode::Light, "svetly"), (theme::Mode::Dark, "tmavy"), (theme::Mode::Contrast, "kontrast")] {
        for launcher in [false, true] {
            let mut t = T::new();
            t.c.set_mode(mode);
            let a = t.client("a");
            t.window(a, Mode::Standard, RED);
            t.frame();
            t.c.set_palette(Some(crate::palette::Palette { launcher, ..Default::default() }));
            t.frame();
            let r = t.c.palette_rect().unwrap();
            let (px, w, h) = crop(&t, r);
            let name = format!("{}-{mname}", if launcher { "spoustec" } else { "paleta" });
            errors.extend(compare(&name, &px, w, h));
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
