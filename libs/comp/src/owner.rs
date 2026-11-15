//! Vlastnik okna (`WINDOW_OWNER`, guiproto verze 7) a zavreni popoveru
//! stiskem mimo nej (`POPOVER_DISMISS`).
//!
//! - Vlastnik je jine okno **tehoz spojeni** (dialog aplikace nad jejim
//!   oknem, obdoba prechodneho okna ve Waylandu). Okno jineho spojeni
//!   (i stejne identity, napr. jiny proces Wine) vlastnikem byt nemuze:
//!   `NOT_SUPPORTED`.
//! - Vlastnene okno se pri prvnim zobrazeni vystredi nad vlastnikem
//!   (v pracovni plose) a pri kazdem zvednuti vlastnika jde nad nej.
//! - Stisk tlacitka mimo popovery okna s fokusem posle oknu
//!   `POPOVER_DISMISS` (jen spojeni verze 7+): aplikace popovery zavre jako
//!   nabidky systemu.

use guiproto::Error;
use guiproto::window::{ServerMsg, op};

use crate::{ClientId, Comp};

/// Nejvetsi hloubka retezce vlastniku (ochrana proti cyklu).
const MAX_DEPTH: usize = 16;

impl Comp {
    /// `WINDOW_OWNER { window, owner }` od klienta `c`.
    pub(crate) fn window_owner(&mut self, c: ClientId, window: u32, owner: u32) {
        let Some(cl) = self.clients.get(&c) else { return };
        if cl.version < 7 {
            return self.error(c, op::WINDOW_OWNER, Error::NotSupported);
        }
        let Some(&gid) = cl.windows.get(&window) else {
            return self.error(c, op::WINDOW_OWNER, Error::InvalidArgs);
        };
        if owner == 0 {
            if let Some(w) = self.windows.get_mut(&gid) {
                w.owner = None;
            }
            return;
        }
        // Jen okno tehoz spojeni (mistni id jineho spojeni sem nepatri).
        let Some(&og) = cl.windows.get(&owner) else {
            return self.error(c, op::WINDOW_OWNER, Error::NotSupported);
        };
        if og == gid || self.owner_chain_contains(og, gid) {
            return self.error(c, op::WINDOW_OWNER, Error::InvalidArgs);
        }
        if let Some(w) = self.windows.get_mut(&gid) {
            w.owner = Some(og);
        }
        // Uz zobrazene okno jde hned nad vlastnika.
        if self.z.contains(&og) {
            self.raise_owned(og);
        }
    }

    /// Vede retezec vlastniku od `from` pres `target` (cyklus)?
    fn owner_chain_contains(&self, from: u32, target: u32) -> bool {
        let mut g = Some(from);
        for _ in 0..MAX_DEPTH {
            match g {
                Some(x) if x == target => return true,
                Some(x) => g = self.windows.get(&x).and_then(|w| w.owner),
                None => return false,
            }
        }
        true
    }

    /// Okna vlastnena oknem `g` (i neprimo) presune v poradi nad nej,
    /// v jejich dosavadnim poradi.
    pub(crate) fn raise_owned(&mut self, g: u32) {
        let mut owned: alloc::vec::Vec<u32> = alloc::vec::Vec::new();
        let mut frontier = alloc::vec![g];
        for _ in 0..MAX_DEPTH {
            let next: alloc::vec::Vec<u32> = self
                .z
                .iter()
                .copied()
                .filter(|x| !owned.contains(x) && self.windows.get(x).and_then(|w| w.owner).is_some_and(|o| frontier.contains(&o)))
                .collect();
            if next.is_empty() {
                break;
            }
            owned.extend(&next);
            frontier = next;
        }
        if owned.is_empty() {
            return;
        }
        // Dosavadni poradi vlastnenych zustane (z je odspodu nahoru).
        let order: alloc::vec::Vec<u32> = self.z.iter().copied().filter(|x| owned.contains(x)).collect();
        self.z.retain(|x| !owned.contains(x));
        self.z.extend(order);
        for o in owned {
            self.damage_window(o);
        }
    }

    /// Vystredi okno `gid` nad vlastnikem `owner` v pracovni plose.
    pub(crate) fn center_over_owner(&mut self, gid: u32, owner: u32) {
        let Some(o) = self.windows.get(&owner).map(|w| w.rect) else { return };
        let work = self.work_area();
        let Some(w) = self.windows.get_mut(&gid) else { return };
        let old = w.visual();
        let border = w.geo.border as i32;
        let top = if w.mode == guiproto::window::Mode::Standard { (w.geo.title_h + w.geo.border) as i32 } else { border };
        let x = o.x + (o.w as i32 - w.rect.w as i32) / 2;
        let y = o.y + (o.h as i32 - w.rect.h as i32) / 2;
        let max_x = (work.right() as i32 - w.rect.w as i32 - border).max(border);
        let max_y = (work.bottom() as i32 - w.rect.h as i32 - border).max(top);
        w.rect.x = x.clamp(border, max_x);
        w.rect.y = y.clamp(top, max_y);
        let new = w.visual();
        self.damage(old);
        self.damage(new);
    }

    /// Stisk na `x`, `y`: mimo popovery okna s fokusem = `POPOVER_DISMISS`.
    pub(crate) fn popover_press(&mut self, x: i32, y: i32) {
        let Some(g) = self.focus else { return };
        let rects = self.popover_rects(g);
        if rects.is_empty() || rects.iter().any(|r| r.contains(x, y)) {
            return;
        }
        let Some(w) = self.windows.get(&g) else { return };
        let (c, local) = (w.client, w.local);
        if self.clients.get(&c).is_some_and(|cl| cl.version >= 7) {
            self.send(c, ServerMsg::PopoverDismiss { window: local });
        }
    }
}
