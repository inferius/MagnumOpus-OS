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
pub(crate) fn agent_chip(c: &mut Canvas<'_>, look: &mut crate::Look, icons: &mut gfx::icon::Icons, x: i32, y: i32, h: u32, text: &str) -> u32 {
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

/// Pruh prubehu (`value` 0-100) v `r`: draha `surface_3`, vypln akcentem
/// (uloha agenta barvou `ai`). Sdileny s aplikacemi, ktere kresli samy
/// (panel uloh terminalu).
pub fn progress_bar(c: &mut Canvas<'_>, r: Rect, value: u32, agent: bool, col: &theme::Colors) {
    c.round_fill(r, r.h / 2, col.surface_3);
    let w = r.w * value.min(100) / 100;
    let fill = if agent { col.ai } else { col.accent };
    if w > 0 {
        c.round_fill(Rect::new(r.x, r.y, w.max(r.h), r.h), r.h / 2, fill);
    }
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
        if !self.transparent {
            c.fill(c.bounds(), col.surface);
        }
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
            if e.row.is_some() {
                self.draw_panel_row(c, i, st);
                continue;
            }
            match &e.kind {
                ElemKind::Label { text, heading, warn, lines } => {
                    let s = if *heading { self.look.heading() } else { crate::panel::tone_style(&self.look, e.tone) };
                    let ink = crate::panel::tone_color(col, e.tone);
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
                    } else if *lines == 1 || (!text.contains('\n') && self.look.width(text, s) <= r.w) {
                        // Jeden radek (vejde se nebo `lines(1)`: zkratit s `...`).
                        let f = self.look.face(s);
                        let b = r.y + f.baseline_in(s.size, r.h);
                        f.draw_ellipsized(c, r.x, b, text, s.size, ink, r.w);
                    } else {
                        // Zalomeny: radky pod sebou, blok svisle na stredu.
                        let ls = crate::layout::label_lines(&self.look, text, s, r.w, *lines);
                        let top = r.y + (r.h.saturating_sub(ls.len() as u32 * s.line) / 2) as i32;
                        let f = self.look.face(s);
                        let b0 = f.baseline_in(s.size, s.line);
                        for (k, l) in ls.iter().enumerate() {
                            f.draw_ellipsized(c, r.x, top + k as i32 * s.line as i32 + b0, l, s.size, ink, r.w);
                        }
                    }
                    c.set_clip(old);
                }
                ElemKind::Button { text, icon, .. } => {
                    use crate::node::Variant;
                    let hot = (pressed || hover) && !dim;
                    // Najeti u plnych variant: vypln ztmavena k `ink` (88 %).
                    let hot_of = |c: u32| fade(c, col.ink, 225);
                    let (bg, edge, ink) = match e.variant {
                        Variant::Secondary => (if hot { col.surface_3 } else { col.surface }, col.border_strong, col.ink),
                        Variant::Primary => (if hot { hot_of(col.accent) } else { col.accent }, col.accent, col.on_accent),
                        Variant::Danger => (if hot { hot_of(col.danger) } else { col.danger }, col.danger, col.on_danger),
                        Variant::Ghost => (if hot { col.surface_3 } else { 0 }, 0, col.ink),
                    };
                    // Nedostupne: stejne tlacitko s kryti `opacity.disabled`.
                    let k = if dim { th.opacity.disabled } else { 255 };
                    if bg != 0 {
                        c.round_fill_bordered(r, r.h / 2, fade(bg, col.surface, k), fade(if edge == 0 { bg } else { edge }, col.surface, k), 1);
                    }
                    if focused && !dim {
                        focus_ring(c, r, r.h / 2, ring, ring_off, col.focus);
                    }
                    let s = self.look.button();
                    let f = self.look.face(s);
                    let tw = f.width(text, s.size) as i32;
                    let fg = fade(ink, col.surface, k);
                    let b = r.y + f.baseline_in(s.size, r.h);
                    // Ikona pred textem (bez textu na stred).
                    let is = th.size.icon_m;
                    let gap = if text.is_empty() { 0 } else { sp[2] as i32 };
                    let total = tw + icon.map_or(0, |_| is as i32 + gap);
                    let x0 = r.x + (r.w as i32 - total) / 2;
                    if let Some(n) = icon {
                        self.icons.draw(c, n, x0, r.y + (r.h as i32 - is as i32) / 2, is, th.icon_stroke, fg);
                    }
                    let f = self.look.face(s);
                    f.draw(c, x0 + total - tw, b, text, s.size, fg);
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
                ElemKind::ListItem { text, selected, agent, index, list } => {
                    let rad = th.radius.control;
                    // Vyber a kurzor jsou dve veci: kurzor seznamu
                    // s vicenasobnym vyberem ma prstenec (jen z klavesnice).
                    let cursor = self.focus == Some(*list - 1) && self.active && self.kbd_focus && self.list_cursor(*list - 1) == Some(*index);
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
                    if cursor {
                        c.round_stroke(r, rad, ring, col.focus);
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
                ElemKind::Progress { value } => progress_bar(c, r, *value, e.agent, col),
                ElemKind::Table(_) => self.draw_table(c, i, st),
                ElemKind::Select { .. } => self.draw_select(c, i, st),
                // Obraz kresli klient do vrstvy videa nad oknem; pod nim je
                // jen pozadi okna (pruhy pri jinem pomeru stran).
                ElemKind::Video { .. } => {}
                ElemKind::Image { w, h, px, .. } => {
                    // Zmenseni se zachovanim pomeru stran, na stred prvku.
                    if let Some(img) = gfx::Image::new(px, *w, *h, *w) {
                        let k = (r.w as u64 * *h as u64).min(r.h as u64 * *w as u64);
                        let (dw, dh) = if *w == 0 || *h == 0 { (0, 0) } else { ((k / *h as u64) as u32, (k / *w as u64) as u32) };
                        let (dw, dh) = (dw.min(*w), dh.min(*h));
                        let d = Rect::new(r.x + (r.w - dw) as i32 / 2, r.y + (r.h - dh) as i32 / 2, dw, dh);
                        c.draw_image(&img, Rect::new(0, 0, *w, *h), d, 255, false);
                    }
                }
                ElemKind::Switch { .. } | ElemKind::Radio { .. } | ElemKind::Slider { .. } | ElemKind::Segmented { .. } | ElemKind::Tabs { .. } | ElemKind::Busy | ElemKind::Segments { .. } => {
                    self.draw_control(c, i, st);
                }
                ElemKind::Group => {
                    if let Some(d) = &e.deco {
                        crate::panel::draw_deco(c, r, d, col);
                    }
                }
                ElemKind::Icon { name, size } => {
                    let s = if *size == 0 { th.size.icon_m } else { *size };
                    let fg = if dim { fade(col.ink, col.surface, th.opacity.disabled) } else { col.ink };
                    self.icons.draw(c, name, r.x + (r.w as i32 - s as i32) / 2, r.y + (r.h as i32 - s as i32) / 2, s, th.icon_stroke, fg);
                }
                ElemKind::Scroll { .. } => {
                    // Obsah je vlastni vrstva; tady jen ram.
                    c.round_stroke(r, th.radius.control, 1, col.border);
                }
            }
        }
        // Nabidka a bublina nad prvky okna (s vrstvou popoveru ve ni).
        if space == 0 && !self.overlay {
            self.draw_overlays(c);
        }
    }

}
