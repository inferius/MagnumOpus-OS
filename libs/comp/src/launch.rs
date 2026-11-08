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
        self.trace(time, format_args!("spustit aplikaci {id} (zadost initu)"));
    }
}
