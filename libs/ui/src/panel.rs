//! Komponenta *panel* (design/bubliny-panely.html, `.pnl`): hlavicka
//! s ikonou a souhrnem, radky s ikonou, nazvem a popisem (tlacitko, volba
//! radio, prepinac nebo jen informace), sekce, stavovy radek, pata.
//!
//! Panel nema vlastni druh prvku: sklada se z beznych uzlu (sloupec,
//! radek, stitek, tlacitko, volba, prepinac, posuvnik) s vlastnostmi
//! vzhledu ([`Node::pad`], [`Node::gap`], [`Deco`], [`TextTone`], [`RowLook`]).
//! Fokus, klavesy (skupina radio: sipky vyberou), pristupnost a najeti
//! jsou tak stejne jako u prvku v aplikacich. Pouziva ho lista kompozitoru
//! (`libs/comp`, `sysui`) i aplikace (Nastaveni).
//!
//! Rozmery jen z tokenu: sirka `size.panel_w`, radek nejmene
//! `size.panel_row`, mezera ikona-text `size.panel_gap`, okraj hlavicky
//! a paty `size.panel_edge`, vodorovne odsazeni `space.s5`.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::node::{Kind, Node, column, label, node, row, spacer};

/// Ton textu stitku.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextTone {
    #[default]
    Normal,
    /// Barva `ink_muted`, pismo `body`.
    Muted,
    /// Popis pod nazvem a souhrn v hlavicce: `caption`, `ink_muted`.
    Small,
    /// Nadpis sekce: `caption` vahy `body_strong`, velkymi, `ink_muted`.
    Section,
    /// Nazev v hlavicce: `body_strong`.
    Strong,
    /// Varovani ve stavovem radku: `small`, barva `danger`.
    Danger,
}

/// Vypln kontejneru.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    /// `surface_3` (stavovy radek).
    Surface3,
    /// `danger_soft` (varovani).
    DangerSoft,
}

/// Vzhled kontejneru (sloupec, radek): vypln se zaoblenim, cara nahore.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Deco {
    pub fill: Option<Fill>,
    pub radius: u32,
    /// Cara `border` pres celou sirku nahore (pata panelu).
    pub top_border: bool,
}

/// Radek panelu: ikona vlevo, nazev (text prvku) a popis pod nim. Plati
/// pro tlacitko (radek s najetim), volbu radio (fajfka vpravo), prepinac
/// (jezdec vpravo) a stitek (jen informace).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RowLook {
    pub icon: Option<&'static str>,
    pub desc: String,
}

/// Ikona (velikost 0 = `size.icon_m`).
pub fn icon<M>(name: &'static str) -> Node<M> {
    node(Kind::Icon { name, size: 0 })
}

impl<M> Node<M> {
    /// Vnitrni odsazeni kontejneru: nahore, vpravo, dole, vlevo (px).
    pub fn pad(mut self, top: u32, right: u32, bottom: u32, left: u32) -> Self {
        self.pad = [top, right, bottom, left];
        self
    }

    /// Mezera mezi detmi kontejneru (jinak `space.s3`).
    pub fn gap(mut self, g: u32) -> Self {
        self.gap = Some(g);
        self
    }

    pub fn deco(mut self, d: Deco) -> Self {
        self.deco = Some(Box::new(d));
        self
    }

    pub fn tone(mut self, t: TextTone) -> Self {
        self.tone = t;
        self
    }

    /// Prvek jako radek panelu s ikonou a popisem (tlacitko, volba,
    /// prepinac, stitek).
    pub fn panel_row(mut self, icon: Option<&'static str>, desc: &str) -> Self {
        self.row = Some(Box::new(RowLook { icon, desc: desc.into() }));
        self
    }
}

/// Rozmery panelu z tokenu.
#[derive(Debug, Clone, Copy)]
pub struct PanelSize {
    pub width: u32,
    /// Vodorovne odsazeni (`s5`).
    pub side: u32,
    pub gap: u32,
    pub edge: u32,
    /// `s1`, `s2`, `s3` (svisle odsazeni radku, sekce a paty).
    pub s1: u32,
    pub s2: u32,
    pub s3: u32,
    pub radius: u32,
}

impl PanelSize {
    pub fn of(t: &theme::Theme) -> Self {
        Self {
            width: t.size.panel_w,
            side: t.space[5],
            gap: t.size.panel_gap,
            edge: t.size.panel_edge,
            s1: t.space[1],
            s2: t.space[2],
            s3: t.space[3],
            radius: t.radius.control,
        }
    }
}

