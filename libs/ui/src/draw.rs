//! Kresleni prvku do bufferu (jeden prostor: okno nebo posouvany obsah).
//!
//! Vzhled podle `design/design-system.html`: tlacitka jako pilulky
//! s ramem `border_strong`, pole a seznamy se zaoblenim `radius.control`,
//! fokus jako ramecek `size.focus_ring` s odsazenim `size.focus_offset`
//! barvou `focus`. Vse z tokenu ([`crate::look::Look`]).

use gfx::{Canvas, Rect};
use theme::TextStyle;

use crate::Ui;
use crate::layout::ElemKind;

/// Barva `c` s kryti `k`/255 pres pozadi `bg` (nedostupny prvek: cely
/// prvek s kryti `opacity.disabled`).
pub(crate) fn fade(c: u32, bg: u32, k: u32) -> u32 {
    let m = |sh: u32| ((c >> sh & 0xFF) * k + (bg >> sh & 0xFF) * (255 - k)) / 255;
    0xFF00_0000 | m(16) << 16 | m(8) << 8 | m(0)
}

/// Stitek agenta: pilulka `ai_soft`, ikona agenta (dve jiskry) a text
/// `ai_ink`. Vraci sirku.
#[allow(clippy::too_many_arguments)]
fn agent_chip(c: &mut Canvas<'_>, look: &mut crate::Look, icons: &mut gfx::icon::Icons, x: i32, y: i32, h: u32, text: &str) -> u32 {
    let th = look.theme.clone();
    let (sp, col) = (&th.space, &th.color);
    let s = look.chip();
    let is = th.size.icon_xs;
    let w = sp[3] + is + sp[1] + look.width(text, s) + sp[3];
    let r = Rect::new(x, y, w, h);
    c.round_fill_bordered(r, h / 2, col.ai_soft, col.ai, 1);
    icons.draw(c, "agent", x + sp[3] as i32, y + (h - is) as i32 / 2, is, th.icon_stroke, col.ai_ink);
    let f = look.face(s);
    let b = y + f.baseline_in(s.size, h);
    f.draw(c, x + (sp[3] + is + sp[1]) as i32, b, text, s.size, col.ai_ink);
    w
}

/// Ramecek fokusu kolem `r` (vne prvku, zaobleni o odsazeni vetsi).
pub(crate) fn focus_ring(c: &mut Canvas<'_>, r: Rect, radius: u32, ring: u32, offset: u32, color: u32) {
    let e = ring + offset;
    let o = Rect::new(r.x - e as i32, r.y - e as i32, r.w + 2 * e, r.h + 2 * e);
    c.round_stroke(o, radius + e, ring, color);
}

