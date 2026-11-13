//! Jednoduche ovladaci prvky: prepinac (`switch`), volba (`radio`),
//! posuvnik hodnoty (`slider`), segmentovy prepinac (`segmented`),
//! zalozky (`tabs`) a neurcity prubeh (`busy`).
//!
//! Vzhled podle `design/assets/mo.css` (`.switch`, `.seg`, `.tab`)
//! a tokenu (`size.switch_*`, `radius.*`). Akce jako u tlacitka az pri
//! pusteni nad prvkem; posuvnik meni hodnotu uz pri stisku a tahu.
//!
//! Klavesy: mezernik / Enter prepne prepinac a vybere volbu; posuvnik
//! sipky (o krok), PgUp/PgDn (o 10 kroku), Home/End, kolecko; segmenty
//! a zalozky sipky vlevo / vpravo, Home/End; Ctrl+Tab a Ctrl+Shift+Tab
//! prepinaji prvni zalozky v okne; prostredni tlacitko zalozku zavre.

use alloc::string::String;
use alloc::vec::Vec;

use gfx::{Canvas, Rect};
use guiproto::a11y::{self, Node as A11yNode, Role};
use guiproto::window::InputEvent;
use inputproto::code::*;
use inputproto::mods;

use crate::event::Response;
use crate::layout::ElemKind;
use crate::{Key, State, Ui};

/// Nejsirsi zalozka (px, `.tab { max-width }` v navrhu).
const TAB_MAX: u32 = 180;

/// Vnitrni okraj segmentoveho prepinace (`.seg { padding: 3px }`).
const SEG_PAD: u32 = 3;

fn virt_id(el: u32, kind: u64, n: u64) -> u32 {
    0x4000_0000 | (Key::ROOT.mix(el as u64).mix(kind).mix(n).0 as u32 & 0x3FFF_FFFF)
}

