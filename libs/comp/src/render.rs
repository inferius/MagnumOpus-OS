//! Skladani snimku: plocha, okna (stin, ram, titulek, vrstvy), odznak
//! duvery, tlacitka, lista, ztlumeni a systemovy dialog, kurzor.
//!
//! Vzhled podle `design/okna.html` a `design/plocha.html`; barvy, rozmery,
//! zaobleni, stiny a pisma jsou z tokenu (`style.rs`), ne z kodu.
//!
//! Kresli se jen do poskozenych obdelniku (orez platna). Pruhlednost
//! skupiny se nasobi do deti (bez mezibufferu: prekryvajici se deti
//! poloprusvitne skupiny se prolnou i mezi sebou - zjednoduseni 2D cesty).
//!
//! Poradi kresleni okna (CPU i GPU dava stejny vysledek, `gpu.rs`):
//! 1. stin (vrstvy slozene do jedne barvy, jedno prolnuti),
//! 2. ulozit pixely rohu pod oknem,
//! 3. telo, titulek, obsah aplikace, prvky nad obsahem, ram (ctverec),
//! 4. rohy: okno pres ulozene pozadi s pokrytim zaobleneho tvaru.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::shape::core_rects;
use gfx::{Canvas, Image, scale};
use guiproto::Rect;
use guiproto::window::{LayerKind, Mode};
use theme::TextStyle;

use crate::Comp;
use crate::scene::{Client, Layer, Window};
use crate::style::Style;

/// Id aplikace spravce uloh (klik na ikonu uloh ji da do popredi).
pub const TASKS_APP: &str = "org.magnumopus.tasks";

/// Kurzor: 12 x 19, 0 pruhledne, 1 obrys, 2 vypln.
const CURSOR: [&str; 19] = [
    "1...........",
    "11..........",
    "121.........",
    "1221........",
    "12221.......",
    "122221......",
    "1222221.....",
    "12222221....",
    "122222221...",
    "1222222221..",
    "12222222221.",
    "122222211111",
    "1222121.....",
    "122111 1....",
    "121..121....",
    "11...121....",
    "1.....121...",
    "......121...",
    ".......11...",
];
pub const CURSOR_W: u32 = 12;
pub const CURSOR_H: u32 = 19;


/// Text ikony uloh (pristupnost, inspekce).
pub fn tasks_label(run: u32, agent: u32) -> String {
    if agent > 0 { format!("Úlohy: {run} (agent {agent})") } else { format!("Úlohy: {run}") }
}

/// Text upozorneni na umely vstup v liste.
pub const INJECT_LABEL: &str = "Vstup ovládá automat";

/// Ikona aplikace v liste podle id (bez vlastnich ikon aplikaci zatim
/// obecna sada).
pub fn app_icon(id: &str) -> &'static str {
    if id.contains("term") {
        "terminal"
    } else if id.contains("tasks") {
        "tasks"
    } else if id.contains("files") || id.contains("soubory") {
        "folder"
    } else if id.contains("settings") || id.contains("nastaveni") {
        "settings"
    } else {
        "window"
    }
}

/// Ikona opravneni v odznaku.
pub fn perm_icon(p: &str) -> &'static str {
    let p = p.to_lowercase();
    if p.contains("síť") || p.contains("net") {
        "globe"
    } else if p.contains("soub") || p.contains("fs") || p.contains("slož") {
        "folder"
    } else if p.contains("kamer") {
        "camera"
    } else if p.contains("mikro") {
        "mic"
    } else {
        "key"
    }
}

/// Rozlozeni listy.
#[derive(Debug, Clone, Default)]
pub struct Bar {
    pub rect: Rect,
    /// Tlacitka oken: (okno, obdelnik).
    pub apps: Vec<(u32, Rect)>,
    pub agent: Option<Rect>,
    pub tasks: Option<Rect>,
    pub clock: Option<Rect>,
    /// Oddelovace (x).
    pub seps: Vec<i32>,
}

/// Na co ukazuje mys (zvyrazneni pri najeti).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hover {
    /// Tlacitko okna (okno, index: 0 zavrit, 1 maximalizovat, 2 minimalizovat).
    Button(u32, usize),
    /// Tlacitko okna v liste.
    BarApp(u32),
    Tasks,
}

