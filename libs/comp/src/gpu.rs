//! Skladani pro GPU backend (`compd` + `libs/vkcomp`): misto kresleni do
//! snimku vznikne seznam operaci nad povrchy.
//!
//! Povrchy (zdola nahoru, pro kazdy poskozeny obdelnik):
//! - `Desktop` - pozadi plochy (CPU),
//! - pro kazde okno `Below` (stin, ram, titulek, pozadi obsahu; CPU),
//!   vrstvy obsahu (`Buffer` = buffer klienta, `Color` = barva 1x1
//!   roztazena na vrstvu) a pripadne `Above` (prvky nad obsahem; CPU),
//! - `Top` - panel, ztlumeni s dialogem, kurzor (CPU).
//!
//! Casti kreslene CPU vznikaji v odkladacim platne (cely snimek): oblast
//! se vynuluje (pruhledna), nakresli se stejnymi funkcemi jako v CPU ceste
//! (`render.rs`) a `GpuSink::upload` ji hned zkopiruje do povrchu. GPU pak
//! operace slozi vypocetnim shaderem se stejnymi vzorci jako `gfx`.
//!
//! Vysledek je pixel po pixelu stejny jako CPU cesta, az na prekryvy, kde
//! se na stejny pixel kresli vic poloprusvitnych veci nad sebou v jedne
//! CPU casti (ztlumeni okna, ktere nereaguje, s textem; poloprusvitne
//! okraje dialogu nad ztlumenim): tam se lisi zaokrouhleni (src-over v
//! celych cislech neni asociativni), nejvys o par urovni slozky.

use alloc::vec::Vec;

use gfx::Canvas;
use guiproto::Rect;
use guiproto::window::LayerKind;

use gfx::shape::corner_rects;

use crate::render::{Hover, Kids, children, draw_window_above, draw_window_below, draw_window_body, draw_window_shadow, window_core};
use crate::scene::{Client, Layer, Window};

/// Ma bezramove okno neco nad obsahem (odznak, ztlumeni)? Okna s ramem
/// maji vzdy ram a rohy.
fn has_above(w: &Window, cl: &Client, now: u64) -> bool {
    w.mode.has_frame() || w.badge_until > now || cl.unresponsive
}
use crate::{ClientId, Comp};

/// Zdrojovy povrch operace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Src {
    Desktop,
    Top,
    /// Cast okna pod obsahem (globalni id okna).
    Below(u32),
    /// Cast okna nad obsahem.
    Above(u32),
    /// Buffer klienta (klient, id bufferu).
    Buffer(ClientId, u32),
    /// Barva (premultiplied) 1x1.
    Color(u32),
}

/// Jedna operace: vyrez `src_rect` povrchu na `dst` snimku (skalovani
/// nejblizsim sousedem), jen v `clip`, s pruhlednosti.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuDraw {
    pub src: Src,
    pub src_rect: Rect,
    pub dst: Rect,
    pub clip: Rect,
    pub opacity: u8,
    pub opaque: bool,
}

/// Prijemce seznamu (sluzba s GPU, testy s referencnim provedenim).
pub trait GpuSink {
    /// Povrch `src` (ve snimku na `bounds`, velky `bounds.w x bounds.h`)
    /// ma novy obsah v oblasti `region` (souradnice snimku). Pixely jsou
    /// v `scratch` (cely snimek, krok `stride` pixelu).
    fn upload(&mut self, src: Src, bounds: Rect, region: Rect, scratch: &[u32], stride: u32);
    /// Buffer klienta pouzity ve snimku (pixely `stride * h`).
    fn buffer(&mut self, client: ClientId, id: u32, px: &[u32], w: u32, h: u32, stride: u32);
    fn draw(&mut self, d: GpuDraw);
}

/// Oblast povrchu `Below`/`Above` okna ve snimku (s okrajem stinu).
pub fn window_bounds(w: &Window) -> Rect {
    w.visual()
}

fn local(r: Rect, b: Rect) -> Rect {
    Rect::new(r.x - b.x, r.y - b.y, r.w, r.h)
}

impl Comp {
    /// Jako `tick`, ale snimek slozi GPU: operace jdou do `sink`, casti
    /// kreslene CPU vznikaji v `scratch` (platno velikosti snimku).
    pub fn tick_gpu(&mut self, now: u64, scratch: &mut Canvas<'_>, sink: &mut dyn GpuSink) -> Option<Vec<Rect>> {
        self.tick_with(now, |c, rects| c.compose_gpu(scratch, sink, rects, now))
    }

