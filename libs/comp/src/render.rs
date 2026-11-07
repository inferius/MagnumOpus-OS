//! Skladani snimku: plocha, okna (ram, titulek, vrstvy), odznak duvery,
//! tlacitka, panel oken, ztlumeni a systemovy dialog, kurzor.
//!
//! Kresli se jen do poskozenych obdelniku (orez platna). Pruhlednost
//! skupiny se nasobi do deti (bez mezibufferu: prekryvajici se deti
//! poloprusvitne skupiny se prolnou i mezi sebou - zjednoduseni 2D cesty).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::text::Text;
use gfx::{Canvas, Image, rgb, rgba, scale};
use guiproto::Rect;
use guiproto::window::{LayerKind, Mode};

use crate::scene::{BORDER, Client, Layer, TITLE_H, Window};
use crate::Comp;

/// Barvy systemu (tmavy motiv).
pub mod color {
    use gfx::rgb;
    pub const DESKTOP_TOP: u32 = rgb(0x1C, 0x2A, 0x3A);
    pub const DESKTOP_BOTTOM: u32 = rgb(0x0E, 0x14, 0x1C);
    pub const TITLE: u32 = rgb(0x2B, 0x30, 0x38);
    pub const TITLE_FOCUSED: u32 = rgb(0x33, 0x3D, 0x4C);
    pub const TITLE_TEXT: u32 = rgb(0xE6, 0xEA, 0xEE);
    pub const TITLE_TEXT_DIM: u32 = rgb(0x9A, 0xA4, 0xAE);
    pub const BORDER: u32 = rgb(0x44, 0x4C, 0x56);
    pub const BORDER_FOCUSED: u32 = rgb(0x5A, 0x9B, 0xE8);
    pub const BUTTON: u32 = rgb(0x3E, 0x46, 0x52);
    pub const CLOSE: u32 = rgb(0xC8, 0x4B, 0x4B);
    pub const BADGE: u32 = rgb(0x1F, 0x6F, 0x4E);
    pub const BADGE_TEXT: u32 = rgb(0xEA, 0xFF, 0xF4);
    pub const PANEL: u32 = rgb(0x12, 0x18, 0x20);
    pub const PANEL_ITEM: u32 = rgb(0x26, 0x2E, 0x38);
    pub const PANEL_ACTIVE: u32 = rgb(0x2F, 0x5E, 0x96);
    pub const WARN: u32 = rgb(0xE8, 0xA8, 0x3A);
    pub const CONTENT: u32 = rgb(0x18, 0x1C, 0x22);
}

/// Id aplikace spravce uloh (klik na ikonu uloh ji da do popredi).
pub const TASKS_APP: &str = "org.magnumopus.tasks";

/// Vyska panelu oken u spodniho okraje.
pub const PANEL_H: u32 = 30;
/// Velikost pisma systemu.
pub const UI_PX: f32 = 14.0;
pub const SMALL_PX: f32 = 12.0;

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

/// Text ikony uloh.
pub fn tasks_label(run: u32, agent: u32) -> String {
    if agent > 0 { format!("Úlohy: {run} (agent {agent})") } else { format!("Úlohy: {run}") }
}

impl Comp {
    /// Obdelnik ikony uloh v panelu (`None` = nic nebezi). Vpravo, vlevo od
    /// upozorneni na umely vstup.
    pub fn tasks_icon(&self) -> Option<Rect> {
        let (run, agent) = self.tasks;
        if run == 0 {
            return None;
        }
        let p = self.panel_rect();
        let w = self.text.width(&tasks_label(run, agent), SMALL_PX) + 16;
        let reserve = if self.input.injecting > 0 { self.text.width("Vstup ovládá automat", SMALL_PX) as i32 + 40 } else { 8 };
        Some(Rect::new(p.right() as i32 - reserve - w as i32, p.y + 4, w, PANEL_H - 8))
    }

    pub(crate) fn panel_rect(&self) -> Rect {
        Rect::new(0, (self.h - PANEL_H) as i32, self.w, PANEL_H)
    }

    pub(crate) fn damage_panel(&mut self) {
        let r = self.panel_rect();
        self.damage(r);
    }

    /// Text odznaku: jmeno aplikace a co smi.
    pub(crate) fn badge_text(c: &Client) -> String {
        if c.info.perms.is_empty() {
            c.info.name.clone()
        } else {
            format!("{} \u{00B7} {}", c.info.name, c.info.perms.join(", "))
        }
    }

    pub(crate) fn badge_width(text: &Text, s: &str) -> u32 {
        (text.width(s, SMALL_PX) + 16).min(260)
    }

    /// Polozky panelu: (okno, obdelnik).
    pub(crate) fn panel_items(&self) -> Vec<(u32, Rect)> {
        let p = self.panel_rect();
        let mut x = 8;
        let mut out = Vec::new();
        for (&g, w) in &self.windows {
            if !w.mapped {
                continue;
            }
            let tw = self.text.width(&w.title, SMALL_PX).clamp(40, 160) + 20;
            out.push((g, Rect::new(x, p.y + 4, tw, PANEL_H - 8)));
            x += tw as i32 + 6;
        }
        out
    }

