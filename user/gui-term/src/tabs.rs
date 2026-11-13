//! Karty relaci v titulku okna (`design/terminal.html`, `.tabs`): za
//! odznakem systemu karta kazde relace sluzby (aktualni zvyraznena,
//! ostatni se stitkem "odpojena") a tlacitko + pro novou relaci. Klik
//! nebo ACTIVATE v pristupnosti prepne relaci (`App::switch_session`).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::{Canvas, Rect};
use term::ui::sessions::SessionInfo;

use crate::Look;

/// Karta v titulku: relace (`0` = tlacitko nove relace) a jeji plocha.
pub struct Tab {
    pub id: u64,
    pub rect: Rect,
    pub current: bool,
}

/// Popisek karty.
pub fn label(id: u64) -> String {
    format!("Relace {id}")
}

/// Rozlozeni karet: od mista za odznakem az po `limit` (zacatek
/// prepinace zapisu). Aktualni relace je videt vzdy (kdyz se vsechny
/// nevejdou, nahradi posledni viditelnou).
pub fn layout(look: &Look, sessions: &[SessionInfo], current: u64, limit: i32) -> Vec<Tab> {
    let t = &look.t;
    let sp = &t.space;
    let first = look.tab();
    let (w, h, y) = (first.w, first.h, first.y);
    let plus = t.size.hit_min;
    let mut out = Vec::new();
    let mut x = first.x;
    // Kolik karet se vejde (s tlacitkem +).
    let fit = ((limit - x - plus as i32).max(0) as u32 / (w + sp[1])) as usize;
    let mut shown: Vec<&SessionInfo> = sessions.iter().take(fit.max(1)).collect();
    if !shown.iter().any(|s| s.id == current)
        && let Some(cur) = sessions.iter().find(|s| s.id == current)
    {
        shown.pop();
        shown.push(cur);
    }
    for s in shown {
        out.push(Tab { id: s.id, rect: Rect::new(x, y, w, h), current: s.id == current });
        x += (w + sp[1]) as i32;
    }
    let bh = t.size.control_h_compact;
    out.push(Tab { id: 0, rect: Rect::new(x, (look.title_h().saturating_sub(bh) / 2) as i32, plus, bh), current: false });
    out
}

/// Nakresli karty (aktualni s plochou `surface_2` jako dosud, ostatni
/// tlumene se stitkem "odpojena", tlacitko + jako ikona).
pub fn paint(c: &mut Canvas<'_>, look: &mut Look, tabs: &[Tab]) {
    let t = look.t.clone();
    let (sp, col) = (&t.space, &t.color);
    let rad = t.radius.control;
    let s = t.text.small;
    let is = t.size.icon_s;
    for tab in tabs {
        let r = tab.rect;
        if tab.id == 0 {
            look.icons.draw(c, "plus", r.x + (r.w - is) as i32 / 2, r.y + (r.h - is) as i32 / 2, is, t.icon_stroke, col.ink_muted);
            continue;
        }
        if tab.current {
            let old = c.clip();
            c.set_clip(old.intersect(&r));
            c.round_fill(Rect::new(r.x, r.y, r.w, r.h + rad), rad, col.surface_2);
            c.set_clip(old);
        }
        let mut x = r.x + sp[4] as i32;
        if tab.current {
            look.icons.draw(c, "terminal", x, r.y + (r.h - is) as i32 / 2, is, t.icon_stroke, col.ink);
            x += (is + sp[2]) as i32;
        }
        let name = label(tab.id);
        let f = look.ui.face(t.text.body_strong.weight);
        let base = r.y + f.baseline_in(s.size, r.h);
        let room = (r.x + r.w as i32 - sp[3] as i32 - x).max(0) as u32;
        let ink = if tab.current { col.ink } else { col.ink_muted };
        let tw = f.width(&name, s.size).min(room);
        f.draw_ellipsized(c, x, base, &name, s.size, ink, room);
        if !tab.current {
            // Stitek "odpojena": relace bezi dal bez okna.
            let cs = t.text.caption;
            let chip = "odpojená";
            let fc = look.ui.face(cs.weight);
            let cw = fc.width(chip, cs.size) + 2 * sp[2];
            let cx = x + (tw + sp[2]) as i32;
            if cx + cw as i32 <= r.x + r.w as i32 - sp[2] as i32 {
                let ch = cs.line;
                let cr = Rect::new(cx, r.y + (r.h - ch) as i32 / 2, cw, ch);
                c.round_fill(cr, ch / 2, col.surface_3);
                let b = cr.y + fc.baseline_in(cs.size, ch);
                fc.draw(c, cr.x + sp[2] as i32, b, chip, cs.size, col.ink_muted);
            }
        }
    }
}
