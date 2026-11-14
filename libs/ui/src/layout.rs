//! Rozvrzeni: strom uzlu -> plochy seznam prvku s obdelniky.
//!
//! Sloupec sklada deti pod sebe (plna sirka), radek vedle sebe; zbyvajici
//! misto dostanou deti s `grow`. Posouvany obsah (`scroll`) ma vlastni
//! "prostor" (souradnice obsahu), ktery kompozitor posouva sam.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use guiproto::Rect;

use crate::look::Look;
use crate::node::{Key, Kind, Node};

/// Prvek po rozvrzeni.
pub enum ElemKind<M> {
    /// `lines` = nejvys radku (0 = bez omezeni); radky se pocitaji pri
    /// kresleni ze sirky obdelniku (`wrap.rs`).
    Label { text: String, heading: bool, warn: bool, lines: u32 },
    Button { text: String, on_click: Option<M>, icon: Option<&'static str> },
    TextInput { value: String, placeholder: String, secure: bool, multiline: bool, on_change: Option<Box<dyn Fn(String) -> M>>, on_submit: Option<M>, key_map: Option<crate::node::KeyMapFn>, suggest: Option<alloc::boxed::Box<crate::suggest::Suggest<M>>> },
    Checkbox { text: String, checked: bool, on_toggle: Option<Box<dyn Fn(bool) -> M>> },
    List { count: usize, on_select: Option<Box<dyn Fn(usize) -> M>> },
    ListItem { text: String, index: usize, selected: bool, list: usize, agent: bool },
    Progress { value: u32 },
    Table(Box<crate::node::TableSpec<M>>),
    Switch { text: String, on: bool, on_toggle: Option<Box<dyn Fn(bool) -> M>> },
    Radio { text: String, checked: bool, on_click: Option<M> },
    Slider { value: i32, min: i32, max: i32, step: i32, on_change: Option<Box<dyn Fn(i32) -> M>> },
    Segmented { options: Vec<String>, selected: usize, on_select: Option<Box<dyn Fn(usize) -> M>> },
    Tabs { labels: Vec<String>, selected: usize, on_select: Option<Box<dyn Fn(usize) -> M>>, on_close: Option<Box<dyn Fn(usize) -> M>> },
    Busy,
    Segments { items: Vec<crate::node::Segment>, legend: bool },
    Select { options: Vec<String>, selected: usize, on_select: Option<Box<dyn Fn(usize) -> M>> },
    Image { w: u32, h: u32, px: alloc::rc::Rc<[u32]>, alt: String },
    Group,
    Scroll { content_h: u32 },
}

pub struct Element<M> {
    pub kind: ElemKind<M>,
    /// Stabilni klic (jedinecny v okne, viz [`Key`]).
    pub key: Key,
    pub error: Option<String>,
    pub skip_tab: bool,
    pub menu: Option<Vec<crate::node::MenuEntry<M>>>,
    pub tooltip: Option<String>,
    pub focus_group: Option<crate::spatial::Axis>,
    pub default_focus: bool,
    /// Obdelnik v souradnicich prostoru `space`.
    pub rect: Rect,
    /// 0 = okno, jinak index prvku `Scroll` + 1.
    pub space: usize,
    /// Rodic pro strom pristupnosti (index + 1, 0 = okno).
    pub parent: usize,
    pub disabled: bool,
    /// Patri agentovi AI.
    pub agent: bool,
}

impl<M> Element<M> {
    pub fn focusable(&self) -> bool {
        !self.disabled
            && matches!(
                self.kind,
                ElemKind::Button { .. }
                    | ElemKind::TextInput { .. }
                    | ElemKind::Checkbox { .. }
                    | ElemKind::List { .. }
                    | ElemKind::Table(_)
                    | ElemKind::Switch { .. }
                    | ElemKind::Radio { .. }
                    | ElemKind::Slider { .. }
                    | ElemKind::Segmented { .. }
                    | ElemKind::Tabs { .. }
                    | ElemKind::Select { .. }
            )
    }
}

/// Druh uzlu pro odvozeny klic.
fn kind_tag<M>(k: &Kind<M>) -> usize {
    match k {
        Kind::Label { .. } => 0,
        Kind::Button { .. } => 1,
        Kind::TextInput { .. } => 2,
        Kind::Checkbox { .. } => 3,
        Kind::List { .. } => 4,
        Kind::Progress { .. } => 5,
        Kind::Column(_) => 6,
        Kind::Row(_) => 7,
        Kind::Scroll(_) => 8,
        Kind::Spacer => 9,
        Kind::Table(_) => 10,
        Kind::Switch { .. } => 11,
        Kind::Radio { .. } => 12,
        Kind::Slider { .. } => 13,
        Kind::Segmented { .. } => 14,
        Kind::Tabs { .. } => 15,
        Kind::Busy => 16,
        Kind::Segments { .. } => 17,
        Kind::Select { .. } => 18,
        Kind::Image { .. } => 19,
    }
}

const TAGS: usize = 20;

/// Znacky odvozenych klicu (aby se nepotkaly s vlastnimi).
const OWN: u64 = 1;
const AUTO: u64 = 2;
/// Radek seznamu: klic seznamu + znacka + index radku.
pub const ITEM: u64 = 3;
const DUP: u64 = 4;

/// Klic ditete: vlastni klic v ramci rodice, jinak druh a poradi mezi
/// sourozenci stejneho druhu.
fn child_key<M>(n: &Node<M>, parent: Key, counts: &mut [u64; TAGS]) -> Key {
    match n.key {
        Some(k) => parent.mix(OWN).mix(k.0),
        None => {
            let t = kind_tag(&n.kind);
            let o = counts[t];
            counts[t] += 1;
            parent.mix(AUTO).mix(t as u64).mix(o)
        }
    }
}

/// Radky stitku pri sirce `width` (`max` = nejvys radku, 0 = bez omezeni).
pub(crate) fn label_lines(t: &Look, text: &str, s: theme::TextStyle, width: u32, max: u32) -> Vec<String> {
    crate::wrap::wrap(text, width, max as usize, &|x| t.width(x, s))
}

/// Velikost obycejneho stitku pri dostupne sirce `avail`: jeden radek,
/// kdyz se vejde, jinak zalomeny (vyska podle poctu radku).
fn label_size(t: &Look, text: &str, s: theme::TextStyle, avail: u32, max: u32) -> (u32, u32) {
    let tw = t.width(text, s);
    if !text.contains('\n') && (tw <= avail || max == 1) {
        return (tw.min(avail), s.line);
    }
    let ls = label_lines(t, text, s, avail, max);
    let w = ls.iter().map(|l| t.width(l, s)).max().unwrap_or(0).min(avail);
    (w, ls.len() as u32 * s.line)
}

/// Prirozena velikost uzlu (rozmery z tokenu) pri dostupne sirce `avail`
/// (zalamovane stitky; sirka nikdy nepresahne `avail`, kdyz jde zalomit).
fn measure<M>(n: &Node<M>, t: &Look, avail: u32) -> (u32, u32) {
    let avail = if n.width > 0 { n.width } else { avail };
    let th = &t.theme;
    let sp = &th.space;
    let ctl = t.control_h();
    let (w, h) = match &n.kind {
        Kind::Label { text, heading, warn, lines } => {
            let s = if *heading { t.heading() } else { t.body() };
            if n.agent {
                // Stitek agenta: ikona a text v pilulce.
                (t.width(text, t.chip()) + 2 * sp[4] + th.size.icon_xs + sp[2], s.line + 2 * sp[2])
            } else if *warn {
                // Stitek: pilulka s okrajem `s4`.
                (t.width(text, s) + 2 * sp[4], s.line + 2 * sp[2])
            } else {
                label_size(t, text, s, avail, *lines)
            }
        }
        // Jen ikona: ctverec `control_h`; jinak text (a ikona) s okraji.
        Kind::Button { text, icon, .. } if text.is_empty() && icon.is_some() => (ctl, ctl),
        Kind::Button { text, icon, .. } => ((t.width(text, t.button()) + 2 * sp[5] + icon.map_or(0, |_| th.size.icon_m + sp[2])).max(th.size.hit_min * 2), ctl),
        Kind::TextInput { multiline, rows, .. } => {
            let h = if *multiline { rows * t.body().line + 2 * sp[2] } else { ctl };
            // Text chyby pod polem.
            let err = if n.error.is_some() { th.text.caption.line + sp[1] } else { 0 };
            (th.size.hit_min * 6, h + err)
        }
        Kind::Checkbox { text, .. } => (th.size.icon_m + sp[3] + t.width(text, t.body()), ctl.min(th.size.hit_min)),
        Kind::List { items, .. } => (th.size.hit_min * 4, items.len() as u32 * t.row_h() + 2 * sp[2]),
        Kind::Progress { .. } => (th.size.hit_min * 4, sp[3]),
        // Tabulka: osm radku a zahlavi (pevna vyska `height`, jinak roste).
        Kind::Table(tb) => (th.size.hit_min * 6, (8 + tb.header as u32) * t.row_h() + 2),
        Kind::Switch { text, .. } => (th.size.switch_w + sp[3] + t.width(text, t.body()), ctl.min(th.size.hit_min)),
        Kind::Radio { text, .. } => (th.size.icon_m + sp[3] + t.width(text, t.body()), ctl.min(th.size.hit_min)),
        Kind::Slider { .. } => (th.size.hit_min * 5, th.size.hit_min.min(ctl)),
        Kind::Segmented { options, .. } => {
            let w: u32 = options.iter().map(|o| t.width(o, t.body()) + 2 * sp[4]).sum();
            (w + 2 * 3 + options.len().saturating_sub(1) as u32 * 2, ctl)
        }
        Kind::Tabs { .. } => (th.size.hit_min * 4, ctl),
        Kind::Busy => (th.size.hit_min * 4, sp[3]),
        Kind::Image { w, h, .. } => (*w, *h),
        Kind::Select { options, .. } => {
            let w = options.iter().map(|o| t.width(o, t.body())).max().unwrap_or(0);
            (w + 2 * sp[4] + th.size.icon_s + sp[2], ctl)
        }
        // Pruh 44 px (`.dbar`) a radek legendy.
        Kind::Segments { legend, .. } => (th.size.hit_min * 6, ctl + sp[2] + if *legend { sp[2] + th.text.caption.line } else { 0 }),
        Kind::Column(ch) => ch.iter().fold((0, 0), |(w, h), c| {
            let (cw, chh) = measure(c, t, avail);
            (w.max(cw), h + chh + if h > 0 { t.gap() } else { 0 })
        }),
        Kind::Row(ch) => {
            // Sirka bez rozdeleni zbytku (`grow`), vyska pri sirkach po nem
            // (zalomeny stitek s `grow` ma radku mene).
            let natural = row_sizes(ch, avail, t, false);
            let w = natural.iter().map(|s| s.0).sum::<u32>() + t.gap() * ch.len().saturating_sub(1) as u32;
            (w, row_sizes(ch, avail, t, true).iter().map(|s| s.1).max().unwrap_or(0))
        }
        Kind::Scroll(c) => (measure(c, t, avail).0, th.size.hit_min * 4),
        Kind::Spacer => (0, 0),
    };
    (if n.width > 0 { n.width } else { w }, if n.height > 0 { n.height } else { h })
}

/// Je uzel zalamovany stitek (smi se v radku zuzit)?
fn wraps<M>(n: &Node<M>) -> bool {
    matches!(n.kind, Kind::Label { warn: false, lines, .. } if lines != 1) && !n.agent && n.width == 0
}

/// Sirky a vysky deti radku sirky `avail`. Kdyz se nevejdou, zuzi se
/// zalamovane stitky (umerne sve sirce); s `grow` dostanou deti s `grow`
/// zbytek sirky. Vyska zalomenych stitku je pro jejich konecnou sirku.
fn row_sizes<M>(ch: &[Node<M>], avail: u32, t: &Look, grow: bool) -> Vec<(u32, u32)> {
    let mut sizes: Vec<(u32, u32)> = ch.iter().map(|c| measure(c, t, avail)).collect();
    let gaps = t.gap() * ch.len().saturating_sub(1) as u32;
    let total = sizes.iter().map(|s| s.0).sum::<u32>() + gaps;
    let mut changed = alloc::vec![false; ch.len()];
    if total > avail {
        let over = (total - avail) as u64;
        let wrap_w: u64 = ch.iter().zip(&sizes).filter(|(c, _)| wraps(c)).map(|(_, s)| s.0 as u64).sum();
        if wrap_w > 0 {
            for (k, c) in ch.iter().enumerate() {
                if wraps(c) {
                    let cut = (over * sizes[k].0 as u64).div_ceil(wrap_w) as u32;
                    sizes[k].0 = sizes[k].0.saturating_sub(cut).max(1);
                    changed[k] = true;
                }
            }
        }
    }
    if grow {
        let fixed = sizes.iter().map(|s| s.0).sum::<u32>() + gaps;
        let n = ch.iter().filter(|c| c.grow).count() as u32;
        let extra = avail.saturating_sub(fixed);
        if n > 0 {
            for (k, c) in ch.iter().enumerate() {
                if c.grow {
                    sizes[k].0 += extra / n;
                    changed[k] = true;
                }
            }
        }
    }
    // Vyska pri konecne sirce (jen zalamovane stitky na ni zavisi).
    for (k, c) in ch.iter().enumerate() {
        if changed[k] && wraps(c) {
            sizes[k].1 = measure(c, t, sizes[k].0).1;
        }
    }
    sizes
}

/// Rozvrhne strom do obdelniku `area` (souradnice okna).
///
/// Klice jsou v okne jedinecne: dva sourozenci se stejnym vlastnim klicem
/// (chyba aplikace) dostanou druhy klic odvozeny z poradi, aby se jejich
/// stav nemichal.
pub fn layout<M>(root: Node<M>, area: Rect, t: &Look) -> Vec<Element<M>> {
    let mut out = Vec::new();
    let key = root.key.unwrap_or(Key::ROOT);
    place(root, key, area, 0, 0, t, &mut out);
    let mut seen = alloc::collections::BTreeSet::new();
    for (i, e) in out.iter_mut().enumerate() {
        if !seen.insert(e.key) {
            e.key = e.key.mix(DUP).mix(i as u64);
            seen.insert(e.key);
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn place<M>(n: Node<M>, key: Key, r: Rect, space: usize, parent: usize, t: &Look, out: &mut Vec<Element<M>>) {
    let disabled = n.disabled;
    let agent = n.agent;
    let skip_tab = n.skip_tab;
    let mut error = n.error;
    let mut menu = n.menu;
    let mut tooltip = n.tooltip;
    let (focus_group, default_focus) = (n.focus_group, n.default_focus);
    let mut push = |out: &mut Vec<Element<M>>, kind: ElemKind<M>, rect: Rect| {
        out.push(Element { kind, key, error: error.take(), skip_tab, menu: menu.take(), tooltip: tooltip.take(), focus_group, default_focus, rect, space, parent, disabled, agent });
        out.len()
    };
    match n.kind {
        Kind::Label { text, heading, warn, lines } => {
            push(out, ElemKind::Label { text, heading, warn, lines }, r);
        }
        Kind::Button { text, on_click, icon } => {
            push(out, ElemKind::Button { text, on_click, icon }, r);
        }
        Kind::TextInput { value, placeholder, secure, multiline, on_change, on_submit, key_map, suggest, .. } => {
            push(out, ElemKind::TextInput { value, placeholder, secure, multiline, on_change, on_submit, key_map, suggest }, r);
        }
        Kind::Checkbox { text, checked, on_toggle } => {
            push(out, ElemKind::Checkbox { text, checked, on_toggle }, r);
        }
        Kind::List { items, selected, on_select, agents } => {
            let id = push(out, ElemKind::List { count: items.len(), on_select }, r);
            let (rh, inset) = (t.row_h(), t.theme.space[2]);
            for (i, text) in items.into_iter().enumerate() {
                let ir = Rect::new(r.x + inset as i32, r.y + (inset + i as u32 * rh) as i32, r.w.saturating_sub(2 * inset), rh);
                out.push(Element {
                    kind: ElemKind::ListItem { text, index: i, selected: selected == Some(i), list: id, agent: agents.get(i).copied().unwrap_or(false) },
                    key: key.mix(ITEM).mix(i as u64),
                    error: None,
                    skip_tab: true,
                    menu: None,
                    tooltip: None,
                    focus_group: None,
                    default_focus: false,
                    rect: ir,
                    space,
                    parent: id,
                    disabled,
                    agent: false,
                });
            }
        }
        Kind::Progress { value } => {
            push(out, ElemKind::Progress { value }, r);
        }
        Kind::Table(t) => {
            push(out, ElemKind::Table(t), r);
        }
        Kind::Switch { text, on, on_toggle } => {
            push(out, ElemKind::Switch { text, on, on_toggle }, r);
        }
        Kind::Radio { text, checked, on_click } => {
            push(out, ElemKind::Radio { text, checked, on_click }, r);
        }
        Kind::Slider { value, min, max, step, on_change } => {
            push(out, ElemKind::Slider { value, min, max, step, on_change }, r);
        }
        Kind::Segmented { options, selected, on_select } => {
            push(out, ElemKind::Segmented { options, selected, on_select }, r);
        }
        Kind::Tabs { labels, selected, on_select, on_close } => {
            push(out, ElemKind::Tabs { labels, selected, on_select, on_close }, r);
        }
        Kind::Busy => {
            push(out, ElemKind::Busy, r);
        }
        Kind::Segments { items, legend } => {
            push(out, ElemKind::Segments { items, legend }, r);
        }
        Kind::Select { options, selected, on_select } => {
            push(out, ElemKind::Select { options, selected, on_select }, r);
        }
        Kind::Image { w, h, px, alt } => {
            push(out, ElemKind::Image { w, h, px, alt }, r);
        }
        Kind::Spacer => {}
        Kind::Column(children) => {
            let id = push(out, ElemKind::Group, r);
            let mut counts = [0; TAGS];
            let sizes: Vec<(u32, u32)> = children.iter().map(|c| measure(c, t, r.w)).collect();
            let fixed: u32 = sizes.iter().map(|s| s.1).sum::<u32>() + t.gap() * children.len().saturating_sub(1) as u32;
            let grow = children.iter().filter(|c| c.grow).count() as u32;
            let extra = r.h.saturating_sub(fixed);
            let mut y = r.y;
            for (c, (_, h)) in children.into_iter().zip(sizes) {
                let h = if c.grow && grow > 0 { h + extra / grow } else { h };
                let k = child_key(&c, key, &mut counts);
                place(c, k, Rect::new(r.x, y, r.w, h), space, id, t, out);
                y += (h + t.gap()) as i32;
            }
        }
        Kind::Row(children) => {
            let id = push(out, ElemKind::Group, r);
            let mut counts = [0; TAGS];
            // Konecne sirky (zuzeni zalamovanych stitku, zbytek pro `grow`).
            let sizes = row_sizes(&children, r.w, t, true);
            let mut x = r.x;
            for (c, (w, h)) in children.into_iter().zip(sizes) {
                // Svisle na stred radku.
                let y = r.y + (r.h.saturating_sub(h) / 2) as i32;
                let k = child_key(&c, key, &mut counts);
                place(c, k, Rect::new(x, y, w, h.min(r.h)), space, id, t, out);
                x += (w + t.gap()) as i32;
            }
        }
        Kind::Scroll(child) => {
            let (_, ch) = measure(&child, t, r.w);
            let id = push(out, ElemKind::Scroll { content_h: ch }, r);
            // Obsah ve vlastnim prostoru (souradnice od 0).
            let k = child_key(&child, key, &mut [0; TAGS]);
            place(*child, k, Rect::new(0, 0, r.w, ch.max(r.h)), id, id, t, out);
        }
    }
}