impl Comp {
    /// Kolik mista zabira lista u spodniho okraje (vcetne odsazeni).
    pub fn bar_reserve(&self) -> u32 {
        let s = &self.style.theme.size;
        s.bar_h + 2 * s.bar_inset
    }

    /// Plocha pro okna (obrazovka bez listy).
    pub fn work_area(&self) -> Rect {
        Rect::new(0, 0, self.w, self.h.saturating_sub(self.bar_reserve()))
    }

    pub(crate) fn panel_rect(&self) -> Rect {
        let s = &self.style.theme.size;
        Rect::new(s.bar_inset as i32, (self.h - s.bar_h - s.bar_inset) as i32, self.w.saturating_sub(2 * s.bar_inset), s.bar_h)
    }

    /// Lista vcetne stinu (poskozeni).
    pub(crate) fn panel_visual(&self) -> Rect {
        let p = self.panel_rect();
        let mut v = p;
        for s in &self.style.theme.shadow.bar {
            let e = s.blur as i32 + s.spread;
            v = v.union(&Rect::new(p.x + s.x - e, p.y + s.y - e, p.w + 2 * e as u32, p.h + 2 * e as u32));
        }
        v
    }

    pub(crate) fn damage_panel(&mut self) {
        let r = self.panel_visual();
        self.damage(r);
    }

    /// Hodiny v liste (cas a datum; `None` = nezobrazovat).
    pub fn set_clock(&mut self, clock: Option<(String, String)>) {
        if self.clock != clock {
            self.clock = clock;
            self.damage_panel();
        }
    }

    /// Rozlozeni listy (`design/plocha.html`, poloha "dole, odsazena").
    pub fn bar_layout(&self) -> Bar {
        let t = &self.style.theme;
        let p = self.panel_rect();
        let item = t.size.bar_icon;
        let pad = t.space[2] as i32;
        let gap = t.space[1] as i32;
        let y = p.y + (p.h as i32 - item as i32) / 2;
        let mut bar = Bar { rect: p, ..Default::default() };
        // Vlevo okna.
        let mut x = p.x + pad;
        for (&g, w) in &self.windows {
            if !w.mapped {
                continue;
            }
            bar.apps.push((g, Rect::new(x, y, item, item)));
            x += item as i32 + gap;
        }
        // Vpravo: hodiny, oddelovac, ulohy, agent.
        let mut right = p.right() as i32 - pad;
        if let Some((time, date)) = &self.clock {
            let w = self.style.width(time, clock_time(t)).max(self.style.width(date, t.text.caption)) + t.space[3];
            right -= w as i32;
            bar.clock = Some(Rect::new(right, y, w, item));
            right -= gap;
            bar.seps.push(right);
            right -= gap + 1;
        }
        let (run, agent) = self.tasks;
        if run > 0 {
            let w = self.tasks_width(run, agent);
            right -= w as i32;
            bar.tasks = Some(Rect::new(right, y, w, item));
            right -= gap;
        }
        if self.input.injecting > 0 {
            let h = t.size.control_h.min(item);
            let w = t.space[4] + t.size.icon_s + t.space[2] + self.style.width(INJECT_LABEL, small_strong(t)) + t.space[4];
            right -= w as i32;
            bar.agent = Some(Rect::new(right, p.y + (p.h as i32 - h as i32) / 2, w, h));
        }
        bar
    }

    fn tasks_width(&self, run: u32, agent: u32) -> u32 {
        let t = &self.style.theme;
        let mut w = t.space[4] + t.size.icon_m + t.space[2] + self.style.width(&format!("{run}"), small_strong(t)) + t.space[4];
        if agent > 0 {
            w += t.space[2] + self.agent_chip_width(agent);
        }
        w
    }

    fn agent_chip_width(&self, agent: u32) -> u32 {
        let t = &self.style.theme;
        t.space[3] + t.size.icon_s - 2 + t.space[1] + self.style.width(&format!("{agent}"), t.text.caption) + t.space[3]
    }

    /// Obdelnik ikony uloh v liste (`None` = nic nebezi).
    pub fn tasks_icon(&self) -> Option<Rect> {
        self.bar_layout().tasks
    }