    /// Slozi obdelniky `rects` do platna.
    pub(crate) fn compose(&mut self, canvas: &mut Canvas<'_>, rects: &[Rect], now: u64) {
        let panel_items = self.panel_items();
        let dialog_active = self.dialogs.active().is_some();
        // Deti vrstev kazdeho viditelneho okna jednou za snimek (ne pro
        // kazdy poskozeny obdelnik).
        let mut trees: Vec<(u32, Kids)> = Vec::new();
        for &gid in &self.z {
            let Some(w) = self.windows.get(&gid) else { continue };
            if !w.mapped || w.minimized || !rects.iter().any(|r| !w.outer().intersect(r).is_empty()) {
                continue;
            }
            let Some(cl) = self.clients.get(&w.client) else { continue };
            trees.push((gid, children(cl, w.local)));
        }
        for r in rects {
            canvas.set_clip(*r);
            desktop(canvas, self.h);
            for (gid, kids) in &trees {
                let w = &self.windows[gid];
                if w.outer().intersect(r).is_empty() {
                    continue;
                }
                let cl = &self.clients[&w.client];
                let focused = self.focus == Some(*gid);
                draw_window(canvas, &mut self.text, w, cl, kids, focused, now);
            }
            let panel = Panel {
                rect: self.panel_rect(),
                windows: &self.windows,
                focus: self.focus,
                injecting: self.input.injecting,
                tasks: self.tasks,
                tasks_rect: self.tasks_icon(),
            };
            draw_panel(canvas, &mut self.text, &panel, &panel_items);
            if dialog_active {
                // Ztlumeni vseho pod dialogem (jen system kresli nad nim).
                canvas.blend(self.screen(), rgba(0, 0, 0, 150));
                self.dialogs.draw(canvas, &mut self.text, self.w, self.h);
            }
            draw_cursor(canvas, self.input.px, self.input.py);
        }
        canvas.reset_clip();
    }
}

fn desktop(c: &mut Canvas<'_>, h: u32) {
    let clip = c.clip();
    // Svisly prechod po pruzich 8 px (jen v orezu).
    let (t, b) = (color::DESKTOP_TOP, color::DESKTOP_BOTTOM);
    let mut y = clip.y - clip.y.rem_euclid(8);
    while (y as i64) < clip.bottom() {
        let k = (y.max(0) as u32 * 255 / h.max(1)).min(255);
        let mix = |sh: u32| ((t >> sh & 0xFF) * (255 - k) + (b >> sh & 0xFF) * k) / 255;
        let col = 0xFF00_0000 | mix(16) << 16 | mix(8) << 8 | mix(0);
        c.fill(Rect::new(clip.x, y, clip.w, 8), col);
        y += 8;
    }
}

/// Deti vrstev okna: rodic -> (poradi, id) serazene.
type Kids = BTreeMap<u32, Vec<(i32, u32)>>;

