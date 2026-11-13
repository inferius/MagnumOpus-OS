//! Prostrednik termd: smerovani zprav mezi oknem a procesy relaci.
//!
//! termd nema interpret - jen predava zpravy. Relace (`shsess`) bezi kazda
//! ve vlastnim procesu, takze pad relace neshodi ostatni ani termd. Cisla
//! bloku a vyzev relace `N` lezi v rozsahu `N << 32` ([`id_base`]), takze
//! odpoved na vyzvu (`Answer`) jde smerovat bez tabulky.
//!
//! Logika je bez systemovych volani (testy na hostiteli); kanaly a
//! spousteni procesu resi `user/termd`.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::proto::{ClientMsg, ServerMsg, VERSION};

/// Prvni cislo bloku/vyzvy relace `session`.
pub fn id_base(session: u64) -> u64 {
    (session << 32) | 1
}

/// Relace podle cisla bloku nebo vyzvy.
pub fn owner(id: u64) -> u64 {
    id >> 32
}

/// Co zna termd o relaci (pro `Sessions` bez dotazu na relaci).
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub id: u64,
    /// Znacka relace (jmena klonu bokem; uklid po padu).
    pub tag: String,
    pub cwd: String,
    pub blocks: u32,
    pub attached: bool,
}

/// Co ma termd udelat.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    Window(ServerMsg),
    Session(u64, ClientMsg),
    /// Spustit proces nove relace (pak `Session(id, Attach)`).
    Spawn { session: u64, tag: String },
}

#[derive(Debug)]
pub struct Broker {
    pub routes: Vec<Route>,
    next_id: u64,
    /// Znacka startu termd (cast znacky relaci).
    nonce: u64,
    /// Kolik bloku si relace pamatuje (stejne jako `Service::keep_blocks`).
    pub keep_blocks: u32,
}

impl Broker {
    pub fn new(nonce: u64) -> Self {
        Self { routes: Vec::new(), next_id: 1, nonce, keep_blocks: 200 }
    }

    fn route(&mut self, id: u64) -> Option<&mut Route> {
        self.routes.iter_mut().find(|r| r.id == id)
    }

    /// Relace prevzata po predchozim termd (z registru).
    pub fn adopt(&mut self, id: u64, tag: String) {
        self.next_id = self.next_id.max(id + 1);
        self.routes.retain(|r| r.id != id);
        self.routes.push(Route { id, tag, cwd: String::new(), blocks: 0, attached: false });
    }

    fn unknown(id: u64) -> Vec<Out> {
        alloc::vec![Out::Window(ServerMsg::Fail { message: format!("relace {id} neexistuje") })]
    }

    fn route_to(&mut self, id: u64, m: ClientMsg) -> Vec<Out> {
        if self.route(id).is_none() {
            return Self::unknown(id);
        }
        alloc::vec![Out::Session(id, m)]
    }

    /// Zprava od okna.
    pub fn from_window(&mut self, m: ClientMsg) -> Vec<Out> {
        match m {
            ClientMsg::Hello { .. } => alloc::vec![Out::Window(ServerMsg::Welcome { version: VERSION })],
            ClientMsg::Attach { session: 0 } => {
                let id = self.next_id;
                self.next_id += 1;
                let tag = format!("{:x}-{id}", self.nonce);
                self.routes.push(Route { id, tag: tag.clone(), cwd: String::new(), blocks: 0, attached: true });
                alloc::vec![Out::Spawn { session: id, tag }, Out::Session(id, ClientMsg::Attach { session: id })]
            }
            ClientMsg::Attach { session } => {
                let Some(r) = self.route(session) else { return Self::unknown(session) };
                r.attached = true;
                r.blocks = 0;
                alloc::vec![Out::Session(session, m)]
            }
            ClientMsg::Detach { session } => {
                if let Some(r) = self.route(session) {
                    r.attached = false;
                }
                self.route_to(session, m)
            }
            ClientMsg::Run { session, .. }
            | ClientMsg::Interrupt { session }
            | ClientMsg::Background { session }
            | ClientMsg::Complete { session, .. }
            | ClientMsg::Format { session, .. }
            | ClientMsg::Highlight { session, .. }
            | ClientMsg::TaskAction { session, .. } => self.route_to(session, m),
            ClientMsg::Answer { prompt, .. } | ClientMsg::Choose { prompt, .. } | ClientMsg::Text { prompt, .. } => {
                self.route_to(owner(prompt), m)
            }
            ClientMsg::Sessions => {
                let sessions = self.routes.iter().map(|r| (r.id, r.blocks, r.cwd.clone())).collect();
                alloc::vec![Out::Window(ServerMsg::SessionList { sessions })]
            }
        }
    }

    /// Zprava od relace `id`: zapsat si stav a predat oknu.
    pub fn from_session(&mut self, id: u64, m: ServerMsg) -> Option<ServerMsg> {
        let keep = self.keep_blocks;
        let r = self.route(id)?;
        match &m {
            ServerMsg::Attached { cwd, blocks, .. } => {
                r.cwd = cwd.clone();
                r.blocks = *blocks;
            }
            ServerMsg::BlockStart { .. } => r.blocks = (r.blocks + 1).min(keep),
            _ => {}
        }
        Some(m)
    }

    /// Proces relace skoncil (nebo spadl).
    pub fn session_ended(&mut self, id: u64) -> Option<ServerMsg> {
        let i = self.routes.iter().position(|r| r.id == id)?;
        let r = self.routes.remove(i);
        r.attached.then(|| ServerMsg::Fail { message: format!("relace {id} skoncila") })
    }

    /// Okno se zavrelo: relace odpojit (bezi dal, bloky si schovaji).
    pub fn window_closed(&mut self) -> Vec<Out> {
        let mut out = Vec::new();
        for r in &mut self.routes {
            if r.attached {
                r.attached = false;
                out.push(Out::Session(r.id, ClientMsg::Detach { session: r.id }));
            }
        }
        out
    }
}