    /// Polozky listy: (okno, obdelnik).
    pub(crate) fn panel_items(&self) -> Vec<(u32, Rect)> {
        self.bar_layout().apps
    }

    /// Sirka odznaku duvery okna klienta.
    pub(crate) fn badge_width(&self, cl: &Client) -> u32 {
        let t = &self.style.theme;
        let mut w = t.space[2] + t.size.icon_s + t.space[2] + self.style.width(&cl.info.name, badge_text(t)) + t.space[3];
        if !cl.info.perms.is_empty() {
            let n = cl.info.perms.len() as u32;
            w += t.space[1] + t.space[2] + 1 + n * perm_icon_size(t) + (n - 1) * t.space[1];
        }
        w
    }

    /// Slozi obdelniky `rects` do platna.
    pub(crate) fn compose(&mut self, canvas: &mut Canvas<'_>, rects: &[Rect], now: u64) {
        let bar = self.bar_layout();
        let dialog_active = self.dialogs.active().is_some();
        // Deti vrstev kazdeho viditelneho okna jednou za snimek (ne pro
        // kazdy poskozeny obdelnik).
        let mut trees: Vec<(u32, Kids)> = Vec::new();
        for &gid in &self.z {
            let Some(w) = self.windows.get(&gid) else { continue };
            if !w.mapped || w.minimized || !rects.iter().any(|r| !w.visual().intersect(r).is_empty()) {
                continue;
            }
            let Some(cl) = self.clients.get(&w.client) else { continue };
            trees.push((gid, children(cl, w.local)));
        }
        let hover = self.input.hover;
        for r in rects {
            canvas.set_clip(*r);
            self.desktop(canvas);
            for (gid, kids) in &trees {
                let w = &self.windows[gid];
                if w.visual().intersect(r).is_empty() {
                    continue;
                }
                let cl = &self.clients[&w.client];
                let focused = self.focus == Some(*gid);
                draw_window(canvas, &mut self.style, w, cl, kids, focused, now, hover.and_then(|h| button_of(h, *gid)));
            }
            // Vse nad okny pres mezibuffer (stejne jako GPU backend: jedno
            // prolnuti kazdeho pixelu).
            self.top_via_scratch(canvas, *r, &bar, dialog_active);
        }
        canvas.reset_clip();
    }

    /// Nakresli vrstvu nad okny do mezibufferu a prolne ji do platna.
    fn top_via_scratch(&mut self, canvas: &mut Canvas<'_>, r: Rect, bar: &Bar, dialog_active: bool) {
        let r = r.intersect(&canvas.bounds());
        if r.is_empty() {
            return;
        }
        let (w, h) = (canvas.w, canvas.h);
        let mut scratch = core::mem::take(&mut self.top_scratch);
        scratch.resize((w * h) as usize, 0);
        {
            let mut tc = Canvas::new(&mut scratch, w, h, w).unwrap();
            tc.set_clip(r);
            tc.fill(r, 0);
            self.draw_top(&mut tc, bar, dialog_active);
        }
        if let Some(img) = Image::new(&scratch, w, h, w) {
            canvas.set_clip(r);
            canvas.draw_image(&img, r, r, 255, false);
        }
        self.top_scratch = scratch;
    }

    /// Plocha (obraz z tokenu, spocitany jednou pro velikost).
    pub(crate) fn desktop(&mut self, c: &mut Canvas<'_>) {
        let (w, h) = (self.w, self.h);
        let img = self.style.backdrop(w, h);
        if let Some(img) = Image::new(img, w, h, w) {
            let clip = c.clip();
            c.draw_image(&img, clip, clip, 255, true);
        }
    }

    /// Vse nad okny: lista, ztlumeni a systemovy dialog, kurzor.
    pub(crate) fn draw_top(&mut self, canvas: &mut Canvas<'_>, bar: &Bar, dialog_active: bool) {
        let info = BarInfo {
            windows: &self.windows,
            clients: &self.clients,
            focus: self.focus,
            tasks: self.tasks,
            clock: self.clock.as_ref(),
            hover: self.input.hover,
        };
        draw_bar(canvas, &mut self.style, bar, &info);
        if dialog_active {
            // Ztlumeni vseho pod dialogem (jen system kresli nad nim).
            canvas.blend(self.screen(), self.style.theme.color.scrim);
            self.dialogs.draw(canvas, &mut self.style, self.w, self.h);
        }
        draw_cursor(canvas, self.input.px, self.input.py);
    }
}