/// Cely panel: polozky pod sebou bez mezer (kazda ma sve odsazeni).
pub fn panel<M>(items: Vec<Node<M>>) -> Node<M> {
    column(items).gap(0)
}

/// Hlavicka: ikona, tucny nazev a vpravo souhrn (`.pnl header`).
pub fn panel_header<M>(s: &PanelSize, icon_name: &'static str, title: &str, summary: &str) -> Node<M> {
    row([icon(icon_name), label(title).tone(TextTone::Strong), spacer(), label(summary).tone(TextTone::Small)]).gap(s.gap).pad(s.edge, s.side, s.s3, s.side)
}

/// Nadpis sekce (`.pnl .sec`): male velke pismo, `ink_muted`.
pub fn panel_section<M>(s: &PanelSize, text: &str) -> Node<M> {
    row([label(&text.to_uppercase()).tone(TextTone::Section)]).pad(s.gap, s.side, s.s1, s.side)
}

/// Stavovy radek (`.pnl .state`): ikona a text v plose `surface_3`
/// (varovani `danger_soft`).
pub fn panel_state<M>(s: &PanelSize, icon_name: &'static str, text: &str, warn: bool) -> Node<M> {
    let inner = row([icon(icon_name), label(text).tone(if warn { TextTone::Danger } else { TextTone::Small }).grow()])
        .gap(s.s3)
        .pad(s.s3, s.gap, s.s3, s.gap)
        .deco(Deco { fill: Some(if warn { Fill::DangerSoft } else { Fill::Surface3 }), radius: s.radius, top_border: false });
    column([inner]).pad(s.s2, s.side, s.s2, s.side)
}

/// Pata (`.pnl .foot`): cara nahore, tlacitka vedle sebe.
pub fn panel_footer<M>(s: &PanelSize, children: Vec<Node<M>>) -> Node<M> {
    row(children).gap(s.s3).pad(s.gap, s.side, s.edge, s.side).deco(Deco { fill: None, radius: 0, top_border: true })
}

/// Obal radku panelu s vlastnim obsahem (posuvnik s tlacitkem): odsazeni
/// jako `.pnl .row`.
pub fn panel_line<M>(s: &PanelSize, children: Vec<Node<M>>) -> Node<M> {
    row(children).gap(s.gap).pad(s.s2, s.side, s.s2, s.side)
}

/// Pismo stitku podle tonu.
pub(crate) fn tone_style(t: &crate::Look, tone: TextTone) -> theme::TextStyle {
    let x = &t.theme.text;
    match tone {
        TextTone::Normal | TextTone::Muted => x.body,
        TextTone::Small => x.caption,
        TextTone::Danger => x.small,
        TextTone::Section => theme::TextStyle { weight: x.body_strong.weight, ..x.caption },
        TextTone::Strong => x.body_strong,
    }
}

/// Barva stitku podle tonu.
pub(crate) fn tone_color(c: &theme::Colors, tone: TextTone) -> u32 {
    match tone {
        TextTone::Normal | TextTone::Strong => c.ink,
        TextTone::Muted | TextTone::Small | TextTone::Section => c.ink_muted,
        TextTone::Danger => c.danger,
    }
}

/// Vypln a cara kontejneru.
pub(crate) fn draw_deco(c: &mut gfx::Canvas<'_>, r: gfx::Rect, d: &Deco, col: &theme::Colors) {
    match d.fill {
        Some(Fill::Surface3) => c.round_fill(r, d.radius, col.surface_3),
        Some(Fill::DangerSoft) => c.round_fill(r, d.radius, col.danger_soft),
        None => {}
    }
    if d.top_border {
        c.fill(gfx::Rect::new(r.x, r.y, r.w, 1), col.border);
    }
}

/// Velikost radku panelu (prvek s [`RowLook`]) pri sirce `avail`: cela
/// sirka, vyska nejmene `size.panel_row`.
pub(crate) fn row_size<M>(t: &crate::Look, kind: &Kind<M>, rl: &RowLook, avail: u32) -> (u32, u32) {
    let th = &t.theme;
    let text = match kind {
        Kind::Button { text, .. } | Kind::Radio { text, .. } | Kind::Switch { text, .. } | Kind::Label { text, .. } => text.as_str(),
        _ => "",
    };
    let s = PanelSize::of(th);
    let body = th.text.body;
    let small = th.text.caption;
    let lines = body.line + if rl.desc.is_empty() { 0 } else { small.line };
    let trailing = match kind {
        Kind::Switch { .. } => th.size.switch_w + s.gap,
        Kind::Radio { .. } => th.size.icon_m + s.gap,
        _ => 0,
    };
    let icon = rl.icon.map_or(0, |_| th.size.icon_m + s.gap);
    let w = 2 * s.side + icon + t.width(text, body).max(t.width(&rl.desc, small)) + trailing;
    (w.max(avail), th.size.panel_row.max(lines + 2 * s.s2))
}