fn children(cl: &Client, window: u32) -> Kids {
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

fn draw_window(c: &mut Canvas<'_>, text: &mut Text, w: &Window, cl: &Client, kids: &Kids, focused: bool, now: u64) {
    let outer = w.outer();
    let clip = c.clip();
    // Ram a stin.
    if w.mode.has_frame() {
        c.blend(Rect::new(outer.x + 3, outer.y + 3, outer.w, outer.h), rgba(0, 0, 0, 70));
        c.fill(outer, if focused { color::BORDER_FOCUSED } else { color::BORDER });
    }
    if w.mode == Mode::Standard {
        let t = w.title_bar().unwrap();
        c.fill(t, if focused { color::TITLE_FOCUSED } else { color::TITLE });
    }
    // Obsah: pozadi a vrstvy aplikace.
    c.fill(w.rect, color::CONTENT);
    let content_clip = clip.intersect(&w.rect);
    if !content_clip.is_empty() {
        draw_children(c, cl, kids, 0, (w.rect.x, w.rect.y), content_clip, 255, 0);
        c.set_clip(clip);
    }
    // Systemove prvky nad obsahem.
    if w.mode.has_frame() {
        if w.mode == Mode::Standard {
            let t = w.title_bar().unwrap();
            let badge_w = Comp::badge_width(text, &Comp::badge_text(cl));
            let (lh, base) = text.line(UI_PX);
            let tx = t.x + badge_w as i32 + 14;
            let ty = t.y + ((TITLE_H as i32 - lh as i32) / 2) + base;
            let col = if focused { color::TITLE_TEXT } else { color::TITLE_TEXT_DIM };
            // Titulek konci pred tlacitky.
            let tclip = clip.intersect(&Rect::new(tx, t.y, (t.right() as i32 - tx - 110).max(0) as u32, TITLE_H));
            if !tclip.is_empty() {
                c.set_clip(tclip);
                text.draw(c, tx, ty, &w.title, UI_PX, col);
                c.set_clip(clip);
            }
        }
        draw_buttons(c, text, w);
        draw_badge(c, text, w, cl);
    } else if w.badge_until > now {
        draw_badge(c, text, w, cl);
    }
    if cl.unresponsive {
        c.blend(outer, rgba(0, 0, 0, 120));
        let (_, base) = text.line(UI_PX);
        text.draw(c, w.rect.x + 12, w.rect.y + 40 + base, "Aplikace nereaguje", UI_PX, color::WARN);
    }
    let _ = BORDER;
}

#[allow(clippy::too_many_arguments)]
fn draw_children(
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

fn draw_buttons(c: &mut Canvas<'_>, text: &mut Text, w: &Window) {
    let b = w.buttons();
    let base_col = if w.button_color != 0 { w.button_color | 0xFF00_0000 } else { color::BUTTON };
    for (i, r) in b.iter().enumerate() {
        let col = if i == 0 { color::CLOSE } else { base_col };
        c.rounded(*r, 4, col);
        let cx = r.x + r.w as i32 / 2;
        let cy = r.y + r.h as i32 / 2;
        match i {
            0 => {
                // Krizek ze dvou uhlopricek.
                for d in -4..=4 {
                    c.fill(Rect::new(cx + d, cy + d, 2, 2), color::TITLE_TEXT);
                    c.fill(Rect::new(cx + d, cy - d, 2, 2), color::TITLE_TEXT);
                }
            }
            1 => c.outline(Rect::new(cx - 5, cy - 5, 11, 10), 2, color::TITLE_TEXT),
            _ => c.fill(Rect::new(cx - 5, cy + 3, 11, 2), color::TITLE_TEXT),
        }
    }
    let _ = text;
}

fn draw_badge(c: &mut Canvas<'_>, text: &mut Text, w: &Window, cl: &Client) {
    let s = Comp::badge_text(cl);
    let bw = Comp::badge_width(text, &s);
    let r = w.badge(bw);
    c.rounded(r, 6, color::BADGE);
    // Stit (system): zelena ctverecek se svislou carkou.
    c.fill(Rect::new(r.x + 6, r.y + 6, 2, r.h.saturating_sub(12)), color::BADGE_TEXT);
    let (lh, base) = text.line(SMALL_PX);
    let ty = r.y + (r.h as i32 - lh as i32) / 2 + base;
    let old = c.clip();
    c.set_clip(old.intersect(&Rect::new(r.x, r.y, r.w.saturating_sub(4), r.h)));
    text.draw(c, r.x + 12, ty, &s, SMALL_PX, color::BADGE_TEXT);
    c.set_clip(old);
}

struct Panel<'a> {
    rect: Rect,
    windows: &'a BTreeMap<u32, Window>,
    focus: Option<u32>,
    injecting: u32,
    tasks: (u32, u32),
    tasks_rect: Option<Rect>,
}

fn draw_panel(c: &mut Canvas<'_>, text: &mut Text, comp: &Panel<'_>, items: &[(u32, Rect)]) {
    let p = comp.rect;
    if c.clip().intersect(&p).is_empty() {
        return;
    }
    c.fill(p, color::PANEL);
    let (lh, base) = text.line(SMALL_PX);
    for (g, r) in items {
        let Some(w) = comp.windows.get(g) else { continue };
        let active = comp.focus == Some(*g) && !w.minimized;
        c.rounded(*r, 5, if active { color::PANEL_ACTIVE } else { color::PANEL_ITEM });
        let old = c.clip();
        c.set_clip(old.intersect(&Rect::new(r.x + 6, r.y, r.w.saturating_sub(12), r.h)));
        text.draw(c, r.x + 10, r.y + (r.h as i32 - lh as i32) / 2 + base, &w.title, SMALL_PX, color::TITLE_TEXT);
        c.set_clip(old);
    }
    // Ikona "bezi N uloh" (oranzova, kdyz bezi uloha agenta AI).
    if let Some(r) = comp.tasks_rect {
        let (run, agent) = comp.tasks;
        let s = tasks_label(run, agent);
        let warn = agent > 0;
        c.rounded(r, 5, if warn { color::WARN } else { color::PANEL_ITEM });
        let fg = if warn { rgb(0x20, 0x16, 0x00) } else { color::TITLE_TEXT };
        text.draw(c, r.x + 8, p.y + (PANEL_H as i32 - lh as i32) / 2 + base, &s, SMALL_PX, fg);
    }
    if comp.injecting > 0 {
        let s = "Vstup ovládá automat";
        let tw = text.width(s, SMALL_PX) as i32;
        let x = p.right() as i32 - tw - 16;
        c.rounded(Rect::new(x - 8, p.y + 4, tw as u32 + 16, PANEL_H - 8), 5, color::WARN);
        text.draw(c, x, p.y + (PANEL_H as i32 - lh as i32) / 2 + base, s, SMALL_PX, rgb(0x20, 0x16, 0x00));
    }
}

fn draw_cursor(c: &mut Canvas<'_>, x: i32, y: i32) {
    for (row, line) in CURSOR.iter().enumerate() {
        for (col, ch) in line.bytes().enumerate() {
            let p = match ch {
                b'1' => rgb(0, 0, 0),
                b'2' => rgb(255, 255, 255),
                _ => continue,
            };
            c.fill(Rect::new(x + col as i32, y + row as i32, 1, 1), p);
        }
    }
}