impl<M: Clone> Ui<M> {
    /// Nakresli prvky prostoru `space` (0 = okno, jinak index oblasti + 1)
    /// do platna; pocatek platna = pocatek prostoru.
    pub fn draw(&mut self, c: &mut Canvas<'_>, space: usize) {
        let th = self.look.theme.clone();
        let col = &th.color;
        let sp = &th.space;
        let (ring, ring_off) = (th.size.focus_ring, th.size.focus_offset);
        c.fill(c.bounds(), col.surface);
        for i in 0..self.elems.len() {
            let e = &self.elems[i];
            if e.space != space {
                continue;
            }
            let r = e.rect;
            // Jednotny stav (`Ui::state`): ramecek fokusu jen v aktivnim
            // okne, stisknuto jen s ukazatelem nad prvkem.
            let st = self.state(i);
            let (focused, hover, pressed, dim) = (st.focus_visible, st.hover, st.pressed, st.disabled);
            match &e.kind {
                ElemKind::Label { text, heading, warn } => {
                    let s = if *heading { self.look.heading() } else { self.look.body() };
                    let old = c.clip();
                    c.set_clip(old.intersect(&r));
                    if e.agent {
                        let h = r.h.min(s.line + 2 * sp[2]);
                        agent_chip(c, &mut self.look, &mut self.icons, r.x, r.y + (r.h - h) as i32 / 2, h, text);
                    } else if *warn {
                        let tw = self.look.width(text, s) + 2 * sp[4];
                        let chip = Rect::new(r.x, r.y, tw.min(r.w), r.h);
                        c.round_fill(chip, chip.h / 2, col.warn_soft);
                        let f = self.look.face(s);
                        let b = chip.y + f.baseline_in(s.size, chip.h);
                        f.draw(c, chip.x + sp[4] as i32, b, text, s.size, col.warn);
                    } else {
                        let f = self.look.face(s);
                        let b = r.y + f.baseline_in(s.size, r.h);
                        f.draw(c, r.x, b, text, s.size, col.ink);
                    }
                    c.set_clip(old);
                }
                ElemKind::Button { text, .. } => {
                    let bg = if (pressed || hover) && !dim { col.surface_3 } else { col.surface };
                    // Nedostupne: stejne tlacitko s kryti `opacity.disabled`.
                    let k = if dim { th.opacity.disabled } else { 255 };
                    c.round_fill_bordered(r, r.h / 2, fade(bg, col.surface, k), fade(col.border_strong, col.surface, k), 1);
                    if focused && !dim {
                        focus_ring(c, r, r.h / 2, ring, ring_off, col.focus);
                    }
                    let s = self.look.button();
                    let f = self.look.face(s);
                    let tw = f.width(text, s.size) as i32;
                    let fg = fade(col.ink, col.surface, k);
                    let b = r.y + f.baseline_in(s.size, r.h);
                    f.draw(c, r.x + (r.w as i32 - tw) / 2, b, text, s.size, fg);
                }
                ElemKind::TextInput { .. } => self.draw_field(c, i, st),
                ElemKind::Checkbox { text, checked, .. } => {
                    let bs = th.size.icon_m;
                    let b = Rect::new(r.x, r.y + (r.h as i32 - bs as i32) / 2, bs, bs);
                    if *checked {
                        c.round_fill(b, th.radius.xs, col.accent);
                        let is = th.size.icon_s;
                        self.icons.draw(c, "check", b.x + (bs - is) as i32 / 2, b.y + (bs - is) as i32 / 2, is, th.icon_stroke, col.on_accent);
                    } else {
                        c.round_fill_bordered(b, th.radius.xs, col.surface, col.border_strong, 1);
                    }
                    if focused {
                        focus_ring(c, b, th.radius.xs, ring, ring_off, col.focus);
                    }
                    let s = self.look.body();
                    let f = self.look.face(s);
                    let base = r.y + f.baseline_in(s.size, r.h);
                    f.draw(c, r.x + (bs + sp[3]) as i32, base, text, s.size, col.ink);
                }
                ElemKind::List { .. } => {
                    c.round_fill_bordered(r, th.radius.card, col.surface, col.border, 1);
                    if focused {
                        focus_ring(c, r, th.radius.card, ring, ring_off, col.focus);
                    }
                }
                ElemKind::ListItem { text, selected, agent, .. } => {
                    let rad = th.radius.control;
                    // Vyber: pozadi `accent_soft`, text normalni (tucne jen
                    // nadpisy). Radek agenta: `ai_soft` na 45 %, levy pruh
                    // `ai` a stitek "agent".
                    if *selected {
                        c.round_fill(r, rad, col.accent_soft);
                    } else if *agent {
                        c.round_fill(r, rad, gfx::scale(col.ai_soft, th.opacity.hung_content));
                    } else if hover {
                        c.round_fill(r, rad, col.surface_3);
                    }
                    let old = c.clip();
                    c.set_clip(old.intersect(&r));
                    let mut x = r.x + sp[4] as i32;
                    if *agent {
                        c.fill(Rect::new(r.x, r.y, sp[2], r.h), col.ai);
                        let h = th.size.badge_h - sp[2];
                        let w = agent_chip(c, &mut self.look, &mut self.icons, x, r.y + (r.h - h) as i32 / 2, h, "agent");
                        x += (w + sp[3]) as i32;
                    }
                    let s: TextStyle = self.look.body();
                    let f = self.look.face(s);
                    let b = r.y + f.baseline_in(s.size, r.h);
                    let room = (r.right() as i32 - sp[4] as i32 - x).max(0) as u32;
                    f.draw_ellipsized(c, x, b, text, s.size, col.ink, room);
                    c.set_clip(old);
                }
                ElemKind::Progress { value } => {
                    c.round_fill(r, r.h / 2, col.surface_3);
                    let w = r.w * (*value).min(100) / 100;
                    let fill = if e.agent { col.ai } else { col.accent };
                    if w > 0 {
                        c.round_fill(Rect::new(r.x, r.y, w.max(r.h), r.h), r.h / 2, fill);
                    }
                }
                ElemKind::Table(_) => self.draw_table(c, i, st),
                ElemKind::Switch { .. } | ElemKind::Radio { .. } | ElemKind::Slider { .. } | ElemKind::Segmented { .. } | ElemKind::Tabs { .. } | ElemKind::Busy => {
                    self.draw_control(c, i, st);
                }
                ElemKind::Group => {}
                ElemKind::Scroll { .. } => {
                    // Obsah je vlastni vrstva; tady jen ram.
                    c.round_stroke(r, th.radius.control, 1, col.border);
                }
            }
        }
    }

}
