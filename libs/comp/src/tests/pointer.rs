//! Kurzor aplikace, cela obrazovka na zadost a zamek ukazatele
//! (`pointer.rs`, guiproto 5).

use alloc::vec;
use alloc::vec::Vec;

use guiproto::cursor::{Cursor as AppCursor, Custom, Shape};

use super::*;
use crate::input::Cursor;

/// Cislo gesta posledniho fyzickeho stisku doruceneho klientovi `c`.
fn serial_in(out: &[Out], c: ClientId) -> u32 {
    client_msgs(out, c)
        .iter()
        .rev()
        .find_map(|m| match m {
            ServerMsg::Input { records, .. } => records.iter().rev().find(|r| r.serial != 0).map(|r| r.serial),
            _ => None,
        })
        .unwrap_or(0)
}

fn lock_states(out: &[Out], c: ClientId) -> Vec<bool> {
    client_msgs(out, c)
        .into_iter()
        .filter_map(|m| match m {
            ServerMsg::PointerLockState { locked, .. } => Some(locked),
            _ => None,
        })
        .collect()
}

fn relative(out: &[Out], c: ClientId) -> Vec<(i32, i32)> {
    client_msgs(out, c)
        .into_iter()
        .flat_map(|m| match m {
            ServerMsg::Input { records, .. } => records,
            _ => Vec::new(),
        })
        .filter_map(|r| match r.event {
            InputEvent::PointerRelative { dx, dy } => Some((dx, dy)),
            _ => None,
        })
        .collect()
}

fn errors(out: &[Out], c: ClientId) -> Vec<(u32, guiproto::Error)> {
    client_msgs(out, c)
        .into_iter()
        .filter_map(|m| match m {
            ServerMsg::Error { op, code } => Some((op, code)),
            _ => None,
        })
        .collect()
}

#[test]
fn app_cursor_over_content_only() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    let r = t.rect_of(a);
    let g = t.c.clients[&a].windows[&1];
    t.msg(a, ClientMsg::SetCursor { window: 1, cursor: AppCursor::Shape(Shape::Crosshair) });
    t.move_to(r.x + 50, r.y + 50);
    assert_eq!(t.c.input.cursor, Cursor::App(g), "nad obsahem kurzor aplikace");
    t.frame();
    // Kriz: cerny stred s bilym obrysem.
    assert_eq!(t.px(r.x + 50, r.y + 50) & 0xFF_FFFF, 0);
    assert_eq!(t.px(r.x + 50, r.y + 50 - 1) & 0xFF_FFFF, 0, "svisla cara krize");
    // Nad titulkem systemova sipka.
    t.move_to(r.x + 50, r.y - 10);
    assert_eq!(t.c.input.cursor, Cursor::Arrow);
    // Vlastni obrazek 2x2 s aktivnim bodem 1,1.
    t.move_to(r.x + 60, r.y + 60);
    let px = vec![0xFF12_3456; 4];
    t.msg(a, ClientMsg::SetCursor { window: 1, cursor: AppCursor::Custom(Custom { w: 2, h: 2, hot_x: 1, hot_y: 1, pixels: px }) });
    t.frame();
    assert_eq!(t.px(r.x + 59, r.y + 59), 0xFF12_3456, "vlastni kurzor od aktivniho bodu: {:?} {:?}", t.c.input.cursor, t.c.app_cursor_rect(t.c.clients[&a].windows[&1]));
    // Skryty: zadne pixely kurzoru, plocha okna.
    t.msg(a, ClientMsg::SetCursor { window: 1, cursor: AppCursor::Shape(Shape::Hidden) });
    t.frame();
    assert_eq!(t.px(r.x + 59, r.y + 59), RED);
    // Default = zpet systemova sipka.
    t.msg(a, ClientMsg::SetCursor { window: 1, cursor: AppCursor::Shape(Shape::Default) });
    assert_eq!(t.c.input.cursor, Cursor::Arrow);
    // Cizi okno: chyba.
    t.out();
    t.msg(a, ClientMsg::SetCursor { window: 9, cursor: AppCursor::Shape(Shape::Text) });
    assert_eq!(errors(&t.out(), a), vec![(guiproto::window::op::SET_CURSOR, guiproto::Error::InvalidArgs)]);
}

#[test]
fn fullscreen_request_needs_gesture() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    let g = t.c.clients[&a].windows[&1];
    t.out();
    // Bez gesta: odmitnuto.
    t.msg(a, ClientMsg::WindowState { window: 1, fullscreen: true, gesture: 0 });
    assert!(!t.c.is_fullscreen(g));
    assert_eq!(errors(&t.out(), a), vec![(guiproto::window::op::WINDOW_STATE, guiproto::Error::AccessDenied)]);
    // Fyzicky klik: gesto, cela obrazovka.
    let r = t.rect_of(a);
    t.click(r.x + 20, r.y + 20);
    let s = serial_in(&t.out(), a);
    assert_ne!(s, 0);
    t.msg(a, ClientMsg::WindowState { window: 1, fullscreen: true, gesture: s });
    assert!(t.c.is_fullscreen(g), "cela obrazovka po fyzickem stisku");
    // Gesto je jednorazove, navrat ho nepotrebuje.
    t.msg(a, ClientMsg::WindowState { window: 1, fullscreen: false, gesture: 0 });
    assert!(!t.c.is_fullscreen(g));
    t.msg(a, ClientMsg::WindowState { window: 1, fullscreen: true, gesture: s });
    assert!(!t.c.is_fullscreen(g), "pouzite gesto znovu neplati");
}

