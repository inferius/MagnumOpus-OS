//! Seznam navrhu u textoveho pole ve strome `Ui`:
//! `text_input(..).suggest(suggest("Cesty", polozky, Msg::Doplnit))`.
//!
//! Seznam je otevreny, kdyz ma pole fokus a aplikace dala aspon jednu
//! polozku. Stav (vyber, najeti, posun) drzi knihovna podle klice pole;
//! nove polozky (jiny obsah) vyber vrati na prvni pouzitelnou. Esc, prijeti
//! a stisk mimo seznam ho zavrou (`on_close`), dokud aplikace nepreda jine
//! polozky. Klavesy, ktere seznam nevezme, jdou do pole. Kresli se ve
//! vrstve popoveru (muze presahovat okno) jako nabidky. Pole s heslem
//! seznam navrhu nikdy nema.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use guiproto::a11y::{self, Node as A11yNode};
use guiproto::window::InputEvent;

use crate::complete::{Completion, Outcome, Paint, Side, Suggestion};
use crate::event::Response;
use crate::layout::ElemKind;
use crate::{Key, Ui};

/// Seznam navrhu pole (viz [`suggest`]).
pub struct Suggest<M> {
    pub(crate) header: String,
    pub(crate) items: Vec<Suggestion>,
    pub(crate) side: Side,
    pub(crate) on_accept: Box<dyn Fn(usize) -> M>,
    pub(crate) on_select: Option<Box<dyn Fn(usize) -> M>>,
    pub(crate) on_close: Option<M>,
}

/// Seznam navrhu se zahlavim `header` (prazdne = bez zahlavi);
/// `on_accept(i)` = prijata polozka `i` (Enter, Tab, klik, ctecka).
pub fn suggest<M>(header: &str, items: Vec<Suggestion>, on_accept: impl Fn(usize) -> M + 'static) -> Suggest<M> {
    Suggest { header: header.into(), items, side: Side::Below, on_accept: Box::new(on_accept), on_select: None, on_close: None }
}

impl<M> Suggest<M> {
    /// Seznam nad polem (radek vstupu dole v okne).
    pub fn above(mut self) -> Self {
        self.side = Side::Above;
        self
    }

    /// Zmena vybrane polozky (sipky; napr. nahled v poli).
    pub fn on_select(mut self, f: impl Fn(usize) -> M + 'static) -> Self {
        self.on_select = Some(Box::new(f));
        self
    }

    /// Uzivatel seznam zavrel (Esc, klik mimo, prijeti).
    pub fn on_close(mut self, m: M) -> Self {
        self.on_close = Some(m);
        self
    }
}

/// Otevreny (nebo zavreny uzivatelem) seznam pole `owner`.
pub(crate) struct SuggestState {
    pub(crate) owner: Key,
    pub(crate) comp: Completion,
    /// Otisk polozek od aplikace (jine polozky = novy seznam).
    sig: u64,
    /// Zavreny uzivatelem do zmeny polozek.
    pub(crate) dismissed: bool,
}

