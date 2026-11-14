//! Najeti ukazatele: zvyrazneni se presouva s mysi pres vic polozek
//! (i uvnitr jednoho prvku - zalozky, radky tabulky) a po odjeti zmizi.
//!
//! Smycka jako v klientovi: snimek se prekresli jen pri `redraw`. Chyba,
//! kterou test hlida: pohyb ze zalozky na zalozku nemenil prvek pod
//! ukazatelem, takze se neprekreslilo a zvyraznena zustala prvni zalozka.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gfx::Canvas;
use guiproto::a11y::Role;
use guiproto::window::InputEvent;

use crate::*;

#[derive(Clone, Debug, PartialEq)]
enum M {
    Tab(usize),
    Ok,
    Cancel,
}

const W: u32 = 500;
const H: u32 = 400;

struct F {
    ui: Ui<M>,
    px: Vec<u32>,
}

impl F {
    fn new() -> Self {
        let mut f = F { ui: Ui::new(crate::Look::ui_fonts(), W, H), px: vec![0; (W * H) as usize] };
        let labels: Vec<String> = ["Pole", "Volby", "Tabulka", "Nabídky", "Grafy"].iter().map(|s| (*s).into()).collect();
        let t = table(vec![col("Název", 0)], 20, |i| i as u64, |i| Cells::new([alloc::format!("řádek {i}")])).key("t");
        let v = column([tabs(labels, 0).on_pick(M::Tab).key("tabs"), row([button("OK").on_click(M::Ok).key("ok"), button("Zrušit").on_click(M::Cancel).key("no")]), t.height(200)]);
        f.ui.rebuild("h".into(), v);
        f.draw();
        f
    }

    fn draw(&mut self) {
        let mut c = Canvas::new(&mut self.px, W, H, W).unwrap();
        self.ui.draw(&mut c, 0);
    }

    /// Udalost; prekresli jen pri `redraw` (jako smycka klienta).
    fn ev(&mut self, e: InputEvent) {
        if self.ui.event(e, 0).redraw {
            self.draw();
        }
    }

    fn mv(&mut self, (x, y): (i32, i32)) {
        self.ev(InputEvent::PointerMove { x, y });
    }

    fn at(&self, (x, y): (i32, i32)) -> u32 {
        self.px[(y as u32 * W + x as u32) as usize]
    }

    fn bounds(&self, role: Role, name: &str) -> Rect {
        self.ui.a11y().into_iter().find(|n| n.role == role && n.name == name).unwrap().bounds
    }

    /// Bod na pozadi zalozky (vlevo pred textem, svisle uprostred).
    fn tab_pt(&self, name: &str) -> (i32, i32) {
        let b = self.bounds(Role::Tab, name);
        (b.x + 3, b.y + b.h as i32 / 2)
    }

    /// Bod na pozadi tlacitka (vlevo pred textem).
    fn button_pt(&self, name: &str) -> (i32, i32) {
        let b = self.bounds(Role::Button, name);
        (b.x + 4, b.y + b.h as i32 / 2)
    }

    /// Bod v radku `k` tabulky (bez zahlavi, v pravem okraji bez textu).
    fn row_pt(&self, k: usize) -> (i32, i32) {
        let n = self.ui.a11y().into_iter().find(|n| n.role == Role::Table).unwrap().bounds;
        let rh = self.ui.look.row_h() as i32;
        (n.x + n.w as i32 / 2, n.y + 1 + (k as i32 + 1) * rh + rh / 2)
    }

    fn hi(&self) -> u32 {
        self.ui.look.theme.color.surface_3
    }
}

/// Zalozky: najeti se presouva ze zalozky na zalozku (jeden prvek)
/// a po odjeti mimo i po opusteni okna zmizi.
#[test]
fn hover_moves_across_tabs_and_leaves() {
    let mut f = F::new();
    let names = ["Volby", "Tabulka", "Nabídky", "Grafy"];
    let hi = f.hi();
    // Zadna neni zvyraznena (vybrana je "Pole", ta ma vlastni pozadi).
    for n in names {
        assert_ne!(f.at(f.tab_pt(n)), hi, "{n} bez najeti");
    }
    // Tam a zpet pres vsechny nevybrane zalozky.
    let order: Vec<&str> = names.iter().chain(names.iter().rev()).copied().collect();
    for cur in order {
        f.mv(f.tab_pt(cur));
        for n in names {
            assert_eq!(f.at(f.tab_pt(n)) == hi, n == cur, "ukazatel nad {cur}, zalozka {n}");
        }
    }
    // Na vybranou zalozku: zadna nevybrana neni zvyraznena.
    f.mv(f.tab_pt("Grafy"));
    f.mv(f.tab_pt("Pole"));
    for n in names {
        assert_ne!(f.at(f.tab_pt(n)), hi, "nad vybranou, {n}");
    }
    // Odjeti mimo prvky.
    f.mv(f.tab_pt("Nabídky"));
    f.mv((W as i32 - 2, H as i32 - 2));
    for n in names {
        assert_ne!(f.at(f.tab_pt(n)), hi, "mimo, {n}");
    }
    // Opusteni okna.
    f.mv(f.tab_pt("Volby"));
    f.ev(InputEvent::PointerLeave);
    assert_ne!(f.at(f.tab_pt("Volby")), hi, "po opusteni okna");
}

/// Tlacitka: najeti z jednoho na druhe a pryc.
#[test]
fn hover_moves_across_buttons() {
    let mut f = F::new();
    let hi = f.hi();
    f.mv(f.button_pt("OK"));
    assert_eq!(f.at(f.button_pt("OK")), hi);
    assert_ne!(f.at(f.button_pt("Zrušit")), hi);
    f.mv(f.button_pt("Zrušit"));
    assert_ne!(f.at(f.button_pt("OK")), hi);
    assert_eq!(f.at(f.button_pt("Zrušit")), hi);
    f.ev(InputEvent::PointerLeave);
    assert_ne!(f.at(f.button_pt("Zrušit")), hi);
}

/// Tabulka: najeti se presouva z radku na radek (jeden prvek) a po
/// odjeti zmizi.
#[test]
fn hover_moves_across_table_rows() {
    let mut f = F::new();
    let hi = f.hi();
    for cur in [0usize, 1, 2, 3, 2, 0] {
        f.mv(f.row_pt(cur));
        for k in 0..4 {
            assert_eq!(f.at(f.row_pt(k)) == hi, k == cur, "ukazatel nad radkem {cur}, radek {k}");
        }
    }
    f.mv(f.tab_pt("Volby"));
    for k in 0..4 {
        assert_ne!(f.at(f.row_pt(k)), hi, "po odjeti, radek {k}");
    }
}