    pub(crate) fn compose_gpu(&mut self, canvas: &mut Canvas<'_>, sink: &mut dyn GpuSink, rects: &[Rect], now: u64) {
        let bar = self.bar_layout();
        let dialog_active = self.dialogs.active().is_some();
        let screen = self.screen();
        let hover = self.input.hover;
        if rects.iter().any(|r| !r.intersect(&self.panel_visual()).is_empty()) {
            self.ensure_bar_blur(now);
        }
        let mut trees: Vec<(u32, Kids)> = Vec::new();
        for &gid in &self.z {
            let Some(w) = self.windows.get(&gid) else { continue };
            // Stejny vyber jako CPU cesta (vcetne stinu).
            if !w.mapped || w.minimized || !rects.iter().any(|r| !w.visual().intersect(r).is_empty()) {
                continue;
            }
            let Some(cl) = self.clients.get(&w.client) else { continue };
            trees.push((gid, children(cl, w.local)));
        }
        let stride = canvas.stride();
        for &r in rects {
            let r = r.intersect(&screen);
            if r.is_empty() {
                continue;
            }
            canvas.set_clip(r);
            self.desktop(canvas);
            sink.upload(Src::Desktop, screen, r, canvas.pixels(), stride);
            sink.draw(GpuDraw { src: Src::Desktop, src_rect: r, dst: r, clip: r, opacity: 255, opaque: true });
            for (gid, kids) in &trees {
                let w = &self.windows[gid];
                if w.visual().intersect(&r).is_empty() {
                    continue;
                }
                let b = window_bounds(w);
                let reg = b.intersect(&r);
                let cl = &self.clients[&w.client];
                let focused = self.focus == Some(*gid);
                let hov = hover.and_then(|h| match h {
                    Hover::Button(g, i) if g == *gid => Some(i),
                    _ => None,
                });
                let core = window_core(w);
                // Pod obsahem: stin vsude, telo a titulek mimo rohove ctverce.
                canvas.set_clip(reg);
                canvas.fill(reg, 0);
                draw_window_shadow(canvas, &mut self.style, w, focused);
                for k in core {
                    canvas.set_clip(reg.intersect(&k));
                    if !canvas.clip().is_empty() {
                        draw_window_below(canvas, &mut self.style, w, cl, focused, hov);
                    }
                }
                sink.upload(Src::Below(*gid), b, reg, canvas.pixels(), stride);
                sink.draw(GpuDraw { src: Src::Below(*gid), src_rect: local(reg, b), dst: reg, clip: r, opacity: 255, opaque: false });
                // Obsah aplikace (mimo rohove ctverce).
                for k in core {
                    let content = r.intersect(&w.rect).intersect(&k);
                    if !content.is_empty() {
                        gpu_children(sink, w.client, cl, kids, 0, (w.rect.x, w.rect.y), content, 255, 0);
                    }
                }
                // Nad obsahem: prvky systemu a ram mimo rohy, v rozich cele
                // okno (i obsah, kresleny CPU) vynasobene zaoblenym tvarem.
                let rad = w.radius();
                if rad > 0 || has_above(w, cl, now) {
                    canvas.set_clip(reg);
                    canvas.fill(reg, 0);
                    for k in core {
                        canvas.set_clip(reg.intersect(&k));
                        if !canvas.clip().is_empty() {
                            draw_window_above(canvas, &mut self.style, w, cl, now, hov, focused);
                        }
                    }
                    if rad > 0 {
                        for k in corner_rects(w.outer(), rad) {
                            canvas.set_clip(reg.intersect(&k));
                            if !canvas.clip().is_empty() {
                                draw_window_body(canvas, &mut self.style, w, cl, kids, focused, now, hov);
                                canvas.set_clip(reg.intersect(&k));
                                canvas.round_corner_patch(w.outer(), rad);
                            }
                        }
                    }
                    sink.upload(Src::Above(*gid), b, reg, canvas.pixels(), stride);
                    sink.draw(GpuDraw { src: Src::Above(*gid), src_rect: local(reg, b), dst: reg, clip: r, opacity: 255, opaque: false });
                }
            }
            for t in self.top_regions(dialog_active) {
                let part = r.intersect(&t);
                if part.is_empty() {
                    continue;
                }
                canvas.set_clip(part);
                canvas.fill(part, 0);
                self.draw_top(canvas, &bar, dialog_active);
                sink.upload(Src::Top, screen, part, canvas.pixels(), stride);
                sink.draw(GpuDraw { src: Src::Top, src_rect: part, dst: part, clip: part, opacity: 255, opaque: false });
            }
        }
        canvas.reset_clip();
    }
}

/// Vrstvy obsahu jako operace (stejne jako `render::draw_children`).
#[allow(clippy::too_many_arguments)]
fn gpu_children(
    sink: &mut dyn GpuSink,
    client: ClientId,
    cl: &Client,
    kids: &Kids,
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
        gpu_layer(sink, client, cl, kids, l, origin, clip, opacity, depth, id);
    }
}

#[allow(clippy::too_many_arguments)]
fn gpu_layer(
    sink: &mut dyn GpuSink,
    client: ClientId,
    cl: &Client,
    kids: &Kids,
    l: &Layer,
    origin: (i32, i32),
    clip: Rect,
    opacity: u32,
    depth: u32,
    id: u32,
) {
    let p = &l.props;
    if !p.visible || p.opacity == 0 {
        return;
    }
    let r = p.rect.offset(origin.0, origin.1);
    let op = opacity * p.opacity as u32 / 255;
    let vis = clip.intersect(&r);
    if !vis.is_empty() {
        match l.kind {
            LayerKind::Solid => {
                if p.color != 0 {
                    sink.draw(GpuDraw {
                        src: Src::Color(p.color),
                        src_rect: Rect::new(0, 0, 1, 1),
                        dst: r,
                        clip: vis,
                        opacity: op as u8,
                        opaque: false,
                    });
                }
            }
            LayerKind::Buffer => {
                if let Some(b) = cl.buffers.get(&p.buffer)
                    && let Some(px) = b.px.pixels()
                    && (b.stride as u64 * b.h as u64) <= px.len() as u64
                    && b.stride >= b.w
                {
                    sink.buffer(client, p.buffer, px, b.w, b.h, b.stride);
                    let src = if p.src.is_empty() { Rect::new(0, 0, b.w, b.h) } else { p.src };
                    sink.draw(GpuDraw { src: Src::Buffer(client, p.buffer), src_rect: src, dst: r, clip: vis, opacity: op as u8, opaque: false });
                }
            }
            LayerKind::Group => {}
        }
    }
    let child_clip = if p.clip { clip.intersect(&r) } else { clip };
    if child_clip.is_empty() {
        return;
    }
    gpu_children(sink, client, cl, kids, id, (r.x.saturating_sub(p.scroll.0), r.y.saturating_sub(p.scroll.1)), child_clip, op, depth + 1);
}
