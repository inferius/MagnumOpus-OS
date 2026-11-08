//! Jadro kompozitoru `compd` bez zavislosti na jadre OS.
//!
//! Sluzba (`user/compd`) prevadi kanaly a objekty pameti na volani
//! [`Comp`] a rozesila zpravy z [`Comp::take_out`]. Diky tomu jde cela
//! logika (transakce, skupiny, takt, skladani, vstup, dialogy, akce)
//! testovat na hostiteli (`cargo test -p comp`).
//!
//! - [`scene`]: klienti, okna, buffery, vrstvy, transakce.
//! - [`render`]: skladani do snimku (dekorace, odznak, panel, kurzor).
//! - [`input`]: rozdeleni vstupu oknum, presun a zmena velikosti.
//! - [`dialog`]: systemove dialogy a bezpecna obrazovka.
//! - [`services`]: akce aplikaci a inspekce.
//! - [`frame`]: takt snimku, animace, skupiny transakci, ping.
//! - [`gpu`]: seznam operaci pro GPU backend (`compd` + `libs/vkcomp`).

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod dialog;
pub mod frame;
pub mod gpu;
pub mod input;
pub mod render;
pub mod scene;
pub mod services;

#[cfg(test)]
mod tests;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use gfx::damage::Damage;
use gfx::text::Text;
use guiproto::control::{CallerReply, DialogReply, InspectReply, Status};
use guiproto::window::ServerMsg;
use guiproto::Rect;

pub type ClientId = u32;
/// Id vlastnika kanalu `dialog`, `actions` nebo `inspect` (prideluje sluzba).
pub type PeerId = u32;

/// Perioda snimku (60 Hz) v ns.
pub const PERIOD: u64 = 16_666_667;
/// Rezerva pred koncem periody pro skladani (termin pro aplikace).
pub const COMPOSE_BUDGET: u64 = 4_000_000;
/// Jak dlouho cekat na potvrzeni `CONFIGURE` nebo na skupinu transakci.
pub const SYNC_TIMEOUT: u64 = 200_000_000;

/// Pamet bufferu aplikace (sluzba ji ma namapovanou jen pro cteni).
pub trait Pixels {
    /// Pixely bufferu (`stride * h` u32); `None` = pamet neni dostupna.
    fn pixels(&self) -> Option<&[u32]>;
}

/// Buffer v pameti procesu (testy, vnitrni buffery).
impl Pixels for Vec<u32> {
    fn pixels(&self) -> Option<&[u32]> {
        Some(self)
    }
}

/// Zprava ven, kterou sluzba posle prislusnemu kanalu.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    Client(ClientId, ServerMsg),
    Dialog(PeerId, DialogReply),
    Caller(PeerId, CallerReply),
    Inspect(PeerId, InspectReply),
    /// Konec bezpecne obrazovky: poslat `release` na kanal `secure`.
    SecureRelease,
    /// Rozhodnuti o novem vstupnim zarizeni: `trust <id>` / `reject <id>`
    /// na kanal `secure`.
    Device { source: u16, trust: bool },
}

/// Kompozitor.
pub struct Comp {
    pub w: u32,
    pub h: u32,
    pub(crate) text: Text,
    pub(crate) clients: BTreeMap<ClientId, scene::Client>,
    pub(crate) windows: BTreeMap<u32, scene::Window>,
    /// Poradi oken zdola nahoru (globalni id).
    pub(crate) z: Vec<u32>,
    pub(crate) focus: Option<u32>,
    pub(crate) next_client: ClientId,
    pub(crate) next_window: u32,
    pub(crate) dirty: Damage,
    pub(crate) out: Vec<Out>,
    pub(crate) frame: frame::FrameState,
    pub(crate) input: input::InputState,
    pub(crate) dialogs: dialog::Dialogs,
    pub(crate) services: services::Services,
    /// Zdroj tokenu skupin (sluzba dava nahodna cisla z jadra).
    pub(crate) token_source: Box<dyn FnMut() -> u128>,
    /// Posledni znamy cas (z volani se `now`).
    pub(crate) input_now: u64,
    /// Bezici ulohy a z toho ulohy agenta AI (ikona v panelu).
    pub(crate) tasks: (u32, u32),
}

impl Comp {
    pub fn new(w: u32, h: u32, text: Text, token_source: Box<dyn FnMut() -> u128>) -> Self {
        let mut c = Self {
            w,
            h,
            text,
            clients: BTreeMap::new(),
            windows: BTreeMap::new(),
            z: Vec::new(),
            focus: None,
            next_client: 1,
            next_window: 1,
            dirty: Damage::new(),
            out: Vec::new(),
            frame: frame::FrameState::default(),
            input: input::InputState::new(w, h),
            dialogs: dialog::Dialogs::default(),
            services: services::Services::default(),
            token_source,
            input_now: 0,
            tasks: (0, 0),
        };
        c.damage_all();
        c
    }

    pub fn screen(&self) -> Rect {
        Rect::new(0, 0, self.w, self.h)
    }

    pub(crate) fn damage(&mut self, r: Rect) {
        let s = self.screen();
        self.dirty.add(r, s);
    }

    pub(crate) fn damage_all(&mut self) {
        let s = self.screen();
        self.dirty.add(s, s);
    }

    pub(crate) fn send(&mut self, c: ClientId, m: ServerMsg) {
        self.out.push(Out::Client(c, m));
    }

    /// Zpravy k odeslani (vyprazdni frontu).
    pub fn take_out(&mut self) -> Vec<Out> {
        core::mem::take(&mut self.out)
    }

    /// Prekreslit celou obrazovku (napr. po zmene backendu skladani).
    pub fn redraw_all(&mut self) {
        self.damage_all();
    }

    /// Pocet bezicich uloh pro ikonu v panelu (od sluzby uloh).
    pub fn set_tasks(&mut self, running: u32, agent: u32) {
        if self.tasks != (running, agent) {
            self.tasks = (running, agent);
            self.damage_panel();
        }
    }

    pub fn status(&self) -> Status {
        Status {
            clients: self.clients.len() as u32,
            windows: self.windows.len() as u32,
            frames: self.frame.frames,
            presented: self.frame.presented,
            dialogs: self.dialogs.len() as u32,
            secure: self.dialogs.secure_active(),
        }
    }
}
