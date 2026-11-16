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
use ui::{App, Cells, Node, button, checkbox, col, column, label, row, segmented, spacer, table};

pub use test_backend::TestBackend;

/// Spojeni s taskd pres kanal: pozadavky odesle zapisovac `send`, odpovedi
/// prijdou zpravami kanalu do [`App::external`].
pub struct ChannelBackend {
    send: alloc::boxed::Box<dyn FnMut(Vec<u8>)>,
}

impl ChannelBackend {
    pub fn new(send: impl FnMut(Vec<u8>) + 'static) -> Self {
        Self { send: alloc::boxed::Box::new(send) }
    }
}

impl Backend for ChannelBackend {
    fn send(&mut self, req: Request) -> Vec<Reply> {
        (self.send)(req.encode());
        Vec::new()
    }
}

/// Spojeni se sluzbou uloh: odesle pozadavek; odpovedi, ktere jsou hned
/// k dispozici (testovaci zdroj), vrati, ostatni prijdou pres
/// [`TaskManager::reply`].
pub trait Backend {
    fn send(&mut self, req: Request) -> Vec<Reply>;
}

/// Filtr seznamu (segmenty Vse / Moje / Agent / System, design/plocha.html).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    /// Spustil uzivatel: primo, skriptem nebo z aplikace.
    Mine,
    /// Ulohy agenta AI.
    Agent,
    /// Sluzby a neznamy proces.
    System,
}

impl Filter {
    pub const ALL: [Filter; 4] = [Filter::All, Filter::Mine, Filter::Agent, Filter::System];

