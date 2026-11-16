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

/// Super samotny otevre a zavre spoustec; Super+klavesa ne; umely vstup ne.
#[test]
fn super_alone_toggles_launcher() {
    use inputproto::code::{KEY_E, KEY_LEFTMETA};
    let mut t = T::new();
    let k = |t: &mut T, code: u16, act: Action, flags: u16| {
        let r = Record::new(Event::Key { code, scancode: code as u32, action: act, ch: None }, 0, flags, t.now);
        t.c.input(&[r], t.now);
    };
    k(&mut t, KEY_LEFTMETA, Action::Press, 0);
    k(&mut t, KEY_LEFTMETA, Action::Release, 0);
    assert!(t.c.palette.as_ref().is_some_and(|p| p.launcher));
    k(&mut t, KEY_LEFTMETA, Action::Press, 0);
    k(&mut t, KEY_LEFTMETA, Action::Release, 0);
    assert!(t.c.palette.is_none());
    k(&mut t, KEY_LEFTMETA, Action::Press, 0);
    k(&mut t, KEY_E, Action::Press, 0);
    k(&mut t, KEY_LEFTMETA, Action::Release, 0);
    assert!(t.c.palette.is_none(), "Super+E neni spoustec");
    let syn = inputproto::event::flags::SYNTHETIC;
    k(&mut t, KEY_LEFTMETA, Action::Press, syn);
    k(&mut t, KEY_LEFTMETA, Action::Release, syn);
    assert!(t.c.palette.is_none(), "umely vstup ne");
}

/// O12: nad spoustecem je kurzor sipka, i kdyz pod nim lezi okraj okna
/// (zmena velikosti okna pod spoustecem se neukazuje); po zavreni zase
/// kurzor okraje.
#[test]
fn launcher_hides_window_edge_cursor() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.frame();
    t.c.set_palette(Some(crate::palette::Palette { launcher: true, ..Default::default() }));
    t.frame();
    let p = t.c.palette_rect().unwrap();
    // Okno presunout tak, aby jeho levy okraj lezel pod spoustecem.
    let g = t.c.clients[&a].windows[&1];
    let r = t.c.windows[&g].rect;
    let (x, y) = (p.x + p.w as i32 / 2, p.y + p.h as i32 / 2);
    t.c.windows.get_mut(&g).unwrap().rect = Rect::new(x, y - 20, r.w, r.h);
    // Bod v zone leveho okraje okna (pod spoustecem).
    let ex = (x - 12..x + 4).find(|&ex| matches!(t.c.hit(ex, y), Some((_, crate::input::Zone::Edge { left: true, .. })))).expect("zona okraje");
    assert!(p.contains(ex, y));
    t.move_to(ex, y);
    assert_eq!(t.c.input.cursor, crate::input::Cursor::Arrow, "nad spoustecem sipka");
    t.c.set_palette(None);
    t.move_to(ex + 1, y);
    t.move_to(ex, y);
    assert_ne!(t.c.input.cursor, crate::input::Cursor::Arrow, "bez spoustece kurzor okraje okna");
}
