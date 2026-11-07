//! Spravce uloh v GUI: seznam uloh ze sluzby `taskd` (`libs/taskproto`),
//! detail vybrane ulohy a akce zrusit, pozastavit, pokracovat a vypis.
//!
//! - Ulohy agenta AI jsou vyrazne oznacene (stitek v seznamu i v detailu,
//!   pocet v zahlavi).
//! - Akce respektuji opravneni: tlacitko je aktivni jen tehdy, kdyz ho
//!   `taskd` volajicimu povolil (bity `actions` ulohy) a stav ulohy ho
//!   dovoluje. Rozhoduje vzdy `taskd` (odpoved `Result`), okno jen
//!   nenabizi, co nejde.
//!
//! Spojeni s taskd je za traitem [`Backend`]; [`TestBackend`] je
//! testovaci zdroj dat, dokud taskd neni v systemu.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod test_backend;

#[cfg(test)]
mod tests;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use taskproto::{Applied, Reply, Request, State, Task, TaskList, actions};
use ui::{App, Node, button, checkbox, column, label, list, progress, row, scroll, spacer};

pub use test_backend::TestBackend;

/// Spojeni se sluzbou uloh: odesle pozadavek; odpovedi, ktere jsou hned
/// k dispozici (testovaci zdroj), vrati, ostatni prijdou pres
/// [`TaskManager::reply`].
pub trait Backend {
    fn send(&mut self, req: Request) -> Vec<Reply>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    Select(usize),
    OnlyAgent(bool),
    ShowDone(bool),
    Cancel,
    Pause,
    Resume,
    Log,
}

pub struct TaskManager<B: Backend> {
    pub list: TaskList,
    pub backend: B,
    selected: Option<u64>,
    only_agent: bool,
    show_done: bool,
    next_req: u32,
    /// Text o posledni akci ("Zruseno", "Odmitnuto: ACCESS_DENIED").
    pub status: String,
    pub log: Vec<String>,
    log_req: u32,
}

/// Lidsky citelny stav.
pub fn state_text(t: &Task) -> String {
    let base = match &t.state {
        State::Running => String::from("běží"),
        State::Paused => String::from("pozastaveno"),
        State::Waiting(p) => format!("čeká: {p}"),
        State::Cancelling => String::from("ruší se"),
        State::Done(r) if r.is_empty() => String::from("hotovo"),
        State::Done(r) => format!("hotovo: {r}"),
        State::Failed { message, .. } => format!("chyba: {message}"),
        State::Cancelled => String::from("zrušeno"),
    };
    match (t.progress, t.state.finished()) {
        (Some(p), false) => format!("{base} {p} %"),
        _ => base,
    }
}

fn origin_text(o: taskproto::Origin) -> &'static str {
    use taskproto::Origin::*;
    match o {
        User => "uživatel",
        Script => "skript",
        App => "aplikace",
        Service => "služba",
        Agent => "agent AI",
        Unknown => "neznámý proces",
    }
}

fn bytes(v: u64) -> String {
    if v >= 1 << 20 { format!("{} MiB", v >> 20) } else { format!("{} KiB", v >> 10) }
}

/// Smi volajici akci (bit od taskd) a dovoluje ji stav?
pub fn can(t: &Task, action: u32) -> bool {
    if t.actions & action == 0 {
        return false;
    }
    match action {
        actions::CANCEL => !t.state.finished() && t.state != State::Cancelling,
        actions::PAUSE => matches!(t.state, State::Running | State::Waiting(_)),
        actions::RESUME => t.state == State::Paused,
        _ => true,
    }
}

impl<B: Backend> TaskManager<B> {
    pub fn new(mut backend: B) -> Self {
        let first = backend.send(Request::Subscribe { req: 1, since: None });
        let mut m = Self {
            list: TaskList::default(),
            backend,
            selected: None,
            only_agent: false,
            show_done: true,
            next_req: 2,
            status: String::new(),
            log: Vec::new(),
            log_req: 0,
        };
        for r in first {
            m.reply(&r);
        }
        m
    }

    /// Zprava od taskd.
    pub fn reply(&mut self, r: &Reply) {
        match r {
            Reply::Result { result, .. } => {
                self.status = match result {
                    Ok(()) => String::from("Provedeno."),
                    Err(e) => format!("Odmítnuto: {}", e.name()),
                };
            }
            Reply::LogLines { req, lines } if *req == self.log_req => {
                self.log = lines.clone();
            }
            other => {
                if self.list.apply(other) == Applied::Gap {
                    // Vypadek udalosti: navazat od posledniho cisla.
                    let req = self.req();
                    let since = Some(self.list.seq);
                    for r in self.backend.send(Request::Subscribe { req, since }) {
                        self.reply(&r);
                    }
                }
            }
        }
    }

