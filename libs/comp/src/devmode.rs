//! Rezim vyvojare v liste: jen vyvojove sestaveni (feature `dev-mode`,
//! zapina ji xtask pro obrazy s vyvojovym profilem). Ve vydani tenhle
//! modul ani pilulka v liste nejsou.
//!
//! Stav urcuje sluzba uctu (`DialogRequest::DevMode` na kanalu `dialog`);
//! dokud ho neposle, pilulka neni. Zapnuto = trvaly vyrazny pruh
//! [`ON_LABEL`] (barva nebezpeci, vzdy s textem); vypnuto = nenapadna
//! pilulka [`OFF_LABEL`]. Klik otevre kartu s jedinym prepinacem; volba
//! odejde sluzbe uctu (`DialogReply::DevMode`), ktera rozhodne a posle
//! novy stav zpet. Lista sama nic nepovoluje.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gfx::Canvas;
use guiproto::Rect;
use guiproto::control::DialogReply;
use theme::TextStyle;

use crate::render::Bar;
use crate::{Comp, Out, PeerId};

/// Text pruhu pri zapnutem rezimu.
pub const ON_LABEL: &str = "Režim vývojáře - systém není chráněný";
/// Text pilulky pri vypnutem rezimu.
pub const OFF_LABEL: &str = "Vývojář";

/// Stav rezimu od sluzby uctu.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DevMode {
    pub on: bool,
    /// Kanal dialogu sluzby uctu (kam poslat prepnuti); `None` = stav
    /// jeste neprisel, pilulka se nekresli.
    pub peer: Option<PeerId>,
}

fn label_style(t: &theme::Theme) -> TextStyle {
    TextStyle { weight: t.text.body_strong.weight, ..t.text.small }
}

impl Comp {
    /// Stav od sluzby uctu (`DialogRequest::DevMode`).
    pub(crate) fn dev_mode_state(&mut self, peer: PeerId, on: bool) {
        let next = DevMode { on, peer: Some(peer) };
        if self.dev_mode != next {
            self.dev_mode = next;
            // Otevreny panel ukaze novy stav (prepinac, varovani).
            self.bar_panel_changed();
            self.damage_panel();
        }
    }

    /// Je rezim vyvojare zapnuty (podle posledni zpravy sluzby uctu)?
    pub fn dev_mode_on(&self) -> bool {
        self.dev_mode.on
    }

    fn dev_label(&self) -> &'static str {
        if self.dev_mode.on { ON_LABEL } else { OFF_LABEL }
    }

    /// Sirka pilulky v liste.
    pub(crate) fn dev_chip_width(&self) -> u32 {
        let t = &self.style.theme;
        t.space[4] + t.size.icon_s + t.space[2] + self.style.width(self.dev_label(), label_style(t)) + t.space[4]
    }

    /// Pilulka v liste (kresli se nad listou, `render::draw_top`).
    pub(crate) fn draw_dev_chip(&mut self, c: &mut Canvas<'_>, bar: &Bar) {
        let Some(r) = bar.dev else { return };
        let t = self.style.theme.clone();
        let col = &t.color;
        let (ink, label) = if self.dev_mode.on {
            c.round_fill_bordered(r, r.h / 2, col.danger, col.danger, 1);
            (col.on_danger, ON_LABEL)
        } else {
            c.round_fill_bordered(r, r.h / 2, col.surface, col.border_strong, 1);
            (col.ink_muted, OFF_LABEL)
        };
        // Ikona pred textem: zapnuto stit s vykricnikem, jinak kod.
        let is = t.size.icon_s;
        let icon = if self.dev_mode.on { "shield-alert" } else { "code" };
        self.style.icons.draw(c, icon, r.x + t.space[4] as i32, r.y + (r.h as i32 - is as i32) / 2, is, t.icon_stroke, ink);
        let s = label_style(&t);
        let face = self.style.face(s);
        let base = r.y + face.baseline_in(s.size, r.h);
        face.draw(c, r.x + (t.space[4] + is + t.space[2]) as i32, base, label, s.size, ink);
    }

    /// Karta nad pilulkou: prvni radek je prepinac, zbytek vysvetleni.
    pub(crate) fn dev_flyout_text(&self) -> (String, Vec<String>) {
        let rows: Vec<&str> = if self.dev_mode.on {
            vec![
                "Vypnout režim vývojáře",
                "Úkony správy se teď potvrzují samy, bez dialogu.",
                "Každý se zapisuje do auditu.",
            ]
        } else {
            vec![
                "Zapnout režim vývojáře",
                "Úkony správy se pak potvrdí samy, bez dialogu.",
                "Jen vývojové sestavení; vydání tuto volbu nemá.",
            ]
        };
        (String::from("Režim vývojáře"), rows.into_iter().map(String::from).collect())
    }

    /// Klik na radek `row` karty: prvni radek prepne (rozhodne sluzba uctu).
    pub(crate) fn dev_flyout_click(&mut self, row: usize) {
        if row != 0 {
            return;
        }
        let Some(peer) = self.dev_mode.peer else { return };
        self.out.push(Out::Dialog(peer, DialogReply::DevMode { on: !self.dev_mode.on }));
        self.set_flyout(None);
    }

    /// Rozlozeni listy: pilulka vpravo pred ostatnimi (`right` = volny
    /// okraj zprava, vraci novy).
    pub(crate) fn dev_chip_layout(&self, bar: &mut Bar, right: i32, item: u32, gap: i32) -> i32 {
        if self.dev_mode.peer.is_none() {
            return right;
        }
        let t = &self.style.theme;
        let h = t.size.control_h.min(item);
        let w = self.dev_chip_width();
        let x = right - w as i32;
        bar.dev = Some(Rect::new(x, bar.rect.y + (bar.rect.h as i32 - h as i32) / 2, w, h));
        x - gap
    }
}
