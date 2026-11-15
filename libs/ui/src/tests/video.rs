//! Plocha videa: rezervace mista v rozvrzeni a obdelnik obrazu se
//! zachovanim pomeru stran (vrstvu a buffery dela klient).

use guiproto::Rect;

use crate::layout::{ElemKind, layout};
use crate::video::fit;
use crate::*;

#[test]
fn fit_keeps_aspect_and_centers() {
    // Sirsi plocha nez 16:9: pruhy vlevo a vpravo.
    assert_eq!(fit(Rect::new(0, 0, 800, 360), 640, 360), Rect::new(80, 0, 640, 360));
    // Uzsi: pruhy nahore a dole, zmenseni.
    assert_eq!(fit(Rect::new(10, 20, 320, 400), 640, 360), Rect::new(10, 20 + 110, 320, 180));
    // Nulovy rozmer nic neukaze.
    assert_eq!(fit(Rect::new(5, 5, 0, 100), 640, 360).w, 0);
    assert_eq!(fit(Rect::new(5, 5, 100, 100), 0, 360).w, 0);
}

#[test]
fn video_node_reserves_space() {
    let look = Look::new(Look::ui_fonts(), theme::Mode::Light);
    let root: Node<()> = column([video(2, 640, 360, "Video").grow(), button("Přehrát")]);
    let els = layout(root, Rect::new(0, 0, 680, 470), &look);
    let v = els.iter().find(|e| matches!(e.kind, ElemKind::Video { id: 2, .. })).expect("prvek videa");
    assert!(v.rect.w >= 640 && v.rect.h >= 300, "plocha videa roste: {:?}", v.rect);
    let f = fit(v.rect, 640, 360);
    assert!((f.w * 9).abs_diff(f.h * 16) <= 16, "pomer stran 16:9: {f:?}");
}
