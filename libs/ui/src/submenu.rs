//! Podnabidky a mnemoniky nabidek.
//!
//! - **Podnabidka** (`ui::submenu`): sipka vpravo, Enter, mezernik nebo
//!   najeti mysi ji otevre vedle polozky (vpravo, kdyz se nevejde,
//!   vlevo); sipka vlevo a Esc ji zavrou a vrati se do rodice; najeti na
//!   jinou polozku rodice podnabidky pod ni zavre. Vybrana polozka zavre
//!   celou nabidku. `Ui::popup` je vzdy nejhlubsi uroven, rodice drzi
//!   `Ui::popup_stack` (od korene).
//! - **Mnemonika**: `&` pred pismenem v textu polozky (`"&Kopírovat"`)
//!   pismeno podtrhne a stisk pismena polozku vybere (je-li jedina s tim
//!   pismenem, jinak se mezi nimi prepina); `&&` je znak `&`.

use alloc::string::String;

use gfx::Rect;

use crate::Ui;
use crate::popup::PAct;

/// Text polozky bez `&` a mnemonika (male pismeno, bajt v textu).
pub(crate) fn mnemonic(label: &str) -> (String, Option<(char, usize)>) {
    let mut out = String::with_capacity(label.len());
    let mut m = None;
    let mut it = label.chars().peekable();
    while let Some(c) = it.next() {
        if c == '&' {
            match it.peek().copied() {
                Some('&') => {
                    it.next();
                    out.push('&');
                }
                Some(n) if m.is_none() && !n.is_whitespace() => {
                    m = Some((n.to_lowercase().next().unwrap_or(n), out.len()));
                }
                _ => {}
            }
            continue;
        }
        out.push(c);
    }
    (out, m)
}

impl<M: Clone> Ui<M> {
    /// Otevre podnabidku polozky `k` nejhlubsi nabidky (prvni polozka
    /// aktivni, jako po sipce vpravo na ostatnich systemech).
    pub(crate) fn open_submenu(&mut self, k: usize) {
        let Some(mut parent) = self.popup.clone() else { return };
        let Some(PAct::Sub(items)) = parent.items.get(k).and_then(|i| i.act.clone()) else { return };
        if items.is_empty() {
            return;
        }
        parent.hi = Some(k);
        // Radek polozky v rodici (souradnice okna).
        let pad = self.look.theme.space[2] as i32;
        let top: i32 = parent.items[..k].iter().map(|i| self.row_h(i) as i32).sum();
        let row_y = parent.rect.y + pad - parent.scroll + top;
        let owner = parent.owner;
        self.popup_stack.push(parent.clone());
        self.open_popup(owner, false, items, Rect::new(parent.rect.x, row_y, 1, 1), 0, None);
        // Vedle rodice: vpravo, jinak vlevo; svisle u radku a v plose.
        let area = self.popup_area();
        if let Some(p) = self.popup.as_mut() {
            let (w, h) = (p.rect.w as i32, p.rect.h as i32);
            let right = parent.rect.right() as i32 - pad;
            let x = if right + w <= area.right() as i32 { right } else { (parent.rect.x + pad - w).max(area.x) };
            let y = (row_y - pad).min(area.bottom() as i32 - h).max(area.y);
            p.rect = Rect::new(x, y, p.rect.w, p.rect.h);
        }
        let first = self.step_item(None, 1);
        if let Some(p) = self.popup.as_mut() {
            p.hi = first;
        }
    }

    /// Zavre nejhlubsi podnabidku (navrat do rodice). `false` = neni.
    pub(crate) fn close_submenu(&mut self) -> bool {
        match self.popup_stack.pop() {
            Some(p) => {
                self.popup = Some(p);
                true
            }
            None => false,
        }
    }

    /// Ukazatel nad nekterou rodicovskou nabidkou (ne nad nejhlubsi):
    /// podnabidky pod ni se zavrou. Vraci, zda se neco zavrelo.
    pub(crate) fn submenu_pointer(&mut self, x: i32, y: i32) -> bool {
        if self.popup.as_ref().is_some_and(|p| p.rect.contains(x, y)) {
            return false;
        }
        let Some(level) = self.popup_stack.iter().rposition(|p| p.rect.contains(x, y)) else { return false };
        while self.popup_stack.len() > level {
            self.close_submenu();
        }
        true
    }

    /// Polozka s mnemonikou `c` v nejhlubsi nabidce: `Ok(k)` = jedina
    /// (vybrat), `Err(k)` = vic polozek (dalsi po `hi`), `None` = zadna.
    pub(crate) fn mnemonic_pick(&self, c: char, hi: Option<usize>) -> Option<Result<usize, usize>> {
        let p = self.popup.as_ref()?;
        let c = c.to_lowercase().next()?;
        let hits: alloc::vec::Vec<usize> = (0..p.items.len()).filter(|&k| p.items[k].usable() && p.items[k].mnemonic.is_some_and(|m| m.0 == c)).collect();
        match hits.len() {
            0 => None,
            1 => Some(Ok(hits[0])),
            _ => Some(Err(hits.iter().copied().find(|&k| hi.is_none_or(|h| k > h)).unwrap_or(hits[0]))),
        }
    }
}