fn button_of(h: Hover, gid: u32) -> Option<usize> {
    match h {
        Hover::Button(g, i) if g == gid => Some(i),
        _ => None,
    }
}

/// Styl casu v hodinach listy (`small` s vahou `body_strong`).
fn clock_time(t: &theme::Theme) -> TextStyle {
    TextStyle { weight: t.text.body_strong.weight, ..t.text.small }
}

fn small_strong(t: &theme::Theme) -> TextStyle {
    TextStyle { weight: t.text.body_strong.weight, ..t.text.small }
}

/// Text odznaku: `caption` s vahou `body_strong` (`.trust` v prototypu).
fn badge_text(t: &theme::Theme) -> TextStyle {
    TextStyle { weight: t.text.body_strong.weight, ..t.text.caption }
}

/// Ikona opravneni v odznaku (`size.icon_xs`).
fn perm_icon_size(t: &theme::Theme) -> u32 {
    t.size.icon_xs
}

/// Deti vrstev okna: rodic -> (poradi, id) serazene.
pub(crate) type Kids = BTreeMap<u32, Vec<(i32, u32)>>;

pub(crate) fn children(cl: &Client, window: u32) -> Kids {
    let mut kids: Kids = BTreeMap::new();
    for (&id, l) in &cl.layers {
        if l.window == window {
            kids.entry(l.parent).or_default().push((l.props.order, id));
        }
    }
    for v in kids.values_mut() {
        v.sort();
    }
    kids
}

/// Prostredni pas okna, ktery stin nemusi kreslit (prekryje ho telo).
pub(crate) fn shadow_skip(w: &Window) -> Rect {
    let o = w.outer();
    let r = w.radius();
    Rect::new(o.x, o.y + r as i32, o.w, o.h.saturating_sub(2 * r))
}

/// Stin okna podle stavu.
pub(crate) fn draw_window_shadow(c: &mut Canvas<'_>, st: &mut Style, w: &Window, focused: bool) {
    if !w.mode.has_frame() {
        return;
    }
    let sh = if focused { st.theme.shadow.window.clone() } else { st.theme.shadow.window_inactive.clone() };
    st.draw_shadow(c, &sh, w.outer(), w.radius(), shadow_skip(w));
}

#[allow(clippy::too_many_arguments)]
fn draw_window(c: &mut Canvas<'_>, st: &mut Style, w: &Window, cl: &Client, kids: &Kids, focused: bool, now: u64, hover: Option<usize>) {
    let clip = c.clip();
    draw_window_shadow(c, st, w, focused);
    let o = w.outer();
    let rad = w.radius();
    let under = if rad > 0 { c.save_corners(o, rad) } else { Vec::new() };
    draw_window_body(c, st, w, cl, kids, focused, now, hover);
    c.set_clip(clip);
    if rad > 0 {
        c.restore_corners(o, rad, &under);
    }
}

/// Cele okno bez stinu jako ctverec (telo, titulek, obsah, prvky nad
/// obsahem, ram). Rohy zaobli volajici (CPU: `restore_corners`, GPU:
/// rohova zaplata).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_window_body(c: &mut Canvas<'_>, st: &mut Style, w: &Window, cl: &Client, kids: &Kids, focused: bool, now: u64, hover: Option<usize>) {
    let clip = c.clip();
    draw_window_below(c, st, w, cl, focused, hover);
    let content_clip = clip.intersect(&w.rect);
    if !content_clip.is_empty() {
        draw_children(c, cl, kids, 0, (w.rect.x, w.rect.y), content_clip, 255, 0);
        c.set_clip(clip);
    }
    draw_window_above(c, st, w, cl, now, hover);
}