    fn label(self) -> &'static str {
        match self {
            Filter::All => "Vše",
            Filter::Mine => "Moje",
            Filter::Agent => "Agent",
            Filter::System => "Systém",
        }
    }

    pub fn matches(self, t: &Task) -> bool {
        use taskproto::Origin::*;
        match self {
            Filter::All => true,
            Filter::Mine => !t.agent && matches!(t.origin, User | Script | App),
            Filter::Agent => t.agent,
            Filter::System => !t.agent && matches!(t.origin, Service | Unknown),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// Vyber radku tabulky (id ulohy).
    Select(Vec<u64>),
    Filter(Filter),
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
    filter: Filter,
    show_done: bool,
    next_req: u32,
    /// Text o posledni akci ("Zruseno", "Odmitnuto: ACCESS_DENIED").
    pub status: String,
    pub log: Vec<String>,
    log_req: u32,
}

/// Lidsky citelny stav.
/// "1 běžící úloha", "2 běžící úlohy", "5 běžících úloh" (české tvary).
pub fn running_tasks(n: u32) -> String {
    match n {
        1 => String::from("1 běžící úloha"),
        2..=4 => format!("{n} běžící úlohy"),
        _ => format!("{n} běžících úloh"),
    }
}

pub fn state_text(t: &Task) -> String {
    let base = state_word(t);
    match (t.progress, t.state.finished()) {
        (Some(p), false) => format!("{base} {p} %"),
        _ => base,
    }
}

/// Stav bez prubehu (sloupec Stav; prubeh ma vlastni sloupec).
fn state_word(t: &Task) -> String {
    match &t.state {
        State::Running => String::from("běží"),
        State::Paused => String::from("pozastaveno"),
        State::Waiting(p) => format!("čeká: {p}"),
        State::Cancelling => String::from("ruší se"),
        State::Done(r) if r.is_empty() => String::from("hotovo"),
        State::Done(r) => format!("hotovo: {r}"),
        State::Failed { message, .. } => format!("chyba: {message}"),
        State::Cancelled => String::from("zrušeno"),
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

/// Ikona puvodu ulohy (sloupec Uloha).
fn origin_icon(t: &Task) -> &'static str {
    use taskproto::Origin::*;
    match t.origin {
        _ if t.agent => "agent",
        User | Script => "terminal",
        App => "window",
        Service => "settings",
        Agent => "agent",
        Unknown => "alert",
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
            filter: Filter::All,
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
            .filter(|t| self.filter.matches(t) && (self.show_done || !t.state.finished()))
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

    /// Bunky radku: Uloha, Kdo spustil, Stav, Prubeh, Pamet.
    fn cells(t: &Task) -> Cells {
        let who = if t.agent { t.who.clone() } else { format!("{} ({})", t.who, origin_text(t.origin)) };
        let running = !t.state.finished();
        let prog = match (t.progress, running) {
            (Some(p), true) => format!("{p} %"),
            _ => String::from("-"),
        };
        let mem = t.memory.map_or(String::from("-"), bytes);
        let mut c = Cells::new([t.name.clone(), who, state_word(t), prog, mem]).icon(origin_icon(t));
        if t.agent {
            c = c.agent(1);
        }
        if let (Some(p), true) = (t.progress, running) {
            c = c.progress(3, p as u32);
        }
        c
    }

    /// Pruh detailu dole (`.tm-detail`): jmeno, kdo a stav, akce nad
    /// vybranou ulohou jako tlacitka s ikonou.
    fn detail(&self) -> Node<Msg> {
        let Some(t) = self.selected() else {
            return row([label("Vyberte úlohu: šipkami nahoru a dolů, Enter otevře výpis.").tone(ui::TextTone::Muted)]);
        };
        let who = if t.agent {
            label(&format!("Spustil agent AI ({})", t.who)).agent()
        } else {
            label(&format!("Spustil: {} ({})", origin_text(t.origin), t.who)).tone(ui::TextTone::Muted)
        };
        let cpu = t.cpu_ns.map_or(String::from("-"), |n| format!("{} ms", n / 1_000_000));
        let mut v = vec![row([
            label(&t.name).tone(ui::TextTone::Strong),
            who,
            spacer(),
            button("").icon("pause").ghost().tooltip("Pozastavit").on_click(Msg::Pause).disabled(!can(t, actions::PAUSE)),
            button("").icon("play").ghost().tooltip("Pokračovat").on_click(Msg::Resume).disabled(!can(t, actions::RESUME)),
            button("").icon("stop").ghost().tooltip("Zrušit").on_click(Msg::Cancel).disabled(!can(t, actions::CANCEL)),
            button("").icon("log").ghost().tooltip("Výpis").on_click(Msg::Log).disabled(!can(t, actions::LOG)),
        ])];
        let mut info = vec![label(&format!("Stav: {}", state_text(t))), label(&format!("Čas CPU: {cpu}")).tone(ui::TextTone::Muted)];
        if t.actions == 0 {
            info.push(label("Tuto úlohu nemůžete řídit (chybí oprávnění).").tone(ui::TextTone::Muted));
        }
        v.push(row(info));
        if !self.status.is_empty() {
            v.push(label(&self.status));
        }
        for l in self.log.iter().rev().take(5).rev() {
            v.push(label(l).tone(ui::TextTone::Small));
        }
        column(v)
    }
}

impl<B: Backend> App for TaskManager<B> {
    type Msg = Msg;

    fn title(&self) -> String {
        let (run, _) = self.list.running();
        format!("Správce úloh ({run} běží)")
    }

    fn view(&self, cx: &ui::Context) -> Node<Msg> {
        // design/plocha.html "Spravce uloh": filtr nahore, tabulka uloh
        // (uloha agenta oznacena), pruh detailu s akcemi dole.
        // Sirky sloupcu v nasobcich nejmensiho cile (roste s dotykem
        // a velikosti textu, jako ostatni prvky).
        let w = cx.min_hit.max(1);
        let (_, agent) = self.list.running();
        let vis: Vec<Task> = self.visible().into_iter().cloned().collect();
        let n = vis.len();
        let ids: Vec<u64> = vis.iter().map(|t| t.id).collect();
        let sel: Vec<u64> = self.selected.filter(|id| ids.contains(id)).into_iter().collect();
        let fi = Filter::ALL.iter().position(|f| *f == self.filter).unwrap_or(0);
        let mut tools = vec![
            segmented(Filter::ALL.iter().map(|f| String::from(f.label())), fi).on_pick(|i| Msg::Filter(Filter::ALL[i])).key("filtr"),
            checkbox("Zobrazit dokončené", self.show_done).on_toggle(Msg::ShowDone),
            spacer(),
        ];
        if agent > 0 {
            tools.push(label(&format!("Agent AI: {}", running_tasks(agent as u32))).agent());
        }
        let data = alloc::rc::Rc::new(vis);
        let d2 = alloc::rc::Rc::clone(&data);
        let tasks = table(
            vec![col("Úloha", 0), col("Kdo spustil", 5 * w), col("Stav", 3 * w), col("Průběh", 2 * w + w / 2), col("Paměť", 2 * w).numeric()],
            n,
            move |i| data.get(i).map_or(0, |t| t.id),
            move |i| d2.get(i).map_or_else(Cells::default, Self::cells),
        )
        .selection(sel)
        .on_selection(Msg::Select)
        .on_activate(|_| Msg::Log)
        .key("ulohy");
        let detail = column([self.detail()]).pad(w / 4, 0, 0, 0).deco(ui::Deco { fill: None, radius: 0, top_border: true });
        column([row(tools), tasks, detail])
    }

    fn external(&mut self, data: &[u8]) -> bool {
        match Reply::decode(data) {
            Ok(r) => {
                self.reply(&r);
                true
            }
            Err(_) => false,
        }
    }

    fn update(&mut self, m: Msg) {
        match m {
            Msg::Select(ids) => {
                let id = ids.first().copied();
                if id != self.selected {
                    self.log.clear();
                    self.status.clear();
                }
                self.selected = id;
            }
            Msg::Filter(f) => self.filter = f,
            Msg::ShowDone(b) => self.show_done = b,
            Msg::Cancel => self.act(actions::CANCEL),
            Msg::Pause => self.act(actions::PAUSE),
            Msg::Resume => self.act(actions::RESUME),
            Msg::Log => self.act(actions::LOG),
        }
    }
}
