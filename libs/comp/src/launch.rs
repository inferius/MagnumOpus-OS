//! Spousteni aplikaci z listy: init posle seznam spustitelnych aplikaci
//! (`apps`, id a jmeno z manifestu). Aplikace bez okna ma v liste
//! ztlumenou ikonu bez znacky; klik na ni (nebo na radek ve spousteci)
//! ji necha spustit initem (kompozitor nespousti nic sam: zadost
//! `launch <id>` na kanalu `launcher`, init ji spusti pod supervisorem
//! se stejnymi handly jako pri startu plochy). Bezici aplikaci klik
//! aktivuje.

use alloc::string::String;
use alloc::vec::Vec;

use crate::Comp;

/// Jak dlouho po zadosti uzivatele o spusteni dostane nove okno fokus.
pub const EXPECT_FOCUS: u64 = 30_000_000_000;

impl Comp {
    /// Seznam spustitelnych aplikaci: (id, jmeno), v poradi listy.
    pub fn set_launchable(&mut self, apps: Vec<(String, String)>) {
        self.launchable = apps;
        self.damage_panel();
    }

    pub fn launchable(&self) -> &[(String, String)] {
        &self.launchable
    }

    /// Zadosti o spusteni od posledniho volani (id aplikaci).
    pub fn take_launch_requests(&mut self) -> Vec<String> {
        core::mem::take(&mut self.launch_requests)
    }

    /// Okno bezici aplikace `id` (zobrazene).
    pub(crate) fn app_window(&self, id: &str) -> Option<u32> {
        self.z
            .iter()
            .rev()
            .copied()
            .find(|g| self.windows.get(g).is_some_and(|w| w.mapped && self.clients.get(&w.client).is_some_and(|c| c.info.id == id)))
    }

    /// Zobrazena okna aplikace `id`, shora dolu.
    pub(crate) fn app_windows(&self, id: &str) -> Vec<u32> {
        self.z
            .iter()
            .rev()
            .copied()
            .filter(|g| self.windows.get(g).is_some_and(|w| w.mapped && self.clients.get(&w.client).is_some_and(|c| c.info.id == id)))
            .collect()
    }

    /// Id aplikace okna `g`.
    pub(crate) fn app_of(&self, g: u32) -> Option<String> {
        self.windows.get(&g).and_then(|w| self.clients.get(&w.client)).map(|c| c.info.id.clone())
    }

    /// Aplikace s vice okny (manifest `[window] instances = "multi"`).
    pub fn set_multi(&mut self, ids: Vec<String>) {
        self.multi = ids;
    }

    /// Smi mit aplikace dalsi okno (`multi` a init ji umi spustit)?
    /// `single` (vychozi): spusteni = jen fokus existujiciho okna.
    pub(crate) fn is_launchable(&self, id: &str) -> bool {
        self.launchable.iter().any(|(a, _)| a == id) && self.multi.iter().any(|m| m == id)
    }

    /// Dalsi okno (instance) aplikace: zadost initu; nove okno dostane fokus
    /// (spustil ho uzivatel).
    pub(crate) fn launch_new(&mut self, id: &str, time: u64) {
        if !self.is_launchable(id) {
            return;
        }
        if !self.launch_requests.iter().any(|r| r == id) {
            self.launch_requests.push(String::from(id));
        }
        self.expect_focus.push((String::from(id), self.input_now));
        self.trace(time, format_args!("nove okno aplikace {id} (zadost initu)"));
    }

    /// Nove zobrazene okno: fokus, kdyz ho spustil uzivatel (lista, paleta,
    /// zkratka) nebo kdyz zadne jine okno fokus nema. Okno otevrene samo na
    /// pozadi fokus neukradne (psani jinam), jen se zvyrazni ikona v liste.
    pub(crate) fn new_window_focus(&mut self, gid: u32, now: u64) {
        let Some(id) = self.app_of(gid) else { return };
        self.expect_focus.retain(|(_, t)| now.saturating_sub(*t) <= EXPECT_FOCUS);
        let user = match self.expect_focus.iter().position(|(a, _)| *a == id) {
            Some(i) => {
                self.expect_focus.remove(i);
                true
            }
            None => false,
        };
        let client = self.windows.get(&gid).map(|w| w.client);
        let other = self.focus.and_then(|f| self.windows.get(&f)).is_some_and(|w| Some(w.client) != client);
        if user || !other {
            self.set_focus(Some(gid));
        } else if let Some(w) = self.windows.get_mut(&gid) {
            w.attention = true;
        }
        self.damage_panel();
    }

    /// Klik na aplikaci: bezici do popredi (i z minimalizace), jinak
    /// zadost o spusteni.
    pub(crate) fn launch_or_activate(&mut self, id: &str, time: u64) {
        if let Some(g) = self.app_window(id) {
            if let Some(w) = self.windows.get_mut(&g) {
                w.minimized = false;
            }
            self.damage_window(g);
            self.set_focus(Some(g));
            self.damage_panel();
            return;
        }
        if !self.launch_requests.iter().any(|r| r == id) {
            self.launch_requests.push(String::from(id));
        }
        self.expect_focus.push((String::from(id), self.input_now));
        self.trace(time, format_args!("spustit aplikaci {id} (zadost initu)"));
    }
}
