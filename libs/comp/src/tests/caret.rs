//! Mala vrstva bez potomku (kurzor textoveho pole): zmena jen viditelnosti
//! nebo polohy poskodi jen jeji stare a nove misto, ne cele okno.

use super::*;

/// Okno s vrstvou obsahu a vrstvou kurzoru 2x20 (buffer) na (50, 30).
fn setup() -> (T, ClientId, Rect) {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.c.client_msg(a, ClientMsg::BufferCreate { buffer: 7, w: 2, h: 20, stride: 2 }, Some(Box::new(vec![GREEN; 40])), t.now);
    t.msg(a, ClientMsg::LayerCreate { layer: 3, window: 1, parent: 0, kind: LayerKind::Buffer });
    t.msg(a, ClientMsg::LayerSet { layer: 3, props: LayerProps { mask: prop::BUFFER | prop::RECT | prop::ORDER, rect: Rect::new(50, 30, 2, 20), buffer: 7, order: 5, ..Default::default() } });
    t.commit(a, 2, 0, 0, 0);
    t.frame();
    let r = t.rect_of(a);
    (t, a, r)
}

fn area(rs: &[Rect]) -> u64 {
    rs.iter().map(|r| r.w as u64 * r.h as u64).sum()
}

#[test]
fn blink_damages_only_the_caret() {
    let (mut t, a, r) = setup();
    let (cx, cy) = (r.x + 50, r.y + 30);
    assert_eq!(t.px(cx, cy + 5), GREEN);
    // Skryti: poskozeny jen obdelnik kurzoru.
    t.msg(a, ClientMsg::LayerSet { layer: 3, props: LayerProps { mask: prop::VISIBLE, visible: false, ..Default::default() } });
    t.commit(a, 3, 0, 0, 0);
    let d = t.frame().expect("snimek");
    assert!(area(&d) <= 2 * 20 * 4, "poskozeni {d:?} je vetsi nez kurzor");
    assert!(d.iter().all(|x| x.intersect(&Rect::new(cx, cy, 2, 20)) == *x), "poskozeni mimo kurzor: {d:?}");
    assert_eq!(t.px(cx, cy + 5), RED, "kurzor zmizel");
    // Zobrazeni a posun: stare i nove misto, porad male.
    t.msg(a, ClientMsg::LayerSet { layer: 3, props: LayerProps { mask: prop::VISIBLE | prop::RECT, visible: true, rect: Rect::new(80, 30, 2, 20), ..Default::default() } });
    t.commit(a, 4, 0, 0, 0);
    let d = t.frame().expect("snimek");
    assert!(area(&d) <= 2 * 2 * 20 * 4, "{d:?}");
    assert_eq!(t.px(r.x + 80, cy + 5), GREEN);
    assert_eq!(t.px(cx, cy + 5), RED);
    // Zmena obsahu okna (barva korene) dal poskodi cele okno.
    t.set_color(a, BLUE);
    t.commit(a, 5, 0, 0, 0);
    let d = t.frame().expect("snimek");
    assert!(area(&d) >= (r.w * r.h) as u64, "cele okno");
}
