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
use crate::node::{Kind, Node};

/// Prvek po rozvrzeni.
pub enum ElemKind<M> {
    Label { text: String, heading: bool, warn: bool },
    Button { text: String, on_click: Option<M> },
    TextInput { value: String, placeholder: String, on_change: Option<Box<dyn Fn(String) -> M>>, on_submit: Option<M> },
    Checkbox { text: String, checked: bool, on_toggle: Option<Box<dyn Fn(bool) -> M>> },
    List { count: usize, on_select: Option<Box<dyn Fn(usize) -> M>> },
    ListItem { text: String, index: usize, selected: bool, list: usize },
    Progress { value: u32 },
    Group,
    Scroll { content_h: u32 },
}

pub struct Element<M> {
    pub kind: ElemKind<M>,
    /// Obdelnik v souradnicich prostoru `space`.
    pub rect: Rect,
    /// 0 = okno, jinak index prvku `Scroll` + 1.
    pub space: usize,
    /// Rodic pro strom pristupnosti (index + 1, 0 = okno).
    pub parent: usize,
    pub disabled: bool,
}

impl<M> Element<M> {
    pub fn focusable(&self) -> bool {
        !self.disabled
            && matches!(
                self.kind,
                ElemKind::Button { .. } | ElemKind::TextInput { .. } | ElemKind::Checkbox { .. } | ElemKind::List { .. }
            )
    }
}

/// Prirozena velikost uzlu (rozmery z tokenu).
fn measure<M>(n: &Node<M>, t: &Look) -> (u32, u32) {
    let th = &t.theme;
    let sp = &th.space;
    let ctl = t.control_h();
    let (w, h) = match &n.kind {
        Kind::Label { text, heading, warn } => {
            let s = if *heading { t.heading() } else { t.body() };
            if *warn {
                // Stitek: pilulka s okrajem `s4`.
                (t.width(text, s) + 2 * sp[4], s.line + 2 * sp[2])
            } else {
                (t.width(text, s), s.line)
            }
        }
        Kind::Button { text, .. } => ((t.width(text, t.button()) + 2 * sp[5]).max(th.size.hit_min * 2), ctl),
        Kind::TextInput { .. } => (th.size.hit_min * 6, ctl),
        Kind::Checkbox { text, .. } => (th.size.icon_m + sp[3] + t.width(text, t.body()), ctl.min(th.size.hit_min)),
        Kind::List { items, .. } => (th.size.hit_min * 4, items.len() as u32 * t.row_h() + 2 * sp[2]),
        Kind::Progress { .. } => (th.size.hit_min * 4, sp[3]),
        Kind::Column(ch) => ch.iter().fold((0, 0), |(w, h), c| {
            let (cw, chh) = measure(c, t);
            (w.max(cw), h + chh + if h > 0 { t.gap() } else { 0 })
        }),
        Kind::Row(ch) => ch.iter().fold((0, 0), |(w, h), c| {
            let (cw, chh) = measure(c, t);
            (w + cw + if w > 0 { t.gap() } else { 0 }, h.max(chh))
        }),
        Kind::Scroll(c) => (measure(c, t).0, th.size.hit_min * 4),
        Kind::Spacer => (0, 0),
    };
    (if n.width > 0 { n.width } else { w }, if n.height > 0 { n.height } else { h })
}

/// Rozvrhne strom do obdelniku `area` (souradnice okna).
pub fn layout<M>(root: Node<M>, area: Rect, t: &Look) -> Vec<Element<M>> {
    let mut out = Vec::new();
    place(root, area, 0, 0, t, &mut out);
    out
}

fn place<M>(n: Node<M>, r: Rect, space: usize, parent: usize, t: &Look, out: &mut Vec<Element<M>>) {
    let disabled = n.disabled;
    let push = |out: &mut Vec<Element<M>>, kind: ElemKind<M>, rect: Rect| {
        out.push(Element { kind, rect, space, parent, disabled });
        out.len()
    };
    match n.kind {
        Kind::Label { text, heading, warn } => {
            push(out, ElemKind::Label { text, heading, warn }, r);
        }
        Kind::Button { text, on_click } => {
            push(out, ElemKind::Button { text, on_click }, r);
        }
        Kind::TextInput { value, placeholder, on_change, on_submit } => {
            push(out, ElemKind::TextInput { value, placeholder, on_change, on_submit }, r);
        }
        Kind::Checkbox { text, checked, on_toggle } => {
            push(out, ElemKind::Checkbox { text, checked, on_toggle }, r);
        }
        Kind::List { items, selected, on_select } => {
            let id = push(out, ElemKind::List { count: items.len(), on_select }, r);
            let (rh, inset) = (t.row_h(), t.theme.space[2]);
            for (i, text) in items.into_iter().enumerate() {
                let ir = Rect::new(r.x + inset as i32, r.y + (inset + i as u32 * rh) as i32, r.w.saturating_sub(2 * inset), rh);
                out.push(Element {
                    kind: ElemKind::ListItem { text, index: i, selected: selected == Some(i), list: id },
                    rect: ir,
                    space,
                    parent: id,
                    disabled,
                });
            }
        }
        Kind::Progress { value } => {
            push(out, ElemKind::Progress { value }, r);
        }
        Kind::Spacer => {}
        Kind::Column(children) => {
            let id = push(out, ElemKind::Group, r);
            let sizes: Vec<(u32, u32)> = children.iter().map(|c| measure(c, t)).collect();
            let fixed: u32 = sizes.iter().map(|s| s.1).sum::<u32>() + t.gap() * children.len().saturating_sub(1) as u32;
            let grow = children.iter().filter(|c| c.grow).count() as u32;
            let extra = r.h.saturating_sub(fixed);
            let mut y = r.y;
            for (c, (_, h)) in children.into_iter().zip(sizes) {
                let h = if c.grow && grow > 0 { h + extra / grow } else { h };
                place(c, Rect::new(r.x, y, r.w, h), space, id, t, out);
                y += (h + t.gap()) as i32;
            }
        }
        Kind::Row(children) => {
            let id = push(out, ElemKind::Group, r);
            let sizes: Vec<(u32, u32)> = children.iter().map(|c| measure(c, t)).collect();
            let fixed: u32 = sizes.iter().map(|s| s.0).sum::<u32>() + t.gap() * children.len().saturating_sub(1) as u32;
            let grow = children.iter().filter(|c| c.grow).count() as u32;
            let extra = r.w.saturating_sub(fixed);
            let mut x = r.x;
            for (c, (w, h)) in children.into_iter().zip(sizes) {
                let w = if c.grow && grow > 0 { w + extra / grow } else { w };
                // Svisle na stred radku.
                let y = r.y + (r.h.saturating_sub(h) / 2) as i32;
                place(c, Rect::new(x, y, w, h.min(r.h)), space, id, t, out);
                x += (w + t.gap()) as i32;
            }
        }
        Kind::Scroll(child) => {
            let (_, ch) = measure(&child, t);
            let id = push(out, ElemKind::Scroll { content_h: ch }, r);
            // Obsah ve vlastnim prostoru (souradnice od 0).
            place(*child, Rect::new(0, 0, r.w, ch.max(r.h)), id, id, t, out);
        }
    }
}
