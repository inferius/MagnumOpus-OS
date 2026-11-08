use alloc::string::String;
use alloc::vec::Vec;

use crate::*;

#[test]
fn tokens_builtin() {
    let t = Tokens::parse(DEFAULT).unwrap();
    for m in Mode::ALL {
        let th = t.theme(m).unwrap();
        assert_eq!(th.mode, m);
    }
    let l = t.theme(Mode::Light).unwrap();
    assert_eq!(l.color.bg, 0xFFFB_F1EA);
    assert_eq!(l.color.surface, 0xFFFF_FDFB);
    assert_eq!(l.text.body.size, 14.0);
    assert_eq!(l.text.heading.weight, 650);
    assert_eq!(l.size.title_h, 40);
    assert_eq!(l.size.bar_h, 52);
    assert_eq!(l.icon_stroke, 1.75);
    assert_eq!(l.space[5], 16);
    assert_eq!(l.radius.pill, 999);
    assert_eq!(l.motion.frame_ms, 16.7);
    assert_eq!(l.layer.system_dialog, 1000);
    assert_eq!(l.font.ui, "Inter");
    assert_eq!(l.font.ui_features, alloc::vec![(*b"tnum", true), (*b"cv11", true)]);
    assert_eq!(l.font.mono_features, alloc::vec![(*b"calt", false), (*b"liga", false)]);
    assert_eq!(l.shadow.window.len(), 2);
    assert_eq!(l.shadow.window[0], ShadowLayer { x: 0, y: 12, blur: 32, spread: 0, color: to_premul(107, 63, 214, 41) });
    let c = t.theme(Mode::Contrast).unwrap();
    assert!(c.shadow.window.is_empty());
    // scrim je poloprusvitny.
    assert_eq!(c.color.scrim >> 24, 204);
}

/// Stejna kontrola jako `node design/tools/build-tokens.mjs --check`.
#[test]
fn contrast_pairs_pass() {
    let t = Tokens::builtin();
    let pairs = t.contrast_pairs().unwrap();
    assert!(pairs.len() >= 30);
    for m in Mode::ALL {
        let th = t.theme(m).unwrap();
        let bad = th.contrast_failures(&pairs);
        assert!(bad.is_empty(), "{m:?}: {bad:?}");
    }
}

#[test]
fn missing_and_invalid() {
    // Chybejici klic ma v chybe celou cestu.
    let text = DEFAULT.replace("ink_muted = \"#665569\"\n", "");
    let t = Tokens::parse(&text).unwrap();
    assert_eq!(t.theme(Mode::Light).unwrap_err(), Error::Missing("color.light.ink_muted".into()));
    assert!(t.theme(Mode::Dark).is_ok());
    let text = DEFAULT.replace("accent = \"#6b3fd6\"", "accent = \"fialova\"");
    let t = Tokens::parse(&text).unwrap();
    assert_eq!(t.theme(Mode::Light).unwrap_err(), Error::Invalid("color.light.accent".into()));
    let text = DEFAULT.replace("ease_standard = \"0.2 0 0 1\"", "ease_standard = \"0.2 0 0\"");
    assert!(Tokens::parse(&text).unwrap().theme(Mode::Light).is_err());
    assert!(matches!(Tokens::parse("[a\n"), Err(Error::Syntax(_))));
}

#[test]
fn shadows() {
    assert_eq!(parse_shadow("none"), Some(Vec::new()));
    let s = parse_shadow("0 2 6 0 rgba(42,31,45,0.10) | 1 -2 3 4 #000000").unwrap();
    assert_eq!(s.len(), 2);
    assert_eq!(s[1], ShadowLayer { x: 1, y: -2, blur: 3, spread: 4, color: 0xFF00_0000 });
    assert_eq!(parse_shadow("0 2 -6 0 #000000"), None);
    assert_eq!(parse_shadow("0 2 x 0 #000000"), None);
}

#[test]
fn curves() {
    let t = Theme::builtin(Mode::Light);
    let c = t.motion.ease_standard;
    assert!(c.at(0.0).abs() < 1e-3);
    assert!((c.at(1.0) - 1.0).abs() < 1e-3);
    // Standardni krivka rychle zrychli: v polovine casu je pres 80 %.
    assert!(c.at(0.5) > 0.8, "{}", c.at(0.5));
    let mut prev = 0.0;
    for i in 0..=20 {
        let v = c.at(i as f32 / 20.0);
        assert!(v + 1e-4 >= prev);
        prev = v;
    }
    let _: String = t.font.mono;
}
