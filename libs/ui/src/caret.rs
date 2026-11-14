//! Casovane chovani textovych poli (pres `Ui::next_deadline` a
//! `Ui::timer`):
//!
//! - **blikani kurzoru**: po cinnosti (psani, pohyb, klik) je kurzor cela
//!   perioda videt, pak blika s pulperiodou `motion.caret_blink`; po
//!   `motion.caret_blink_stop` bez cinnosti zustane videt a casovac se
//!   zastavi (zadne prekreslovani v klidu);
//! - **automaticky posun** pri tahu s vyberem za okraj pole: kazdych
//!   `motion.autoscroll` ms se vyber rozsiri dal (rychlost podle
//!   vzdalenosti ukazatele od okraje);
//! - **hledani s prodlevou**: `on_search` pole hledani az po
//!   `motion.search_delay` ms bez psani.

use crate::Ui;

const MS: u64 = 1_000_000;

impl<M: Clone> Ui<M> {
    /// Cinnost v poli: kurzor hned videt a blikani znovu od zacatku.
    pub(crate) fn caret_reset(&mut self) {
        self.caret_since = self.now;
        self.caret_on = true;
    }

    /// Cinnost bez casu udalosti (akce pristupnosti): `Ui::now` muze byt
    /// stary, zacatek blikani doplni nejblizsi `Ui::timer` (hned).
    pub(crate) fn caret_reset_untimed(&mut self) {
        self.caret_reset();
        self.caret_fresh = true;
    }

    /// Blika kurzor (pole s fokusem v aktivnim okne, ne nedostupne)?
    fn caret_blinking(&self) -> bool {
        self.active && self.focused_field().is_some_and(|(i, _)| !self.elems[i].disabled)
    }

    /// Ukazatel je mimo plochu textu pole, ve kterem tah vybira?
    fn autoscroll_pending(&self) -> bool {
        let Some(i) = self.drag_field.and_then(|k| self.index_of(k)) else { return false };
        if self.pressed.is_none() {
            return false;
        }
        let inner = self.field_inner(i);
        let (x, y) = self.to_space(i, self.pointer.0, self.pointer.1);
        let multi = self.field_kind(i).is_some_and(|k| k.0);
        if multi { y < inner.y || y >= inner.bottom() as i32 } else { x < inner.x || x >= inner.right() as i32 }
    }

    /// Nejblizsi casovana udalost poli (ns), `None` = nic.
    pub(crate) fn caret_deadline(&self) -> Option<u64> {
        let m = &self.look.theme.motion;
        let half = m.caret_blink as u64 * MS;
        let mut next: Option<u64> = None;
        let mut at = |t: u64| next = Some(next.map_or(t, |n| n.min(t)));
        if self.caret_fresh {
            at(self.now);
        }
        if self.caret_blinking() && half > 0 {
            let since = self.now.saturating_sub(self.caret_since);
            let stop = m.caret_blink_stop as u64 * MS;
            if since < stop {
                // Dalsi prepnuti: prvni po cele periode, pak po pulperiodach.
                let k = since / half + 1;
                at(self.caret_since + (k.max(2) * half).min(stop));
            }
        }
        if self.autoscroll_pending() {
            at(self.now + m.autoscroll.max(1) as u64 * MS);
        }
        if let Some((_, t)) = self.search_due {
            at(t);
        }
        next
    }

    /// Casovac poli v case `now`; vraci, zda prekreslit.
    pub(crate) fn caret_timer(&mut self, now: u64) -> bool {
        let mut redraw = false;
        if core::mem::take(&mut self.caret_fresh) {
            self.caret_since = now;
        }
        let m = self.look.theme.motion.clone();
        let half = m.caret_blink as u64 * MS;
        if self.caret_blinking() && half > 0 {
            let since = now.saturating_sub(self.caret_since);
            let on = since >= m.caret_blink_stop as u64 * MS || since < 2 * half || (since / half) % 2 == 1;
            // S vrstvou kurzoru staci prepnout jeji viditelnost (klient
            // podle `Ui::caret`), okno se neprekresluje.
            if on != self.caret_on {
                self.caret_on = on;
                redraw |= !self.caret_layer;
            }
        } else if !self.caret_on {
            self.caret_on = true;
            redraw |= !self.caret_layer;
        }
        if self.autoscroll_pending() {
            let (x, y) = self.pointer;
            redraw |= self.field_drag(x, y);
        }
        if let Some((k, t)) = self.search_due
            && now >= t
        {
            self.search_due = None;
            self.fire_search(k);
        }
        redraw
    }

    /// Zprava `on_search` pole `k` s jeho aktualnim textem.
    pub(crate) fn fire_search(&mut self, k: crate::Key) {
        let Some(i) = self.index_of(k) else { return };
        let t = alloc::string::String::from(self.field_text(i));
        if let crate::layout::ElemKind::TextInput { on_search: Some(f), .. } = &self.elems[i].kind {
            self.timer_msgs.push(f(t));
        }
    }

    /// Zpravy vznikle v casovaci (`Ui::timer`): klient je preda aplikaci.
    pub fn take_msgs(&mut self) -> alloc::vec::Vec<M> {
        core::mem::take(&mut self.timer_msgs)
    }
}