/// Cast okna pod obsahem aplikace: telo, titulek (pozadi, odznak, text,
/// tlacitka). Kresli se jako ctverec (bez stinu a zaobleni).
pub(crate) fn draw_window_below(c: &mut Canvas<'_>, st: &mut Style, w: &Window, cl: &Client, focused: bool, hover: Option<usize>) {
    let col = st.theme.color.clone();
    if w.mode.has_frame() {
        c.fill(w.outer(), col.surface);
    } else {
        c.fill(w.rect, col.surface);
    }
    if w.mode != Mode::Standard {
        return;
    }
    let t = w.title_bar().unwrap();
    let o = w.outer();
    // Titulek pres celou sirku okna (az k ramu).
    let bar = Rect::new(o.x, o.y, o.w, t.bottom() as u32 - o.y as u32);
    c.fill(bar, if focused { col.surface_2 } else { col.surface });
    let badge = draw_badge(c, st, w, cl);
    draw_buttons(c, st, w, hover);
    // Jmeno: na stred mezi odznakem a tlacitky, zkracene.
    let th = &st.theme;
    let style = if focused { th.text.body_strong } else { TextStyle { weight: th.text.caption.weight, ..th.text.body_strong } };
    let gap = th.space[3] as i32;
    let b = w.buttons();
    let (left, right) = match w.side {
        guiproto::window::ButtonSide::Right => (badge.right() as i32 + gap, b[2].x - gap),
        guiproto::window::ButtonSide::Left => (b[2].right() as i32 + gap, badge.x - gap),
    };
    if right > left {
        let room = (right - left) as u32;
        let ink = if focused { col.ink } else { col.ink_muted };
        let face = st.face(style);
        let tw = face.width(&w.title, style.size).min(room);
        let x = left + (room - tw) as i32 / 2;
        let base = t.y + face.baseline_in(style.size, t.h);
        let old = c.clip();
        c.set_clip(old.intersect(&Rect::new(left, t.y, room, t.h)));
        face.draw_ellipsized(c, x, base, &w.title, style.size, ink, room);
        c.set_clip(old);
    }
}

/// Prvky nad obsahem: tlacitka a odznak oken bez titulku (vlastni titulek,
/// bez ramu), ztlumeni neodpovidajici aplikace, ram okna.
pub(crate) fn draw_window_above(c: &mut Canvas<'_>, st: &mut Style, w: &Window, cl: &Client, now: u64, hover: Option<usize>) {
    if w.mode.has_frame() {
        if w.mode != Mode::Standard {
            draw_buttons(c, st, w, hover);
            draw_badge(c, st, w, cl);
        }
    } else if w.badge_until > now {
        draw_badge(c, st, w, cl);
    }
    if cl.unresponsive {
        draw_hung(c, st, w, cl);
    }
    if w.mode.has_frame() {
        let col = st.theme.color.border;
        c.inner_ring(w.outer(), w.radius(), w.geo.border, col);
    }
}

/// Aplikace nereaguje: ztlumeny obsah a karta uprostred (kresli system).
fn draw_hung(c: &mut Canvas<'_>, st: &mut Style, w: &Window, cl: &Client) {
    let t = st.theme.clone();
    // Obsah na kryti `opacity.hung_content` = plocha s doplnkovym kryti pres nej.
    c.blend(w.rect, scale(t.color.surface, 255 - t.opacity.hung_content));
    let title = format!("{} nereaguje", cl.info.name);
    let note = format!("{} s bez odpovědi.", t.motion.unresponsive_ms / 1000);
    let (s1, s2) = (t.text.body_strong, t.text.small);
    let pad = t.space[5];
    let cw = (st.width(&title, s1).max(st.width(&note, s2)) + 2 * pad).min(w.rect.w);
    let ch = (s1.line + t.space[2] + s2.line + 2 * pad).min(w.rect.h);
    let card = Rect::new(w.rect.x + (w.rect.w as i32 - cw as i32) / 2, w.rect.y + (w.rect.h as i32 - ch as i32) / 2, cw, ch);
    let pop = t.shadow.popover.clone();
    st.draw_shadow(c, &pop, card, t.radius.card, Rect::default());
    c.round_fill_bordered(card, t.radius.card, t.color.surface, t.color.border_strong, 1);
    let mut y = card.y + pad as i32;
    for (s, style, ink) in [(&title, s1, t.color.ink), (&note, s2, t.color.ink_muted)] {
        let face = st.face(style);
        let tw = face.width(s, style.size) as i32;
        let base = y + face.baseline_in(style.size, style.line);
        face.draw(c, card.x + (card.w as i32 - tw) / 2, base, s, style.size, ink);
        y += style.line as i32 + t.space[2] as i32;
    }
}

