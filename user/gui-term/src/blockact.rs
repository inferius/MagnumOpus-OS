//! Akce bloku (`design/terminal.html`, `.blk-act`): u bloku pod
//! ukazatelem se v hlavicce ukazou tlacitka Sbalit / Rozbalit, Kopirovat
//! vystup (schranka) a Spustit znovu (cas bloku se posune vlevo);
//! porovnani s predchozim behem je navrzene zvlast.
//! V pristupnosti jsou akce u vsech viditelnych bloku.

use alloc::vec::Vec;

use gfx::{Canvas, Rect};
use term::ui::draw::{Line, Role};

use crate::Look;
use crate::rows::Grid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Fold,
    Copy,
    Again,
}

/// Poradi tlacitek v hlavicce bloku.
const ACTS: [Act; 3] = [Act::Fold, Act::Copy, Act::Again];

/// Tlacitko akce: plocha, blok a akce.
pub struct Button {
    pub rect: Rect,
    pub block: u64,
    pub act: Act,
}

/// Blok, ke kteremu patri radek.
pub fn block_of(r: &Role) -> Option<u64> {
    match r {
        Role::Head { block, .. } | Role::Body { block, .. } => Some(*block),
        _ => None,
    }
}

/// Sirka tlacitek akci (o tolik se posune cas bloku).
pub fn width(look: &Look, g: &Grid) -> u32 {
    ACTS.len() as u32 * g.ch + (ACTS.len() as u32 - 1) * look.t.space[1]
}

/// Tlacitka v hlavickach bloku, pro ktere `pick` vrati `true`.
pub fn layout(look: &Look, lines: &[Line], g: &Grid, pick: impl Fn(u64) -> bool) -> Vec<Button> {
    let mut out = Vec::new();
    let gap = look.t.space[1] as i32;
    for (row, l) in lines.iter().enumerate() {
        let Role::Head { block, .. } = l.role else { continue };
        if !pick(block) {
            continue;
        }
        let y = g.y + (row as u32 * g.ch) as i32;
        let x = g.x + (g.cols as u32 * g.cw) as i32 - width(look, g) as i32;
        for (k, act) in ACTS.into_iter().enumerate() {
            out.push(Button { rect: Rect::new(x + k as i32 * (g.ch as i32 + gap), y, g.ch, g.ch), block, act });
        }
    }
    out
}

/// Nakresli tlacitka (`collapsed` = sbalene bloky: ikona doprava).
pub fn paint(c: &mut Canvas<'_>, look: &mut Look, buttons: &[Button], collapsed: &[u64]) {
    let t = look.t.clone();
    let col = &t.color;
    for b in buttons {
        let r = b.rect;
        c.round_fill(r, t.radius.xs, col.surface_3);
        let icon = match b.act {
            Act::Fold if collapsed.contains(&b.block) => "right",
            Act::Fold => "down",
            Act::Copy => "copy",
            Act::Again => "refresh",
        };
        let is = t.size.icon_s.min(r.h.saturating_sub(4));
        look.icons.draw(c, icon, r.x + (r.w - is) as i32 / 2, r.y + (r.h - is) as i32 / 2, is, t.icon_stroke, col.term_ink);
    }
}
