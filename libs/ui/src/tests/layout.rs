//! Rozvrzeni: natazeni kontejneru v radku (`layout::stretches`).

use alloc::vec::Vec;

use guiproto::Rect;

use crate::layout::{ElemKind, layout};
use crate::*;

fn look() -> Look {
    Look::new(Look::ui_fonts(), theme::Mode::Light)
}

/// Obdelniky primych deti radku (rodic = prvni prvek, index 1).
fn row_children(root: Node<()>, area: Rect) -> Vec<Rect> {
    let els = layout(root, area, &look());
    els.iter().filter(|e| e.parent == 1).map(|e| e.rect).collect()
}

#[test]
fn row_stretches_columns_to_full_height() {
    let area = Rect::new(10, 20, 600, 400);
    let root: Node<()> = row([column([label("Místa")]).width(150), column([label("Obsah")]).grow()]);
    let r = row_children(root, area);
    assert_eq!(r.len(), 2);
    for c in &r {
        assert_eq!((c.y, c.h), (area.y, area.h), "sloupec od vrchu radku na celou vysku: {c:?}");
    }
    assert!(r[1].w > r[0].w, "grow bere zbytek sirky");
}

#[test]
fn row_keeps_leaves_and_fixed_height_centered() {
    let area = Rect::new(0, 0, 600, 300);
    let root: Node<()> = row([button("OK"), column([label("x")]).height(100), scroll(label("y")), list(Vec::new(), None)]);
    let r = row_children(root, area);
    assert_eq!(r.len(), 4);
    assert!(r[0].h < area.h && r[0].y > 0, "tlacitko na stredu: {:?}", r[0]);
    assert_eq!((r[1].h, r[1].y), (100, 100), "pevna vyska ma prednost, na stredu");
    assert_eq!((r[2].y, r[2].h), (0, 300), "posuv natazeny");
    assert_eq!((r[3].y, r[3].h), (0, 300), "seznam natazeny");
    // Prvek uvnitr natazeneho sloupce zacina nahore.
    let els = layout::<()>(row([column([label("nahore")])]), area, &look());
    let lbl = els.iter().find(|e| matches!(e.kind, ElemKind::Label { .. })).unwrap();
    assert_eq!(lbl.rect.y, 0);
}
