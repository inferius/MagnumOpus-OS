//! Popover okna (vrstva s `prop::POPOVER`): kresli se nad okny mimo
//! obdelnik okna, vstup v nem patri oknu, poloha okna jde aplikaci.

use super::*;

#[test]
fn popover_outside_window_draws_and_gets_input() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.frame();
    let r = t.rect_of(a);
    // Pod oknem (mimo jeho obdelnik), 120 x 60.
    let pop = Rect::new(-40, r.h as i32 + 10, 120, 60);
    t.msg(a, ClientMsg::LayerCreate { layer: 5, window: 1, parent: 0, kind: LayerKind::Solid });
    t.msg(a, ClientMsg::LayerSet { layer: 5, props: LayerProps { mask: prop::RECT | prop::COLOR | prop::POPOVER, rect: pop, color: GREEN, popover: true, ..Default::default() } });
    t.commit(a, 2, 0, 0, 0);
    t.frame();
    let (px, py) = (r.x + pop.x + 60, r.y + pop.y + 30);
    assert!(!r.contains(px, py), "bod je mimo okno");
    assert_eq!(t.px(px, py), GREEN, "popover nad plochou mimo okno");
    // Vstup v plose popoveru: okno a, souradnice obsahu (mimo jeho obdelnik).
    let g = t.c.clients[&a].windows[&1];
    assert!(matches!(t.c.hit(px, py), Some((w, crate::input::Zone::Content(x, y))) if w == g && x == pop.x + 60 && y == pop.y + 30));
    // Skryti: stare misto se prekresli (zadna zelena).
    t.msg(a, ClientMsg::LayerSet { layer: 5, props: LayerProps { mask: prop::VISIBLE, visible: false, ..Default::default() } });
    t.commit(a, 3, 0, 0, 0);
    t.frame();
    assert_ne!(t.px(px, py), GREEN);
    assert!(t.c.hit(px, py).is_none_or(|(w, _)| w != g));
}

/// `WINDOW_PLACE`: po zobrazeni a po presunu (ne behem tahu) obdelnik
/// obsahu na obrazovce.
#[test]
fn window_place_after_map_and_move() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.frame();
    let r = t.rect_of(a);
    assert!(client_msgs(&t.out(), a).contains(&ServerMsg::WindowPlace { window: 1, rect: r }));
    t.frame();
    assert!(!client_msgs(&t.out(), a).iter().any(|m| matches!(m, ServerMsg::WindowPlace { .. })), "beze zmeny nic");
    let g = t.c.clients[&a].windows[&1];
    t.c.windows.get_mut(&g).unwrap().rect.x += 30;
    t.frame();
    assert!(client_msgs(&t.out(), a).contains(&ServerMsg::WindowPlace { window: 1, rect: Rect::new(r.x + 30, r.y, r.w, r.h) }));
}