impl<M: Clone> crate::Ui<M> {
    /// Radek panelu: najeti `surface_3`, ikona, nazev, popis, vpravo
    /// fajfka (zvolena volba) nebo prepinac; fokus jako vnitrni ramecek.
    pub(crate) fn draw_panel_row(&mut self, c: &mut gfx::Canvas<'_>, i: usize, st: crate::State) {
        use crate::layout::ElemKind;
        let th = self.look.theme.clone();
        let col = &th.color;
        let s = PanelSize::of(&th);
        let e = &self.elems[i];
        let r = e.rect;
        let Some(rl) = e.row.clone() else { return };
        let (text, active, check, switch) = match &e.kind {
            ElemKind::Button { text, .. } => (text.clone(), true, None, None),
            ElemKind::Radio { text, checked, .. } => (text.clone(), true, Some(*checked), None),
            ElemKind::Switch { text, on, .. } => (text.clone(), true, None, Some(*on)),
            ElemKind::Label { text, .. } => (text.clone(), false, None, None),
            _ => return,
        };
        let k = if st.disabled { th.opacity.disabled } else { 255 };
        let fade = |x: u32| crate::draw::fade(x, col.surface, k);
        if active && (st.hover || st.pressed) && !st.disabled {
            c.fill(r, col.surface_3);
        }
        if active && st.focus_visible {
            let ring = th.size.focus_ring;
            let inset = gfx::Rect::new(r.x + ring as i32, r.y + ring as i32, r.w.saturating_sub(2 * ring), r.h.saturating_sub(2 * ring));
            c.round_stroke(inset, s.radius, ring, col.focus);
        }
        let mut x = r.x + s.side as i32;
        let im = th.size.icon_m;
        if let Some(n) = rl.icon {
            self.icons.draw(c, n, x, r.y + (r.h as i32 - im as i32) / 2, im, th.icon_stroke, fade(col.ink));
            x += (im + s.gap) as i32;
        }
        // Vpravo: fajfka zvolene volby nebo prepinac.
        let mut right = r.right() as i32 - s.side as i32;
        if check == Some(true) {
            let d = im;
            let chip = gfx::Rect::new(right - d as i32, r.y + (r.h as i32 - d as i32) / 2, d, d);
            c.round_fill(chip, d / 2, fade(col.accent_soft));
            let is = th.size.icon_xs;
            self.icons.draw(c, "check", chip.x + (d - is) as i32 / 2, chip.y + (d - is) as i32 / 2, is, th.icon_stroke, fade(col.accent_ink));
        }
        if check.is_some() {
            right -= (im + s.gap) as i32;
        }
        if let Some(on) = switch {
            let (sw, sh) = (th.size.switch_w, th.size.switch_h);
            let tr = gfx::Rect::new(right - sw as i32, r.y + (r.h as i32 - sh as i32) / 2, sw, sh);
            let (bg, edge, knob) = if on { (col.accent, col.accent, col.on_accent) } else { (col.surface_3, col.border_strong, col.ink_muted) };
            c.round_fill_bordered(tr, sh / 2, fade(bg), fade(edge), 1);
            let kn = th.size.switch_knob;
            let inset = (sh - kn) / 2;
            let kx = if on { tr.right() as i32 - (kn + inset) as i32 } else { tr.x + inset as i32 };
            c.round_fill(gfx::Rect::new(kx, tr.y + inset as i32, kn, kn), kn / 2, fade(knob));
            right -= (sw + s.gap) as i32;
        }
        // Nazev (zvolena volba tucne) a popis pod nim.
        let room = (right - x).max(0) as u32;
        let title = if check == Some(true) { th.text.body_strong } else { th.text.body };
        let small = th.text.caption;
        let block = title.line + if rl.desc.is_empty() { 0 } else { small.line };
        let top = r.y + (r.h as i32 - block as i32) / 2;
        let f = self.look.face(title);
        let b = top + f.baseline_in(title.size, title.line);
        f.draw_ellipsized(c, x, b, &text, title.size, fade(col.ink), room);
        if !rl.desc.is_empty() {
            let f = self.look.face(small);
            let b = top + title.line as i32 + f.baseline_in(small.size, small.line);
            f.draw_ellipsized(c, x, b, &rl.desc, small.size, fade(col.ink_muted), room);
        }
    }
}