    fn req(&mut self) -> u32 {
        let r = self.next_req;
        self.next_req = self.next_req.wrapping_add(1).max(2);
        r
    }

    /// Ulohy v seznamu podle filtru (bezici nahore, pak dokoncene).
    pub fn visible(&self) -> Vec<&Task> {
        let mut v: Vec<&Task> = self
            .list
            .tasks
            .iter()
            .filter(|t| (!self.only_agent || t.agent) && (self.show_done || !t.state.finished()))
            .collect();
        v.sort_by_key(|t| (t.state.finished(), !t.agent, t.id));
        v
    }

    pub fn selected(&self) -> Option<&Task> {
        self.selected.and_then(|id| self.list.tasks.iter().find(|t| t.id == id))
    }

    fn act(&mut self, action: u32) {
        let Some(t) = self.selected() else { return };
        if !can(t, action) {
            return;
        }
        let id = t.id;
        let req = self.req();
        let r = match action {
            actions::CANCEL => Request::Cancel { req, id },
            actions::PAUSE => Request::Pause { req, id },
            actions::RESUME => Request::Resume { req, id },
            _ => {
                self.log_req = req;
                Request::Log { req, id, lines: 20 }
            }
        };
        self.status = String::from("Čekám na službu úloh...");
        for r in self.backend.send(r) {
            self.reply(&r);
        }
    }

    fn row_text(t: &Task) -> String {
        let tag = if t.agent { "[AGENT AI] " } else { "" };
        format!("{tag}{} - {}", t.name, state_text(t))
    }
}

impl<B: Backend> App for TaskManager<B> {
    type Msg = Msg;

    fn title(&self) -> String {
        let (run, _) = self.list.running();
        format!("Správce úloh ({run} běží)")
    }

    fn view(&self) -> Node<Msg> {
        let (run, agent) = self.list.running();
        let vis = self.visible();
        let sel = self.selected.and_then(|id| vis.iter().position(|t| t.id == id));
        let rows: Vec<String> = vis.iter().map(|t| Self::row_text(t)).collect();
        let mut head = vec![label(&format!("Úlohy: běží {run}")).heading(), spacer()];
        if agent > 0 {
            head.push(label(&format!("Agent AI: {agent} běžících úloh")).warn());
        }
        let mut items = vec![
            row(head),
            row([
                checkbox("Jen úlohy agenta", self.only_agent).on_toggle(Msg::OnlyAgent),
                checkbox("Zobrazit dokončené", self.show_done).on_toggle(Msg::ShowDone),
            ]),
            scroll(list(rows, sel).on_select(Msg::Select)).height(190),
        ];
        match self.selected() {
            None => items.push(label("Vyberte úlohu v seznamu.")),
            Some(t) => {
                items.push(label(&t.name).heading());
                if t.agent {
                    items.push(label(&format!("Spustil agent AI ({})", t.who)).warn());
                } else {
                    items.push(label(&format!("Spustil: {} ({})", origin_text(t.origin), t.who)));
                }
                items.push(label(&format!("Stav: {}", state_text(t))));
                if let (Some(p), false) = (t.progress, t.state.finished()) {
                    items.push(progress(p as u32));
                }
                let mem = t.memory.map_or(String::from("-"), bytes);
                let cpu = t.cpu_ns.map_or(String::from("-"), |n| format!("{} ms", n / 1_000_000));
                items.push(label(&format!("Paměť: {mem}, čas CPU: {cpu}")));
                items.push(row([
                    button("Zrušit").on_click(Msg::Cancel).disabled(!can(t, actions::CANCEL)),
                    button("Pozastavit").on_click(Msg::Pause).disabled(!can(t, actions::PAUSE)),
                    button("Pokračovat").on_click(Msg::Resume).disabled(!can(t, actions::RESUME)),
                    button("Výpis").on_click(Msg::Log).disabled(!can(t, actions::LOG)),
                ]));
                if t.actions == 0 {
                    items.push(label("Tuto úlohu nemůžete řídit (chybí oprávnění)."));
                }
            }
        }
        if !self.status.is_empty() {
            items.push(label(&self.status));
        }
        for l in self.log.iter().rev().take(5).rev() {
            items.push(label(l));
        }
        column(items)
    }

    fn update(&mut self, m: Msg) {
        match m {
            Msg::Select(i) => {
                let id = self.visible().get(i).map(|t| t.id);
                if id != self.selected {
                    self.log.clear();
                    self.status.clear();
                }
                self.selected = id;
            }
            Msg::OnlyAgent(b) => self.only_agent = b,
            Msg::ShowDone(b) => self.show_done = b,
            Msg::Cancel => self.act(actions::CANCEL),
            Msg::Pause => self.act(actions::PAUSE),
            Msg::Resume => self.act(actions::RESUME),
            Msg::Log => self.act(actions::LOG),
        }
    }
}
