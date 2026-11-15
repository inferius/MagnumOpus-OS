//! Rezim vyvojare v liste (jen `--features dev-mode`; ve vydani modul
//! neni, viz `devmode_ve_vydani_neni`).

use super::*;

/// Bez zpravy sluzby uctu pilulka neni; stav od sluzby ji ukaze, klik
/// otevre kartu a prvni radek posle prepnuti sluzbe uctu. Lista sama
/// stav nemeni, az nova zprava sluzby (zapnuto = vyrazny pruh).
#[test]
fn devmode_pilulka_karta_a_prepnuti() {
    let mut t = T::new();
    assert!(t.c.bar_layout().dev.is_none(), "bez stavu od sluzby uctu neni pilulka");
    t.c.dialog_request(7, DialogRequest::DevMode { on: false }, t.now);
    let r = t.c.bar_layout().dev.expect("pilulka rezimu vyvojare");
    t.frame();
    t.click(r.x + 4, r.y + r.h as i32 / 2);
    assert_eq!(t.c.flyout, Some(crate::flyout::Flyout::Dev), "karta rezimu");
    let f = t.c.flyout_rect().unwrap();
    let th = &t.c.style.theme;
    let y = f.y + (th.space[4] + th.text.body_strong.line + th.space[2] + th.text.body.line / 2) as i32;
    t.click(f.x + 8, y);
    let out = t.out();
    assert!(out.iter().any(|o| matches!(o, Out::Dialog(7, DialogReply::DevMode { on: true }))), "prepnuti jde sluzbe uctu: {out:?}");
    assert!(!t.c.dev_mode_on(), "lista stav sama nezmeni");
    t.c.dialog_request(7, DialogRequest::DevMode { on: true }, t.now);
    assert!(t.c.dev_mode_on());
    let on = t.c.bar_layout().dev.unwrap();
    assert!(on.w > r.w, "zapnuto: pruh s vysvetlenim");
    t.frame();
    // Pruh ma barvu nebezpeci (vyrazny, ne jen text).
    let danger = t.c.style.theme.color.danger;
    assert_eq!(t.px(on.x + on.w as i32 / 2, on.y + 2) | 0xFF00_0000, danger | 0xFF00_0000, "barva pruhu");
    save_png("comp-rezim-vyvojare.png", &t.frame, W, H);
}
