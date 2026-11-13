//! Relace okna (karty v titulku okna GUI) a akce bloku: seznam relaci
//! sluzby
//! (`ServerMsg::SessionList`), prepnuti na jinou relaci a nova relace.
//!
//! Prepnuti = odpojit aktualni relaci (bezi dal) a pripojit jinou; ta
//! prehraje sve bloky. Aby se okno nezaseklo ani u dlouhe historie,
//! relace posle nejdriv posledni bloky (viditelna cast), pak `Attached`
//! a starsi bloky po davkach od nejnovejsich; okno je vklada pred
//! existujici bloky podle cisla bloku ([`App::place_block`]) a GUI
//! zpracuje nejvys davku zprav na snimek.

use alloc::string::String;
use alloc::vec::Vec;

use super::{App, Focus};
use crate::proto::ClientMsg;

/// Cislo bloku uvodni napovedy a vyssi: nejsou bloky relace.
pub(super) const LOCAL_BLOCKS: u64 = u64::MAX - 1_000_000;

/// Relace sluzby: cislo, pocet bloku, slozka.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub id: u64,
    pub blocks: u32,
    pub cwd: String,
}

impl App {
    /// Novy seznam relaci od sluzby (serazeny podle cisla).
    pub(super) fn set_sessions(&mut self, list: Vec<(u64, u32, String)>) {
        let mut v: Vec<SessionInfo> = list.into_iter().map(|(id, blocks, cwd)| SessionInfo { id, blocks, cwd }).collect();
        v.sort_by_key(|s| s.id);
        self.sessions = v;
    }

    /// Relace pro karty: seznam od sluzby, aktualni relace vzdy v nem.
    pub fn session_tabs(&self) -> Vec<SessionInfo> {
        let mut v = self.sessions.clone();
        if self.session != 0 && !v.iter().any(|s| s.id == self.session) {
            v.push(SessionInfo { id: self.session, blocks: self.blocks.len() as u32, cwd: self.cwd.clone() });
            v.sort_by_key(|s| s.id);
        }
        v
    }

    /// Prepnout okno na relaci `id` (`0` = nova relace). Aktualni relace
    /// bezi dal odpojena; okno zapomene jeji bloky a vyzvy.
    pub fn switch_session(&mut self, id: u64) {
        if id != 0 && id == self.session {
            return;
        }
        if self.session != 0 {
            self.out.push(ClientMsg::Detach { session: self.session });
        }
        self.blocks.clear();
        self.prompt = None;
        self.menu = None;
        self.queued.clear();
        self.fg = None;
        self.expect_start = false;
        self.scroll = 0;
        self.focus = Focus::Editor;
        // Uvodni napovedu dostane i nova relace.
        self.welcomed = false;
        self.session = id;
        self.out.push(ClientMsg::Attach { session: id });
        self.out.push(ClientMsg::Sessions);
    }

    /// Kam patri blok `id`: starsi blok prehrany po `Attached` jde pred
    /// novejsi bloky relace (podle cisla), jinak na konec. `None` = blok
    /// se uz nevejde (starsi nez vsechno, co okno drzi, a okno je plne).
    pub(super) fn place_block(&self, id: u64, keep: usize) -> Option<usize> {
        let at = self.blocks.iter().position(|b| b.id > id && b.id < LOCAL_BLOCKS).unwrap_or(self.blocks.len());
        if at == 0 && !self.blocks.is_empty() && self.blocks.len() >= keep {
            return None;
        }
        Some(at)
    }

    /// Akce bloku z okna GUI: sbalit / rozbalit blok `id`.
    pub fn toggle_block(&mut self, id: u64) {
        if let Some(b) = self.blocks.iter_mut().find(|b| b.id == id) {
            b.collapsed = !b.collapsed;
        }
    }

    /// Akce bloku z okna GUI: spustit prikaz bloku `id` znovu (jako by ho
    /// uzivatel napsal; bezi-li prikaz, zaradi se do fronty).
    pub fn rerun_block(&mut self, id: u64) {
        let Some(line) = self.blocks.iter().find(|b| b.id == id).map(|b| b.line.clone()) else { return };
        if !line.trim().is_empty() {
            self.run(line);
        }
    }
}
