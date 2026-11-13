//! Svisly posuvnik nezavisly na stromu prvku: geometrie, zasah, tazeni
//! a kresleni. Pouziva ho tabulka a aplikace, ktere kresli samy (vystup
//! terminalu).
//!
//! Posuvnik je prekryvny (nad obsahem u praveho okraje), sirka
//! `size.scrollbar`; jezdec ma nejmene `size.hit_min / 2`, aby sel chytit.
//! Klik do drahy posune o stranku, tazeni jezdce plynule.

use gfx::{Canvas, Rect};
use theme::Theme;

/// Spocitana geometrie posuvniku.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scrollbar {
    /// Draha (cela vyska oblasti u praveho okraje).
    pub track: Rect,
    pub thumb: Rect,
    /// Nejvetsi posun obsahu.
    pub max: i32,
}

/// Kam mira klik.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// Jezdec: `grab` = vzdalenost od jeho horniho okraje.
    Thumb { grab: i32 },
    /// Draha nad jezdcem (stranka nahoru) nebo pod nim (dolu).
    PageUp,
    PageDown,
}

impl Scrollbar {
    /// Posuvnik oblasti `area` s obsahem vysky `content` posunutym
    /// o `offset`; `None`, kdyz se obsah vejde.
    pub fn new(area: Rect, content: u32, offset: i32, t: &Theme) -> Option<Self> {
        let view = area.h;
        if content <= view || view == 0 {
            return None;
        }
        let w = t.size.scrollbar;
        let track = Rect::new(area.right() as i32 - w as i32 - 2, area.y + 2, w, view.saturating_sub(4));
        let max = (content - view) as i32;
        let th = ((track.h as u64 * view as u64 / content as u64) as u32).clamp((t.size.hit_min / 2).min(track.h), track.h);
        let room = (track.h - th) as i64;
        let y = track.y + (room * offset.clamp(0, max) as i64 / max as i64) as i32;
        Some(Self { track, thumb: Rect::new(track.x, y, w, th), max })
    }

    /// Zasah bodu (souradnice oblasti) do posuvniku; o kousek sirsi
    /// nez kresba (snazsi trefa).
    pub fn hit(&self, x: i32, y: i32) -> Option<Hit> {
        let wide = Rect::new(self.track.x - 4, self.track.y, self.track.w + 6, self.track.h);
        if !wide.contains(x, y) {
            return None;
        }
        Some(if y < self.thumb.y {
            Hit::PageUp
        } else if y >= self.thumb.bottom() as i32 {
            Hit::PageDown
        } else {
            Hit::Thumb { grab: y - self.thumb.y }
        })
    }

    /// Posun obsahu pro jezdec tazeny tak, ze bod `y` je `grab` pod jeho
    /// hornim okrajem.
    pub fn offset_for(&self, y: i32, grab: i32) -> i32 {
        let room = (self.track.h - self.thumb.h) as i64;
        if room <= 0 {
            return 0;
        }
        let top = (y - grab - self.track.y).clamp(0, room as i32) as i64;
        (top * self.max as i64 / room) as i32
    }

    /// Nakresli posuvnik (jezdec; draha jen pri najeti nebo tazeni).
    pub fn draw(&self, c: &mut Canvas<'_>, t: &Theme, hover: bool, active: bool) {
        let col = &t.color;
        let r = self.track.w / 2;
        if hover || active {
            c.round_fill(self.track, r, col.surface_3);
        }
        let ink = if active { col.ink_muted } else if hover { col.ink_faint } else { col.border_strong };
        c.round_fill(self.thumb, r, ink);
    }
}