/// Odznak duvery: stit, jmeno z manifestu (od initu), ikony opravneni.
/// Vraci jeho obdelnik.
fn draw_badge(c: &mut Canvas<'_>, st: &mut Style, w: &Window, cl: &Client) -> Rect {
    let t = st.theme.clone();
    let style = badge_text(&t);
    let mut bw = t.space[2] + t.size.icon_s + t.space[2] + st.width(&cl.info.name, style) + t.space[3];
    let n = cl.info.perms.len() as u32;
    let pi = perm_icon_size(&t);
    if n > 0 {
        bw += t.space[1] + t.space[2] + 1 + n * pi + (n - 1) * t.space[1];
    }
    let r = w.badge(bw);
    let col = &t.color;
    c.round_fill_bordered(r, t.radius.badge, col.trust_bg, col.trust_border, 1);
    // Obsah jen uvnitr (mimo vyhlazene rohy): v uzkem okne se jmeno zkrati
    // a ikony opravneni vynechaji.
    let old = c.clip();
    let inner = Rect::new(r.x + t.space[2] as i32, r.y, r.w.saturating_sub(t.space[2] + t.space[3]), r.h);
    c.set_clip(old.intersect(&inner));
    let is = t.size.icon_s;
    let mut x = inner.x;
    st.icons.draw(c, "shield", x, r.y + (r.h as i32 - is as i32) / 2, is, t.icon_stroke, col.trust_ink);
    x += (is + t.space[2]) as i32;
    let perms_w = if n > 0 { t.space[1] + t.space[2] + 1 + n * pi + (n - 1) * t.space[1] } else { 0 };
    let room = (inner.right() as i32 - x).max(0) as u32;
    let fits = st.width(&cl.info.name, style) + perms_w <= room;
    let face = st.face(style);
    let base = r.y + face.baseline_in(style.size, r.h);
    x = face.draw_ellipsized(c, x, base, &cl.info.name, style.size, col.trust_ink, if fits { room } else { room.saturating_sub(perms_w.min(room / 2)) });
    if n > 0 && fits {
        x += t.space[1] as i32;
        // Svisla cara oddelujici opravneni.
        c.fill(Rect::new(x, r.y + 5, 1, r.h.saturating_sub(10)), col.trust_border);
        x += 1 + t.space[2] as i32;
        for p in &cl.info.perms {
            st.icons.draw(c, perm_icon(p), x, r.y + (r.h as i32 - pi as i32) / 2, pi, t.icon_stroke, col.trust_ink);
            x += (pi + t.space[1]) as i32;
        }
    }
    c.set_clip(old);
    r
}