#[test]
fn pointer_lock_rules_and_release() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    let g = t.c.clients[&a].windows[&1];
    t.out();
    // Bez gesta: zamek odmitnut.
    t.msg(a, ClientMsg::PointerLock { window: 1, lock: true, gesture: 0 });
    assert_eq!(lock_states(&t.out(), a), vec![false]);
    let r = t.rect_of(a);
    t.click(r.x + 20, r.y + 20);
    let s = serial_in(&t.out(), a);
    t.msg(a, ClientMsg::PointerLock { window: 1, lock: true, gesture: s });
    assert_eq!(lock_states(&t.out(), a), vec![true]);
    assert_eq!(t.c.input.cursor, Cursor::Hidden);
    // Upozorneni systemu nahore.
    assert!(t.c.pointer_notice_rect().is_some());
    // Pohyb: relativni udalosti, kurzor stoji.
    let (px, py) = (t.c.input.px, t.c.input.py);
    t.input(Event::Motion { dx: 7, dy: -3 }, false);
    assert_eq!(relative(&t.out(), a), vec![(7, -3)]);
    assert_eq!((t.c.input.px, t.c.input.py), (px, py));
    // Super zamek zrusi (a aplikaci nejde).
    t.key(inputproto::code::KEY_LEFTMETA, false);
    let out = t.out();
    assert_eq!(lock_states(&out, a), vec![false]);
    assert!(t.c.plock.is_none());
    assert_ne!(t.c.input.cursor, Cursor::Hidden);
    // Po zruseni bezny pohyb.
    t.input(Event::Motion { dx: 5, dy: 0 }, false);
    assert!(relative(&t.out(), a).is_empty());
    assert_eq!(t.c.input.px, px + 5);
    let _ = g;
}

#[test]
fn pointer_lock_ends_with_focus_and_window() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    let b = t.client("b");
    t.window(b, Mode::Standard, BLUE);
    let r = t.rect_of(a);
    t.click(r.x + 20, r.y + 20);
    let s = serial_in(&t.out(), a);
    t.msg(a, ClientMsg::PointerLock { window: 1, lock: true, gesture: s });
    assert_eq!(lock_states(&t.out(), a), vec![true]);
    // Fokus jinam (napr. nove okno nebo prepnuti systemem): zamek konci.
    let gb = t.c.clients[&b].windows[&1];
    t.c.set_focus(Some(gb));
    assert_eq!(lock_states(&t.out(), a), vec![false]);
    // Okno bez fokusu zamek nedostane ani s gestem.
    t.msg(a, ClientMsg::PointerLock { window: 1, lock: true, gesture: s });
    assert_eq!(lock_states(&t.out(), a), vec![false]);
    // Zavreni okna se zamkem.
    let rb = t.rect_of(b);
    t.click(rb.x + 20, rb.y + 20);
    let sb = serial_in(&t.out(), b);
    t.msg(b, ClientMsg::PointerLock { window: 1, lock: true, gesture: sb });
    assert_eq!(lock_states(&t.out(), b), vec![true]);
    t.msg(b, ClientMsg::WindowDestroy { window: 1 });
    assert!(t.c.plock.is_none(), "zavrene okno zamek nedrzi");
}

#[test]
fn synthetic_input_cannot_lock() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    let r = t.rect_of(a);
    t.move_to(r.x + 20, r.y + 20);
    // Umely klik: bez cisla gesta.
    t.input(Event::Button { button: Button::Left, action: Action::Press }, true);
    t.input(Event::Button { button: Button::Left, action: Action::Release }, true);
    let s = serial_in(&t.out(), a);
    assert_eq!(s, 0, "umely vstup gesto nedava");
    t.msg(a, ClientMsg::PointerLock { window: 1, lock: true, gesture: s });
    assert_eq!(lock_states(&t.out(), a), vec![false]);
}

#[test]
fn recording_indicator_in_bar() {
    use crate::input::SysKey;
    use crate::tooltip::Tip;
    let mut t = T::new();
    t.c.set_volume(Some((60, false)));
    t.frame();
    let r = t.c.bar_layout().volume.expect("ikona zvuku");
    let before: Vec<u32> = (r.y..r.y + r.h as i32).flat_map(|y| (r.x..r.x + r.w as i32).map(move |x| (x, y))).map(|(x, y)| t.px(x, y)).collect();
    assert_eq!(t.c.tip_text(Tip::Bar(SysKey::BarIcon(1))), "Zvuk: hlasitost 60 %");
    // Nahravani: mikrofon v barve nebezpeci a jmena aplikaci v bublince.
    t.c.set_capturing(vec!["SDL test".into()]);
    t.frame();
    let danger = t.c.style.theme.color.danger;
    let after: Vec<u32> = (r.y..r.y + r.h as i32).flat_map(|y| (r.x..r.x + r.w as i32).map(move |x| (x, y))).map(|(x, y)| t.px(x, y)).collect();
    assert_ne!(before, after, "ikona se zmenila");
    assert!(after.iter().any(|&p| p == danger), "mikrofon v barve nebezpeci");
    assert_eq!(t.c.tip_text(Tip::Bar(SysKey::BarIcon(1))), "Nahrává zvuk: SDL test");
    // Konec nahravani: zpet ikona hlasitosti.
    t.c.set_capturing(Vec::new());
    t.frame();
    let back: Vec<u32> = (r.y..r.y + r.h as i32).flat_map(|y| (r.x..r.x + r.w as i32).map(move |x| (x, y))).map(|(x, y)| t.px(x, y)).collect();
    assert_eq!(before, back);
}