fn signature(header: &str, items: &[Suggestion]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |b: &[u8]| {
        for &x in b {
            h = (h ^ x as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
        h = (h ^ 0xff).wrapping_mul(0x0000_0100_0000_01b3);
    };
    eat(header.as_bytes());
    for it in items {
        eat(it.icon.as_bytes());
        eat(it.name.as_bytes());
        eat(it.detail.as_bytes());
        eat(it.disabled.as_deref().unwrap_or("\u{1}").as_bytes());
    }
    h
}

impl<M: Clone> Ui<M> {
    /// Je seznam navrhu videt?
    pub fn suggest_open(&self) -> bool {
        self.suggest.as_ref().is_some_and(|s| !s.dismissed)
    }

    /// Pismo jmen v seznamu navrhu (tokeny `font.mono`); bez nej pisma UI.
    pub fn set_mono_fonts(&mut self, f: gfx::text::Family) {
        self.mono = Some(f);
    }

    /// Seznam podle pole s fokusem a jeho polozek; prepocita polohu.
    pub(crate) fn sync_suggest(&mut self) {
        let target = self.focus.filter(|&i| !self.elems[i].disabled).and_then(|i| match &self.elems[i].kind {
            ElemKind::TextInput { suggest: Some(s), secure: false, .. } if !s.items.is_empty() => Some((i, signature(&s.header, &s.items))),
            _ => None,
        });
        let Some((i, sig)) = target else {
            self.suggest = None;
            return;
        };
        let key = self.elems[i].key;
        let fresh = !self.suggest.as_ref().is_some_and(|s| s.owner == key && s.sig == sig);
        if fresh && let ElemKind::TextInput { suggest: Some(s), .. } = &self.elems[i].kind {
            let comp = Completion::new(&s.header, s.items.clone());
            self.suggest = Some(SuggestState { owner: key, comp, sig, dismissed: false });
        }
        let side = match &self.elems[i].kind {
            ElemKind::TextInput { suggest: Some(s), .. } => s.side,
            _ => Side::Below,
        };
        let anchor = self.window_rect(i);
        let area = self.popup_area();
        let min_hit = self.look.min_hit;
        if let Some(st) = self.suggest.as_mut() {
            st.comp.anchor = anchor;
            st.comp.prefer = side;
            st.comp.place(&self.look.theme, min_hit, area);
        }
    }

    /// Udalost pro otevreny seznam. Zpravy jdou do `r`; vraci, zda ji
    /// seznam spotreboval (jinak obecne zpracovani).
    pub(crate) fn suggest_event(&mut self, ev: &InputEvent, r: &mut Response<M>) -> bool {
        if self.popup.is_some() {
            return false;
        }
        let Some(st) = self.suggest.as_mut().filter(|s| !s.dismissed) else { return false };
        let Some(i) = self.by_key.get(&st.owner).copied() else { return false };
        let before = st.comp.selected;
        // Klik do vlastniho pole (posun kurzoru) seznam necha otevreny.
        let field = self.window_rect(i);
        let Some(st) = self.suggest.as_mut() else { return false };
        let (out, inside) = match *ev {
            InputEvent::Button { x, y, .. } if field.contains(x, y) && !st.comp.rect().contains(x, y) => return false,
            InputEvent::Key { .. } if self.focus == Some(i) => (st.comp.key(ev, self.mods), false),
            InputEvent::PointerMove { x, y } | InputEvent::Button { x, y, .. } => (st.comp.pointer(ev), st.comp.rect().contains(x, y)),
            InputEvent::Scroll { .. } | InputEvent::PointerLeave => {
                let o = st.comp.pointer(ev);
                (o, o == Outcome::Moved)
            }
            InputEvent::Focus(false) => {
                st.dismissed = true;
                r.redraw = true;
                return false;
            }
            _ => return false,
        };
        let msg_close = |el: &ElemKind<M>| match el {
            ElemKind::TextInput { suggest: Some(s), .. } => s.on_close.clone(),
            _ => None,
        };
        match out {
            Outcome::Ignored => {}
            Outcome::Moved => {
                r.redraw = true;
                let now = st.comp.selected;
                if now != before
                    && let Some(k) = now
                    && let ElemKind::TextInput { suggest: Some(s), .. } = &self.elems[i].kind
                    && let Some(f) = &s.on_select
                {
                    r.msgs.push(f(k));
                }
            }
            Outcome::Accept(k) => {
                st.dismissed = true;
                r.redraw = true;
                if let ElemKind::TextInput { suggest: Some(s), .. } = &self.elems[i].kind {
                    r.msgs.push((s.on_accept)(k));
                    if let Some(m) = s.on_close.clone() {
                        r.msgs.push(m);
                    }
                }
                return true;
            }
            Outcome::Close => {
                st.dismissed = true;
                r.redraw = true;
                if let Some(m) = msg_close(&self.elems[i].kind) {
                    r.msgs.push(m);
                }
                // Esc patri seznamu; stisk mysi mimo jde dal.
                return matches!(ev, InputEvent::Key { .. });
            }
        }
        let consumed = inside || (matches!(ev, InputEvent::Key { .. }) && out != Outcome::Ignored);
        if consumed && inside && self.hover.take().is_some() {
            r.redraw = true;
        }
        consumed
    }

    /// Obdelnik otevreneho seznamu (souradnice okna).
    pub(crate) fn suggest_rect(&self) -> Option<gfx::Rect> {
        self.suggest.as_ref().filter(|s| !s.dismissed).map(|s| s.comp.rect()).filter(|r| !r.is_empty())
    }

    /// Nakresli otevreny seznam; pocatek platna = `tip_shift` (vrstva
    /// popoveru) nebo okno.
    pub(crate) fn draw_suggest(&mut self, c: &mut gfx::Canvas<'_>) {
        let Some(st) = self.suggest.as_ref().filter(|s| !s.dismissed) else { return };
        let mut p = Paint { theme: &self.look.theme, fonts: &mut self.look.fonts, mono: self.mono.as_mut(), icons: &mut self.icons };
        st.comp.draw(c, self.tip_shift, &mut p);
    }

    /// Stav pole pro pristupnost: `AUTOCOMPLETE`, otevreny seznam
    /// `EXPANDED`.
    pub(crate) fn suggest_state_bits(&self, i: usize) -> u32 {
        let ElemKind::TextInput { suggest: Some(_), secure: false, .. } = &self.elems[i].kind else { return 0 };
        let open = self.suggest.as_ref().is_some_and(|s| !s.dismissed && s.owner == self.elems[i].key);
        a11y::state::AUTOCOMPLETE | if open { a11y::state::EXPANDED } else { 0 }
    }

    /// Uzly seznamu pod polem `i` (id `field_id`).
    pub(crate) fn suggest_a11y(&self, i: usize, field_id: u32, out: &mut Vec<A11yNode>) {
        let Some(st) = self.suggest.as_ref().filter(|s| !s.dismissed && s.owner == self.elems[i].key) else { return };
        out.extend(st.comp.a11y(field_id, crate::popup::virt_id(field_id, 9, 0), |k| crate::popup::virt_id(field_id, 10, k as u64), crate::ROOT_LAYER));
    }

    /// Akce pristupnosti na polozce seznamu.
    pub(crate) fn suggest_a11y_action(&mut self, node: u32, action: u32, r: &mut Response<M>) -> bool {
        let Some(st) = self.suggest.as_mut().filter(|s| !s.dismissed) else { return false };
        let Some(i) = self.by_key.get(&st.owner).copied() else { return false };
        let fid = self.elem_ids[i];
        let Some(k) = (0..st.comp.items.len()).find(|&k| crate::popup::virt_id(fid, 10, k as u64) == node) else { return false };
        let before = st.comp.selected;
        match st.comp.a11y_action(k, action) {
            Outcome::Accept(k) => {
                st.dismissed = true;
                if let ElemKind::TextInput { suggest: Some(s), .. } = &self.elems[i].kind {
                    r.msgs.push((s.on_accept)(k));
                    if let Some(m) = s.on_close.clone() {
                        r.msgs.push(m);
                    }
                }
            }
            Outcome::Moved if st.comp.selected != before => {
                if let ElemKind::TextInput { suggest: Some(s), .. } = &self.elems[i].kind
                    && let Some(f) = &s.on_select
                {
                    r.msgs.push(f(k));
                }
            }
            _ => {}
        }
        r.redraw = true;
        true
    }
}
