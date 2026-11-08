//! Kontrast barev textu v terminalu: kazda barva (i tlumena) aspon 4,5:1
//! proti pozadi motivu i proti pozadi z tokenu (`term_bg` svetly, tmavy,
//! vysoky kontrast), na kterem je okno terminalu kresli.

use crate::theme::{Rgb, Theme};

fn lin(c: u8) -> f64 {
    let c = c as f64 / 255.0;
    if c <= 0.039_28 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn lum(c: Rgb) -> f64 {
    0.2126 * lin(c.0) + 0.7152 * lin(c.1) + 0.0722 * lin(c.2)
}

pub fn ratio(a: Rgb, b: Rgb) -> f64 {
    let (x, y) = (lum(a), lum(b));
    let (hi, lo) = if x > y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}

/// Vsechny barvy textu motivu s popisem.
fn inks(t: &Theme) -> Vec<(String, Rgb)> {
    let u = &t.ui;
    let mut v = vec![
        ("ui.fg".into(), u.fg),
        ("ui.muted".into(), u.muted),
        ("ui.accent".into(), u.accent),
        ("ui.ok".into(), u.ok),
        ("ui.warn".into(), u.warn),
        ("ui.err".into(), u.err),
    ];
    v.extend(t.syntax.iter().enumerate().map(|(i, s)| (format!("syntax[{i}]"), s.fg)));
    v.extend(t.files.iter().enumerate().map(|(i, s)| (format!("files[{i}]"), s.fg)));
    v.extend(t.scale.iter().enumerate().map(|(i, c)| (format!("scale[{i}]"), *c)));
    v
}

fn check(t: &Theme, bg: Rgb, what: &str) -> Vec<String> {
    let mut bad = Vec::new();
    for (name, c) in inks(t) {
        let r = ratio(c, bg);
        if r < 4.5 {
            bad.push(format!("{what} {name} #{:02x}{:02x}{:02x}: {r:.2}", c.0, c.1, c.2));
        }
        // Vybrany radek a listy: text i na pozadi `select` citelny.
        let s = ratio(c, t.ui.select);
        if s < 4.5 && name.starts_with("ui.") {
            bad.push(format!("{what} {name} na select: {s:.2}"));
        }
    }
    bad
}

#[test]
fn every_terminal_color_has_contrast() {
    let mut bad = Vec::new();
    let light = Theme::light();
    let dark = Theme::dark();
    bad.extend(check(&light, light.ui.bg, "svetly"));
    bad.extend(check(&dark, dark.ui.bg, "tmavy"));
    // Okno terminalu: pozadi z tokenu (design/tokens.toml).
    bad.extend(check(&light, Rgb(0xff, 0xfd, 0xfb), "svetly/term_bg"));
    bad.extend(check(&dark, Rgb(0x1c, 0x15, 0x21), "tmavy/term_bg"));
    bad.extend(check(&dark, Rgb(0, 0, 0), "kontrast/term_bg"));
    assert!(bad.is_empty(), "malo kontrastni barvy:\n{}", bad.join("\n"));
}