impl<M: Clone> Ui<M> {
    /// Obdelniky segmentu nebo zalozek prvku `i`.
    pub(crate) fn part_rects(&self, i: usize) -> Vec<Rect> {
        let e = &self.elems[i];
        let r = e.rect;
        let sp = &self.look.theme.space;
        let body = self.look.body();
        match &e.kind {
            ElemKind::Segmented { options, .. } => {
                let ws: Vec<u32> = options.iter().map(|o| self.look.width(o, body) + 2 * sp[4]).collect();
                let total: u32 = ws.iter().sum::<u32>() + 2 * options.len().saturating_sub(1) as u32;
                let inner = r.w.saturating_sub(2 * SEG_PAD);
                let extra = inner.saturating_sub(total) / options.len().max(1) as u32;
                let mut x = r.x + SEG_PAD as i32;
                ws.iter()
                    .map(|&w| {
                        let o = Rect::new(x, r.y + SEG_PAD as i32, w + extra, r.h.saturating_sub(2 * SEG_PAD));
                        x += (w + extra + 2) as i32;
                        o
                    })
                    .collect()
            }
            ElemKind::Tabs { labels, on_close, .. } => {
                let close = if on_close.is_some() { self.look.theme.size.icon_xs + sp[2] } else { 0 };
                let mut x = r.x;
                labels
                    .iter()
                    .map(|l| {
                        let w = (self.look.width(l, body) + 2 * sp[4] + close).min(TAB_MAX);
                        let o = Rect::new(x, r.y, w, r.h);
                        x += (w + sp[1]) as i32;
                        o
                    })
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /// Krizek zalozky `k` (jen s `on_close`).
    fn tab_close(&self, i: usize, k: usize) -> Option<Rect> {
        let ElemKind::Tabs { on_close: Some(_), .. } = &self.elems[i].kind else { return None };
        let t = self.part_rects(i).get(k).copied()?;
        let is = self.look.theme.size.icon_xs;
        let sp = &self.look.theme.space;
        Some(Rect::new(t.right() as i32 - (sp[3] + is) as i32, t.y + (t.h - is) as i32 / 2, is, is))
    }

    /// Draha a jezdec posuvniku.
    fn slider_geo(&self, i: usize) -> Option<(Rect, Rect)> {
        let ElemKind::Slider { value, min, max, .. } = self.elems[i].kind else { return None };
        let r = self.elems[i].rect;
        let k = self.look.theme.size.switch_knob;
        let th = self.look.theme.space[1];
        let track = Rect::new(r.x + k as i32 / 2, r.y + (r.h - th) as i32 / 2, r.w.saturating_sub(k), th);
        let span = (max - min).max(1) as i64;
        let x = track.x + ((value - min) as i64 * track.w as i64 / span) as i32;
        Some((track, Rect::new(x - k as i32 / 2, r.y + (r.h - k) as i32 / 2, k, k)))
    }

    fn slider_value_at(&self, i: usize, x: i32) -> Option<i32> {
        let ElemKind::Slider { min, max, step, .. } = self.elems[i].kind else { return None };
        let (track, _) = self.slider_geo(i)?;
        let span = (max - min) as i64;
        let v = min as i64 + ((x - track.x).clamp(0, track.w as i32) as i64 * span + track.w as i64 / 2) / track.w.max(1) as i64;
        // Na krok od `min`.
        let st = step.max(1) as i64;
        let v = min as i64 + ((v - min as i64 + st / 2) / st) * st;
        Some((v as i32).clamp(min, max))
    }

    fn slider_emit(&self, i: usize, v: i32, r: &mut Response<M>) {
        if let ElemKind::Slider { value, on_change: Some(f), .. } = &self.elems[i].kind
            && *value != v
        {
            r.msgs.push(f(v));
        }
        r.redraw = true;
    }

    fn pick(&self, i: usize, k: usize, r: &mut Response<M>) {
        match &self.elems[i].kind {
            ElemKind::Segmented { selected, on_select: Some(f), options } | ElemKind::Tabs { selected, on_select: Some(f), labels: options, .. }
                if k < options.len() && k != *selected =>
            {
                r.msgs.push(f(k));
            }
            _ => {}
        }
        r.redraw = true;
    }

    fn close_tab(&self, i: usize, k: usize, r: &mut Response<M>) {
        if let ElemKind::Tabs { on_close: Some(f), labels, .. } = &self.elems[i].kind
            && k < labels.len()
        {
            r.msgs.push(f(k));
        }
    }

    /// Aktivace (klik, Enter, mezernik, akce pristupnosti) noveho prvku;
    /// `false` = neni to prvek z tohoto modulu.
    pub(crate) fn activate_control(&mut self, i: usize, r: &mut Response<M>) -> bool {
        let (px, py) = self.to_space(i, self.pointer.0, self.pointer.1);
        match &self.elems[i].kind {
            ElemKind::Switch { on, on_toggle, .. } => {
                if let Some(f) = on_toggle {
                    r.msgs.push(f(!*on));
                }
            }
            ElemKind::Radio { checked, on_click, .. } => {
                if !*checked && let Some(m) = on_click {
                    r.msgs.push(m.clone());
                }
            }
            ElemKind::Segmented { .. } | ElemKind::Tabs { .. } => {
                let parts = self.part_rects(i);
                if let Some(k) = parts.iter().position(|p| p.contains(px, py)) {
                    if self.tab_close(i, k).is_some_and(|c| Rect::new(c.x - 4, c.y - 4, c.w + 8, c.h + 8).contains(px, py)) {
                        self.close_tab(i, k, r);
                    } else {
                        self.pick(i, k, r);
                    }
                }
            }
            ElemKind::Slider { .. } | ElemKind::Busy => {}
            _ => return false,
        }
        r.redraw = true;
        true
    }

    /// Klavesa na prvku s fokusem. `false` = nezpracovano.
    pub(crate) fn control_key(&mut self, i: usize, code: u16, r: &mut Response<M>) -> bool {
        match &self.elems[i].kind {
            ElemKind::Slider { value, min, max, step, .. } => {
                let (v, lo, hi, st) = (*value, *min, *max, *step);
                let n = match code {
                    KEY_LEFT | KEY_DOWN => v - st,
                    KEY_RIGHT | KEY_UP => v + st,
                    KEY_PAGEDOWN => v - 10 * st,
                    KEY_PAGEUP => v + 10 * st,
                    KEY_HOME => lo,
                    KEY_END => hi,
                    _ => return false,
                };
                self.slider_emit(i, n.clamp(lo, hi), r);
                true
            }
            ElemKind::Segmented { selected, options, .. } | ElemKind::Tabs { selected, labels: options, .. } => {
                let (s, n) = (*selected, options.len());
                if n == 0 {
                    return false;
                }
                let k = match code {
                    KEY_LEFT => s.saturating_sub(1),
                    KEY_RIGHT => (s + 1).min(n - 1),
                    KEY_HOME => 0,
                    KEY_END => n - 1,
                    _ => return false,
                };
                self.pick(i, k, r);
                true
            }
            _ => false,
        }
    }

    /// Udalosti, ktere novym prvkum patri pred obecnym zpracovanim
    /// (tah posuvniku, kolecko nad nim, prostredni tlacitko na zalozce,
    /// Ctrl+Tab). `None` = obecne zpracovani.
    pub(crate) fn control_event(&mut self, ev: &InputEvent) -> Option<Response<M>> {
        let mut r = Response::default();
        match *ev {
            InputEvent::Button { code, pressed: true, x, y } if code == BTN_LEFT => {
                let i = self.hit(x, y)?;
                if !matches!(self.elems[i].kind, ElemKind::Slider { .. }) || self.elems[i].disabled {
                    return None;
                }
                self.pointer = (x, y);
                self.focus = Some(i);
                self.pressed = Some(i);
                self.hover = Some(i);
                let (sx, _) = self.to_space(i, x, y);
                let v = self.slider_value_at(i, sx)?;
                self.slider_emit(i, v, &mut r);
                Some(r)
            }
            InputEvent::PointerMove { x, y } => {
                let i = self.pressed?;
                if !matches!(self.elems.get(i)?.kind, ElemKind::Slider { .. }) {
                    return None;
                }
                self.pointer = (x, y);
                let (sx, _) = self.to_space(i, x, y);
                let v = self.slider_value_at(i, sx)?;
                self.slider_emit(i, v, &mut r);
                Some(r)
            }
            InputEvent::Button { code: BTN_MIDDLE, pressed: true, x, y } => {
                let i = self.hit(x, y)?;
                if !matches!(self.elems[i].kind, ElemKind::Tabs { .. }) {
                    return None;
                }
                let (sx, sy) = self.to_space(i, x, y);
                let k = self.part_rects(i).iter().position(|p| p.contains(sx, sy))?;
                self.close_tab(i, k, &mut r);
                Some(r)
            }
            InputEvent::Scroll { dy, .. } => {
                let i = self.hit(self.pointer.0, self.pointer.1)?;
                let ElemKind::Slider { value, min, max, step, .. } = self.elems[i].kind else { return None };
                if self.elems[i].disabled {
                    return None;
                }
                self.slider_emit(i, (value + dy * step).clamp(min, max), &mut r);
                Some(r)
            }
            // Ctrl+Tab / Ctrl+Shift+Tab: dalsi / predchozi zalozka (prvni
            // zalozky v okne), at je fokus kdekoli.
            InputEvent::Key { code: KEY_TAB, action, .. } if action != 0 && self.mods & mods::CTRL != 0 => {
                let i = self.elems.iter().position(|e| matches!(e.kind, ElemKind::Tabs { .. }))?;
                let ElemKind::Tabs { selected, labels, .. } = &self.elems[i].kind else { return None };
                let n = labels.len();
                if n == 0 {
                    return Some(r);
                }
                let k = if self.mods & mods::SHIFT != 0 { (selected + n - 1) % n } else { (selected + 1) % n };
                self.pick(i, k, &mut r);
                Some(r)
            }
            _ => None,
        }
    }

    /// Bezi nejaky neurcity prubeh (animace)?
    pub(crate) fn has_busy(&self) -> bool {
        self.elems.iter().any(|e| matches!(e.kind, ElemKind::Busy))
    }

    /// Nakresli prvek z tohoto modulu; `false` = jiny druh.
    pub(crate) fn draw_control(&mut self, c: &mut Canvas<'_>, i: usize, st: State) -> bool {
        let th = self.look.theme.clone();
        let col = &th.color;
        let sp = &th.space;
        let r = self.elems[i].rect;
        let k = if st.disabled { th.opacity.disabled } else { 255 };
        let fade = |x: u32| crate::draw::fade(x, col.surface, k);
        let body = self.look.body();
        let (ring, off) = (th.size.focus_ring, th.size.focus_offset);
        match &self.elems[i].kind {
            ElemKind::Switch { text, on, .. } => {
                let (sw, sh) = (th.size.switch_w, th.size.switch_h);
                let t = Rect::new(r.x, r.y + (r.h as i32 - sh as i32) / 2, sw, sh);
                let (bg, edge, knob) = if *on { (col.accent, col.accent, col.on_accent) } else { (col.surface_3, col.border_strong, col.ink_muted) };
                c.round_fill_bordered(t, sh / 2, fade(bg), fade(edge), 1);
                let kn = th.size.switch_knob;
                let ky = t.y + (sh - kn) as i32 / 2;
                let kx = if *on { t.right() as i32 - kn as i32 - 3 } else { t.x + 3 };
                c.round_fill(Rect::new(kx, ky, kn, kn), kn / 2, fade(knob));
                if st.focus_visible {
                    crate::draw::focus_ring(c, t, sh / 2, ring, off, col.focus);
                }
                let text = text.clone();
                let f = self.look.face(body);
                let b = r.y + f.baseline_in(body.size, r.h);
                f.draw(c, r.x + (sw + sp[3]) as i32, b, &text, body.size, fade(col.ink));
            }
            ElemKind::Radio { text, checked, .. } => {
                let d = th.size.icon_m;
                let o = Rect::new(r.x, r.y + (r.h as i32 - d as i32) / 2, d, d);
                c.round_fill_bordered(o, d / 2, col.surface, fade(if *checked { col.accent } else { col.border_strong }), if *checked { 2 } else { 1 });
                if *checked {
                    let dot = d / 2;
                    c.round_fill(Rect::new(o.x + (d - dot) as i32 / 2, o.y + (d - dot) as i32 / 2, dot, dot), dot / 2, fade(col.accent));
                }
                if st.focus_visible {
                    crate::draw::focus_ring(c, o, d / 2, ring, off, col.focus);
                }
                let text = text.clone();
                let f = self.look.face(body);
                let b = r.y + f.baseline_in(body.size, r.h);
                f.draw(c, r.x + (d + sp[3]) as i32, b, &text, body.size, fade(col.ink));
            }
            ElemKind::Slider { .. } => {
                let Some((track, knob)) = self.slider_geo(i) else { return true };
                c.round_fill(track, track.h / 2, fade(col.surface_3));
                let filled = Rect::new(track.x, track.y, (knob.x + knob.w as i32 / 2 - track.x).max(0) as u32, track.h);
                c.round_fill(filled, track.h / 2, fade(col.accent));
                let edge = if st.pressed || st.hover { col.accent } else { col.border_strong };
                c.round_fill_bordered(knob, knob.w / 2, col.surface, fade(edge), 2);
                if st.focus_visible {
                    crate::draw::focus_ring(c, knob, knob.w / 2, ring, off, col.focus);
                }
            }
            ElemKind::Segmented { options, selected, .. } => {
                let (options, selected) = (options.clone(), *selected);
                c.round_fill(r, r.h / 2, fade(col.surface_3));
                let parts = self.part_rects(i);
                let strong = theme::TextStyle { weight: th.text.body_strong.weight, ..body };
                for (k, p) in parts.iter().enumerate() {
                    let on = k == selected;
                    if on {
                        c.round_fill_bordered(*p, p.h / 2, col.surface, col.border, 1);
                    }
                    let s = if on { strong } else { body };
                    let f = self.look.face(s);
                    let tw = f.width(&options[k], s.size) as i32;
                    let b = p.y + f.baseline_in(s.size, p.h);
                    f.draw(c, p.x + (p.w as i32 - tw) / 2, b, &options[k], s.size, fade(if on { col.accent_ink } else { col.ink }));
                    if on && st.focus_visible {
                        crate::draw::focus_ring(c, *p, p.h / 2, ring, off, col.focus);
                    }
                }
            }
            ElemKind::Tabs { labels, selected, on_close, .. } => {
                let (labels, selected, closable) = (labels.clone(), *selected, on_close.is_some());
                let parts = self.part_rects(i);
                let strong = theme::TextStyle { weight: th.text.body_strong.weight, ..body };
                let (px, py) = self.to_space(i, self.pointer.0, self.pointer.1);
                let rad = th.radius.control;
                for (k, p) in parts.iter().enumerate() {
                    let on = k == selected;
                    if on {
                        c.round_fill(*p, rad, col.surface_2);
                        c.fill(Rect::new(p.x, p.bottom() as i32 - rad as i32, p.w, rad), col.surface_2);
                    } else if st.hover && p.contains(px, py) {
                        c.round_fill(*p, rad, col.surface_3);
                    }
                    let s = if on { strong } else { body };
                    let room = p.w.saturating_sub(2 * sp[4] + if closable { th.size.icon_xs + sp[2] } else { 0 });
                    let f = self.look.face(s);
                    let b = p.y + f.baseline_in(s.size, p.h);
                    f.draw_ellipsized(c, p.x + sp[4] as i32, b, &labels[k], s.size, fade(if on { col.ink } else { col.ink_muted }), room);
                    if let Some(x) = self.tab_close(i, k) {
                        self.icons.draw(c, "close", x.x, x.y, x.w, th.icon_stroke, col.ink_muted);
                    }
                    if on && st.focus_visible {
                        crate::draw::focus_ring(c, *p, rad, ring, off, col.focus);
                    }
                }
            }
            ElemKind::Busy => {
                c.round_fill(r, r.h / 2, col.surface_3);
                // Usek 30 % sirky obiha drahu (krok na snimek).
                let w = (r.w * 3 / 10).max(r.h);
                let span = r.w + w;
                let x = r.x - w as i32 + (self.busy_phase % span.max(1)) as i32;
                let seg = Rect::new(x, r.y, w, r.h).intersect(&r);
                if seg.w > 0 {
                    let fill = if self.elems[i].agent { col.ai } else { col.accent };
                    c.round_fill(seg, r.h / 2, fill);
                }
            }
            _ => return false,
        }
        true
    }

    /// Role, jmeno, hodnota a akce noveho prvku pro strom pristupnosti.
    pub(crate) fn control_role(&self, i: usize) -> Option<(Role, String, String, u32, u32)> {
        use a11y::actions::*;
        Some(match &self.elems[i].kind {
            ElemKind::Switch { text, on, .. } => (Role::Switch, text.clone(), String::new(), if *on { a11y::state::CHECKED } else { 0 }, TOGGLE | ACTIVATE | FOCUS),
            ElemKind::Radio { text, checked, .. } => (Role::Radio, text.clone(), String::new(), if *checked { a11y::state::CHECKED } else { 0 }, ACTIVATE | FOCUS),
            ElemKind::Slider { value, min, max, .. } => (Role::Slider, String::new(), alloc::format!("{value} ({min}-{max})"), 0, SET_VALUE | FOCUS),
            ElemKind::Segmented { options, selected, .. } => (Role::Segmented, String::new(), options.get(*selected).cloned().unwrap_or_default(), 0, FOCUS),
            ElemKind::Tabs { labels, selected, .. } => (Role::TabList, String::new(), labels.get(*selected).cloned().unwrap_or_default(), 0, FOCUS),
            ElemKind::Busy => (Role::Progress, String::new(), String::from("probíhá"), a11y::state::BUSY, 0),
            _ => return None,
        })
    }

    /// Podrizene uzly segmentu (`radio`) a zalozek (`tab`).
    pub(crate) fn control_children(&self, i: usize, id: u32, layer: u32, out: &mut Vec<A11yNode>) {
        let (names, selected, role) = match &self.elems[i].kind {
            ElemKind::Segmented { options, selected, .. } => (options, *selected, Role::Radio),
            ElemKind::Tabs { labels, selected, .. } => (labels, *selected, Role::Tab),
            _ => return,
        };
        let sel = if role == Role::Tab { a11y::state::SELECTED } else { a11y::state::CHECKED };
        for (k, (n, b)) in names.iter().zip(self.part_rects(i)).enumerate() {
            out.push(A11yNode {
                id: virt_id(id, 3, k as u64),
                parent: id,
                role,
                name: n.clone(),
                value: String::new(),
                state: if k == selected { sel } else { 0 },
                layer,
                bounds: b,
                actions: a11y::actions::ACTIVATE,
            });
        }
    }

    /// Akce pristupnosti na podrizenem uzlu (segment, zalozka) nebo
    /// nastaveni hodnoty posuvniku. Vraci, zda uzel patril sem.
    pub(crate) fn control_a11y_action(&mut self, node: u32, action: u32, r: &mut Response<M>) -> bool {
        for i in 0..self.elems.len() {
            let n = match &self.elems[i].kind {
                ElemKind::Segmented { options, .. } => options.len(),
                ElemKind::Tabs { labels, .. } => labels.len(),
                _ => continue,
            };
            let id = self.elem_ids[i];
            if let Some(k) = (0..n).find(|&k| virt_id(id, 3, k as u64) == node) {
                if action & a11y::actions::ACTIVATE != 0 {
                    self.pick(i, k, r);
                }
                return true;
            }
        }
        false
    }

    /// `SET_VALUE` na posuvniku (cislo v textu).
    pub(crate) fn slider_set(&mut self, i: usize, value: &str, r: &mut Response<M>) -> bool {
        let ElemKind::Slider { min, max, .. } = self.elems[i].kind else { return false };
        if let Ok(v) = value.trim().parse::<i32>() {
            self.slider_emit(i, v.clamp(min, max), r);
        }
        true
    }
}
