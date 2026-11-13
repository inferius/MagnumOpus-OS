//! Spoustec (varianta D) a paleta nad knihovnou ui.

use super::*;

/// Pata spoustece: tlacitko napajeni otevre nabidku, Vypnout vede na
/// systemove potvrzeni; psani ve spoustechi hleda jako paleta.
#[test]
fn launcher_footer_power_and_search() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    let l = t.c.bar_layout().launcher;
    t.click(l.x + 4, l.y + 4);
    let r = t.c.palette_rect().unwrap();
    // Napajeni vpravo dole v pate.
    let pad = t.c.style.theme.space[3] as i32;
    let b = t.c.sysui.as_ref().unwrap().ui.a11y().into_iter().find(|n| n.name == "Napájení").expect("tlacitko napajeni").bounds;
    assert!(b.bottom() as i32 + pad <= r.h as i32, "pata se vejde do karty: {b:?} {r:?}");
    t.click(r.x + pad + b.x + b.w as i32 / 2, r.y + pad + b.y + b.h as i32 / 2);
    assert!(t.c.sys_popup_open(), "nabidka napajeni");
    t.key(inputproto::code::KEY_END, false);
    t.key(inputproto::code::KEY_ENTER, false);
    assert!(t.c.palette.is_none());
    assert!(t.c.dialogs.active().is_some(), "potvrzeni vypnuti");
    // Psani ve spoustechi: stejne vysledky jako paleta (Vypnout).
    let mut t = T::new();
    let l = t.c.bar_layout().launcher;
    t.click(l.x + 4, l.y + 4);
    for ch in "vypn".chars() {
        t.c.palette_text(ch);
    }
    assert_eq!(t.c.palette.as_ref().unwrap().query, "vypn");
    assert!(t.c.palette_items().contains(&crate::palette::Item::System(crate::palette::SysAction::Shutdown)));
}

/// Rozpocet: otevreni palety (sestaveni pohledu a kresleni) se vejde do
/// zlomku snimku; meri se na hostiteli (v QEMU je CPU pomalejsi).
#[test]
fn palette_open_budget() {
    extern crate std;
    let mut t = T::new();
    for i in 0..8 {
        let c = t.client(&alloc::format!("app{i}"));
        t.window(c, Mode::Standard, RED);
    }
    let start = std::time::Instant::now();
    let n = 20;
    for _ in 0..n {
        t.c.set_palette(Some(crate::palette::Palette::default()));
        t.frame();
        t.c.set_palette(None);
        t.frame();
    }
    let per = start.elapsed() / n;
    std::println!("otevreni a zavreni palety se snimky: {per:?}");
    assert!(per < std::time::Duration::from_millis(40), "{per:?}");
}