/// Tlacitka okna: minimalizovat, maximalizovat, zavrit (pilulky, ikona
/// vedlejsi barvou; pri najeti zvyraznene, zavrit cervene).
fn draw_buttons(c: &mut Canvas<'_>, st: &mut Style, w: &Window, hover: Option<usize>) {
    let t = st.theme.clone();
    let col = &t.color;
    let icons = ["close", if w.restore.is_some() { "restore" } else { "max" }, "min"];
    for (i, b) in w.buttons().iter().enumerate() {
        let hot = hover == Some(i);
        let (bg, ink) = match (hot, i) {
            (true, 0) => (Some(col.danger), col.on_danger),
            (true, _) => (Some(col.surface_3), col.ink),
            _ => (None, col.ink_muted),
        };
        if let Some(bg) = bg {
            c.round_fill(*b, b.w / 2, bg);
        } else if w.button_color != 0 {
            c.round_fill(*b, b.w / 2, w.button_color | 0xFF00_0000);
        }
        let s = t.size.icon_xs;
        st.icons.draw(c, icons[i], b.x + (b.w - s) as i32 / 2, b.y + (b.h - s) as i32 / 2, s, t.size.title_btn_stroke as f32, ink);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_children(
    c: &mut Canvas<'_>,
    cl: &Client,
    kids: &BTreeMap<u32, Vec<(i32, u32)>>,
    parent: u32,
    origin: (i32, i32),
    clip: Rect,
    opacity: u32,
    depth: u32,
) {
    if depth > 32 {
        return;
    }
    let Some(list) = kids.get(&parent) else { return };
    for &(_, id) in list {
        let Some(l) = cl.layers.get(&id) else { continue };
        draw_layer(c, cl, kids, id, l, origin, clip, opacity, depth);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_layer(
    c: &mut Canvas<'_>,
    cl: &Client,
    kids: &BTreeMap<u32, Vec<(i32, u32)>>,
    id: u32,
    l: &Layer,
    origin: (i32, i32),
    clip: Rect,
    opacity: u32,
    depth: u32,
) {
    let p = &l.props;
    if !p.visible || p.opacity == 0 {
        return;
    }
    let r = p.rect.offset(origin.0, origin.1);
    let op = opacity * p.opacity as u32 / 255;
    let vis = clip.intersect(&r);
    if !vis.is_empty() {
        c.set_clip(vis);
        match l.kind {
            LayerKind::Solid => c.blend(r, scale(p.color, op)),
            LayerKind::Buffer => {
                if let Some(b) = cl.buffers.get(&p.buffer)
                    && let Some(px) = b.px.pixels()
                    && let Some(img) = Image::new(px, b.w, b.h, b.stride)
                {
                    let src = if p.src.is_empty() { Rect::new(0, 0, b.w, b.h) } else { p.src };
                    c.draw_image(&img, src, r, op as u8, false);
                }
            }
            LayerKind::Group => {}
        }
    }
    let child_clip = if p.clip { clip.intersect(&r) } else { clip };
    if child_clip.is_empty() {
        return;
    }
    draw_children(c, cl, kids, id, (r.x.saturating_sub(p.scroll.0), r.y.saturating_sub(p.scroll.1)), child_clip, op, depth + 1);
}

/// Co lista potrebuje vedet o scene.
struct BarInfo<'a> {
    windows: &'a BTreeMap<u32, Window>,
    clients: &'a BTreeMap<crate::ClientId, Client>,
    focus: Option<u32>,
    tasks: (u32, u32),
    clock: Option<&'a (String, String)>,
    hover: Option<Hover>,
}

fn draw_bar(c: &mut Canvas<'_>, st: &mut Style, bar: &Bar, info: &BarInfo<'_>) {
    let t = st.theme.clone();
    let col = &t.color;
    let p = bar.rect;
    let vis = c.clip();
    let shadow = t.shadow.bar.clone();
    let mut area = p;
    for s in &shadow {
        let e = s.blur as i32 + s.spread;
        area = area.union(&Rect::new(p.x + s.x - e, p.y + s.y - e, p.w + 2 * e as u32, p.h + 2 * e as u32));
    }
    if vis.intersect(&area).is_empty() {
        return;
    }
    st.draw_shadow(c, &shadow, p, t.radius.window, Rect::default());
    c.round_fill_bordered(p, t.radius.window, scale(col.surface, t.opacity.bar), col.border, 1);
    let ctl = t.radius.control;
    // Okna.
    let il = t.size.icon_l;
    for (g, r) in &bar.apps {
        let Some(w) = info.windows.get(g) else { continue };
        let active = info.focus == Some(*g) && !w.minimized;
        if info.hover == Some(Hover::BarApp(*g)) {
            c.round_fill(*r, ctl, col.surface_3);
        }
        let icon = info.clients.get(&w.client).map(|cl| app_icon(&cl.info.id)).unwrap_or("window");
        st.icons.draw(c, icon, r.x + (r.w - il) as i32 / 2, r.y + (r.h - il) as i32 / 2, il, t.icon_stroke, col.ink);
        // Znacka bezici / aktivni (pod ikonou).
        let s = &t.size;
        let (mw, mc) = if active { (s.bar_indicator_active_w, col.accent) } else { (s.bar_indicator_w, col.ink_faint) };
        let mh = s.bar_indicator_h;
        c.round_fill(Rect::new(r.x + (r.w - mw) as i32 / 2, r.bottom() as i32 - mh as i32 - 1, mw, mh), mh, mc);
    }
    for &x in &bar.seps {
        let h = t.space[7];
        c.fill(Rect::new(x, p.y + (p.h - h) as i32 / 2, 1, h), col.border);
    }
    // Ulohy: ikona, pocet, stitek agenta.
    if let Some(r) = bar.tasks {
        let (run, agent) = info.tasks;
        if info.hover == Some(Hover::Tasks) {
            c.round_fill(r, ctl, col.surface_3);
        }
        let im = t.size.icon_m;
        let mut x = r.x + t.space[4] as i32;
        st.icons.draw(c, "tasks", x, r.y + (r.h - im) as i32 / 2, im, t.icon_stroke, col.ink);
        x += (im + t.space[2]) as i32;
        let s = small_strong(&t);
        let face = st.face(s);
        let base = r.y + face.baseline_in(s.size, r.h);
        x = face.draw(c, x, base, &format!("{run}"), s.size, col.ink);
        if agent > 0 {
            x += t.space[2] as i32;
            let cw = t.space[3] + t.size.icon_s - 2 + t.space[1] + st.width(&format!("{agent}"), t.text.caption) + t.space[3];
            let chip_h = t.space[7] - 2;
            let chip = Rect::new(x, r.y + (r.h - chip_h) as i32 / 2, cw, chip_h);
            c.round_fill_bordered(chip, chip_h / 2, col.ai_soft, col.ai, 1);
            let is = t.size.icon_s - 2;
            st.icons.draw(c, "agent", chip.x + t.space[3] as i32, chip.y + (chip.h - is) as i32 / 2, is, t.icon_stroke, col.ai_ink);
            let cs = t.text.caption;
            let face = st.face(cs);
            let base = chip.y + face.baseline_in(cs.size, chip.h);
            face.draw(c, chip.x + (t.space[3] + is + t.space[1]) as i32, base, &format!("{agent}"), cs.size, col.ai_ink);
        }
    }
    // Umely vstup: pilulka agenta (barva jen pro agenta, vzdy s ikonou
    // a textem).
    if let Some(r) = bar.agent {
        c.round_fill_bordered(r, r.h / 2, col.ai_soft, col.ai, 1);
        let is = t.size.icon_s;
        st.icons.draw(c, "agent", r.x + t.space[4] as i32, r.y + (r.h - is) as i32 / 2, is, t.icon_stroke, col.ai_ink);
        let s = small_strong(&t);
        let face = st.face(s);
        let base = r.y + face.baseline_in(s.size, r.h);
        face.draw(c, r.x + (t.space[4] + is + t.space[2]) as i32, base, INJECT_LABEL, s.size, col.ai_ink);
    }
    // Hodiny: cas a pod nim datum, zarovnane vpravo.
    if let (Some(r), Some((time, date))) = (bar.clock, info.clock) {
        let (ts, ds) = (clock_time(&t), t.text.caption);
        // Dva radky s vyskou 1,15 x velikost (prototyp `.bar-clock`).
        let l1 = ts.size as i32 * 115 / 100;
        let l2 = ds.size as i32 * 115 / 100;
        let top = r.y + (r.h as i32 - l1 - l2) / 2;
        let right = r.right() as i32;
        let face = st.face(ts);
        let tw = face.width(time, ts.size) as i32;
        let b1 = top + face.baseline_in(ts.size, l1 as u32);
        face.draw(c, right - tw, b1, time, ts.size, col.ink);
        let face = st.face(ds);
        let dw = face.width(date, ds.size) as i32;
        let b2 = top + l1 + face.baseline_in(ds.size, l2 as u32);
        face.draw(c, right - dw, b2, date, ds.size, col.ink_muted);
    }
}

fn draw_cursor(c: &mut Canvas<'_>, x: i32, y: i32) {
    for (row, line) in CURSOR.iter().enumerate() {
        for (col, ch) in line.bytes().enumerate() {
            let p = match ch {
                b'1' => gfx::rgb(0, 0, 0),
                b'2' => gfx::rgb(255, 255, 255),
                _ => continue,
            };
            c.fill(Rect::new(x + col as i32, y + row as i32, 1, 1), p);
        }
    }
}

/// Pasy okna bez rohovych ctvercu (GPU backend kresli obsah jen do nich).
pub(crate) fn window_core(w: &Window) -> [Rect; 3] {
    core_rects(w.outer(), w.radius())
}
