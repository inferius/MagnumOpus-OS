//! Interaktivni terminal (okno) nad nativnim protokolem: editor radku se
//! zvyraznenim a napovedou z historie, bloky vystupu, interaktivni
//! tabulky, nabidka doplnovani, vyzvy a pruhy prubehu v realnem case.
//!
//! Knihovna nema systemova volani: dostava klavesy ([`Key`]), zpravy
//! sluzby a cas, posila [`ClientMsg`] a kresli snimek jako text s ANSI
//! ([`App::frame`]) pro [`crate::screen`]. Pouziva ji textova a zachranna
//! konzole (`user/console`) a stejne ji muze pouzit okno GUI.

pub mod draw;
pub mod editor;
pub mod preview;
pub mod progress;
pub mod sessions;
pub mod table;
pub mod tasks;

use alloc::string::String;
use alloc::vec::Vec;

use shlang::value::Value;

use crate::proto::{ClientMsg, Item, ServerMsg, VERSION};
use crate::render;
use crate::theme::{Style, Theme};
use draw::Line;
use editor::Editor;
use progress::Track;
use table::TableView;

/// Klavesa (nezavisle na zdroji vstupu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
    BackTab,
    Esc,
    /// Posun vystupu o stranku (Shift+PgUp / Shift+PgDn; samotne PgUp je
    /// navigace v tabulce).
    ScrollPageUp,
    ScrollPageDown,
    /// Kolecko mysi: kroky, kladne nahoru.
    Wheel(i32),
    /// Alt+sipka vlevo / vpravo: pri prazdnem radku zpet / vpred
    /// v historii slozek (`cd -`, `cd +`).
    AltLeft,
    AltRight,
    /// Ctrl + pismeno (malym).
    Ctrl(char),
}

/// Blok vystupu v okne.
#[derive(Debug, Clone)]
pub struct UiBlock {
    pub id: u64,
    pub line: String,
    pub items: Vec<Item>,
    pub ok: Option<bool>,
    pub duration: i64,
    pub collapsed: bool,
    /// Interaktivni tabulka (prvni tabulka bloku) a index jeji polozky.
    pub table: Option<TableView>,
    table_item: Option<usize>,
    /// Rychlost a odhad pruhu (index polozky -> stopa).
    tracks: Vec<(usize, Track)>,
    /// Cas zacatku (ns, cas okna) - delka behu pod prikazem.
    started: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editor,
    /// Navigace v tabulce bloku (index do `blocks`).
    Block(usize),
    /// Psani filtru tabulky bloku.
    Filter(usize),
}

#[derive(Debug, Clone)]
pub struct Menu {
    pub start: usize,
    pub items: Vec<(String, u8, String)>,
    pub sel: usize,
}

#[derive(Debug, Clone)]
pub struct PromptUi {
    pub id: u64,
    pub question: String,
    pub detail: Item,
    /// Prazdne = ano/ne.
    pub choices: Vec<String>,
    pub sel: usize,
    /// Dotaz na text: (skryty, napsany text). `None` = volba.
    pub input: Option<(bool, String)>,
}

pub struct App {
    pub theme: Theme,
    pub cols: usize,
    pub rows: usize,
    pub session: u64,
    pub cwd: String,
    pub editor: Editor,
    pub blocks: Vec<UiBlock>,
    pub focus: Focus,
    pub menu: Option<Menu>,
    pub prompt: Option<PromptUi>,
    out: Vec<ClientMsg>,
    /// Aktualni cas (ns od 1970) pro rychlosti, odhady a stari.
    pub now: Option<i64>,
    /// Jak kreslit ukazatel cinnosti a pruh ([`crate::activity`]): okno
    /// GUI vlastnim prvkem, konzole znaky, "Omezit pohyb" staticky.
    pub glyphs: crate::activity::Glyphs,
    /// Okno GUI kresli pred slozku ve vyzve ikonu: radek vstupu pro ni
    /// nechava dve bunky (pilulka podle `design/terminal.html`).
    pub icon_room: bool,
    /// Zvoleny zapis jazyka: `false` = B (prikazovy), `true` = C
    /// (vyrazovy). Ctrl+F a prepinac v titulku okna ho meni a rozepsany
    /// radek prevedou.
    pub notation_c: bool,
    /// Pocitadlo snimku (tocici se znak u bezicich bloku).
    tick: u32,
    /// Radky napsane behem behu prikazu (pobezi po nem, v poradi).
    pub queued: Vec<String>,
    /// Blok prave beziciho radku z okna; `expect_start` = radek odeslan,
    /// jeho blok jeste nezacal.
    fg: Option<u64>,
    expect_start: bool,
    /// Posun vystupu nahoru (radky od konce; 0 = dole, novy vystup a psani
    /// vraci dolu). Nejvetsi mozny posun spocita posledni snimek.
    pub scroll: usize,
    max_scroll: core::cell::Cell<usize>,
    pub connected: bool,
    /// Radky zadane pred pripojenim k relaci (odeslou se po `Attached`).
    pending: Vec<String>,
    /// Ukazat uvodni napovedu v nove relaci ([`WELCOME`]).
    pub welcome: bool,
    /// Uvodni napoveda uz se ukazala.
    welcomed: bool,
    /// Cislo posledniho pozadavku `Complete`/`Format` (`seq`).
    req: u32,
    /// Sedy navrh argumentu z doplnovani: (seq pozadavku, seq textu
    /// editoru, zbytek k doplneni).
    hint_req: u32,
    hint: Option<(u32, String)>,
    /// Ulohy uzivatele z prehledu uloh (`taskd`).
    pub tasks: Vec<taskproto::Task>,
    /// Panel uloh: vybrana uloha (`None` = zavreny).
    task_sel: Option<usize>,
    notice: Option<tasks::Notice>,
    /// Relace sluzby (karty v titulku okna GUI).
    sessions: Vec<sessions::SessionInfo>,
}

/// Uvodni napoveda nove relace (blok bez prikazu).
pub const WELCOME: &[&str] = &[
    "Terminál MagnumOpus. Napište help pro přehled příkazů, help <příkaz> pro podrobnosti.",
    "Příklady:  ls | where size > 1KB   cd ~/Dokumenty   cat poznamky.txt | from text   history",
    "Tab doplní, šipky nahoru/dolů historie, PgUp projde tabulku, Ctrl-C přeruší, undo vrátí změnu.",
];

/// Kolik bloku si okno drzi.
const KEEP_BLOCKS: usize = 100;
/// Nejvic radku tabulky v bloku mimo navigaci.
const TABLE_ROWS: usize = 8;
/// Delka tiku okna (ms) pro ukazatel cinnosti bez znameho casu.
const TICK_MS: i64 = 150;

impl App {
    pub fn new(theme: Theme, cols: usize, rows: usize) -> App {
        App {
            theme,
            cols: cols.max(20),
            rows: rows.max(6),
            session: 0,
            cwd: String::new(),
            editor: Editor::new(),
            blocks: Vec::new(),
            focus: Focus::Editor,
            menu: None,
            prompt: None,
            out: Vec::new(),
            now: None,
            glyphs: crate::activity::Glyphs::Text,
            icon_room: false,
            notation_c: false,
            tick: 0,
            scroll: 0,
            queued: Vec::new(),
            fg: None,
            expect_start: false,
            max_scroll: core::cell::Cell::new(0),
            connected: false,
            pending: Vec::new(),
            welcome: false,
            welcomed: false,
            req: 0,
            hint_req: HINT_SEQ,
            hint: None,
            tasks: Vec::new(),
            task_sel: None,
            notice: None,
            sessions: Vec::new(),
        }
    }

    /// Spusti radek, jako by ho uzivatel napsal (akce okna, ctecka).
    pub fn run_line(&mut self, line: &str) {
        self.run(line.into());
    }

    /// Spojeni se sluzbou spadlo (termd skoncil); po novem pripojeni
    /// [`App::start`] se stejnou relaci.
    pub fn disconnected(&mut self) {
        self.connected = false;
        self.prompt = None;
        self.menu = None;
    }

    /// Prvni zpravy po pripojeni (nova relace, nebo `session`).
    pub fn start(&mut self, session: u64) {
        self.out.push(ClientMsg::Hello { version: VERSION, cols: self.cols as u16, rows: self.rows as u16 });
        self.out.push(sessions::attach(session));
    }

    /// Zpravy k odeslani sluzbe.
    pub fn take_out(&mut self) -> Vec<ClientMsg> {
        core::mem::take(&mut self.out)
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols.max(20);
        self.rows = rows.max(6);
    }

    /// Cas pro animace a odhady (volat pravidelne).
    pub fn tick(&mut self, now: Option<i64>) {
        self.now = now;
        self.tick = self.tick.wrapping_add(1);
    }

    fn block(&mut self, id: u64) -> Option<&mut UiBlock> {
        self.blocks.iter_mut().find(|b| b.id == id)
    }

    /// Bezi prave nejaky prikaz?
    pub fn busy(&self) -> bool {
        self.blocks.iter().any(|b| b.ok.is_none())
    }

    /// Zprava od sluzby.
    pub fn on_msg(&mut self, m: ServerMsg) {
        match m {
            ServerMsg::Welcome { .. } => self.connected = true,
            ServerMsg::Attached { session, cwd, blocks } => {
                self.session = session;
                self.cwd = cwd;
                // Karty relaci: aktualni seznam od sluzby.
                self.out.push(ClientMsg::Sessions);
                if self.welcome && blocks == 0 && !self.welcomed && self.blocks.is_empty() {
                    self.welcomed = true;
                    let items = WELCOME.iter().map(|l| Item::Note((*l).into())).collect();
                    self.blocks.push(UiBlock {
                        id: sessions::LOCAL_BLOCKS,
                        line: String::new(),
                        items,
                        ok: Some(true),
                        duration: 0,
                        collapsed: false,
                        table: None,
                        table_item: None,
                        tracks: Vec::new(),
                        started: None,
                    });
                }
                for line in core::mem::take(&mut self.pending) {
                    self.out.push(ClientMsg::Run { session, line });
                }
            }
            ServerMsg::BlockStart { block, line, .. } => {
                self.blocks.retain(|b| b.id != block);
                // Starsi blok z prehrani po pripojeni jde pred novejsi
                // (sessions.rs); novy blok na konec a zpet dolu.
                let Some(at) = self.place_block(block, KEEP_BLOCKS) else { return };
                if at == self.blocks.len() {
                    self.scroll = 0;
                } else if let Focus::Block(i) | Focus::Filter(i) = &mut self.focus
                    && *i >= at
                {
                    *i += 1;
                }
                self.blocks.insert(at, UiBlock {
                    id: block,
                    line,
                    items: Vec::new(),
                    ok: None,
                    duration: 0,
                    collapsed: false,
                    table: None,
                    table_item: None,
                    tracks: Vec::new(),
                    started: self.now,
                });
                // Blok radku, ktery okno poslalo (ne uloha async uvnitr).
                if self.expect_start {
                    self.expect_start = false;
                    self.fg = Some(block);
                }
                if self.blocks.len() > KEEP_BLOCKS && at == self.blocks.len() - 1 {
                    self.blocks.remove(0);
                    if let Focus::Block(i) | Focus::Filter(i) = self.focus {
                        self.focus = if i == 0 { Focus::Editor } else { Focus::Block(i - 1) };
                    }
                }
            }
            ServerMsg::Item { block, item, .. } => {
                let now = self.now;
                if let Some(b) = self.block(block) {
                    if b.table.is_none()
                        && let Item::Value(v) = &item
                    {
                        b.table = TableView::of(v);
                        if b.table.is_some() {
                            b.table_item = Some(b.items.len());
                        }
                    }
                    if let (Item::Progress { done, .. }, Some(t)) = (&item, now) {
                        let mut tr = Track::default();
                        tr.push(t, *done);
                        b.tracks.push((b.items.len(), tr));
                    }
                    b.items.push(item);
                }
            }
            ServerMsg::ItemUpdate { block, index, item, .. } => {
                let now = self.now;
                if let Some(b) = self.block(block)
                    && let Some(slot) = b.items.get_mut(index as usize)
                {
                    if let (Item::Progress { done, .. }, Some(t)) = (&item, now) {
                        let i = index as usize;
                        match b.tracks.iter_mut().find(|(k, _)| *k == i) {
                            Some((_, tr)) => tr.push(t, *done),
                            None => {
                                let mut tr = Track::default();
                                tr.push(t, *done);
                                b.tracks.push((i, tr));
                            }
                        }
                    }
                    *slot = item;
                }
            }
            ServerMsg::BlockEnd { block, ok, duration, .. } => {
                if let Some(b) = self.block(block) {
                    b.ok = Some(ok);
                    b.duration = duration;
                }
                self.block_ended(block);
            }
            ServerMsg::Prompt { prompt, question, detail, choices } => {
                self.prompt = Some(PromptUi { id: prompt, question, detail, choices, sel: 0, input: None });
            }
            ServerMsg::Ask { prompt, question, secret, default } => {
                self.prompt = Some(PromptUi {
                    id: prompt,
                    question,
                    detail: Item::Note(String::new()),
                    choices: Vec::new(),
                    sel: 0,
                    input: Some((secret, default)),
                });
            }
            // Sedy navrh argumentu (automaticky pozadavek, ne Tab).
            ServerMsg::Completions { seq, start, items } if seq & HINT_SEQ != 0 => {
                if seq == self.hint_req
                    && let Some((text, _, _)) = items.first()
                {
                    let start = (start as usize).min(self.editor.cursor);
                    let word = &self.editor.text[start..self.editor.cursor];
                    if let Some(rest) = text.strip_prefix(word)
                        && !rest.is_empty()
                    {
                        self.hint = Some((self.editor.seq, rest.into()));
                    }
                }
            }
            // Odpoved na starsi pozadavek (mezitim novy): zahodit.
            ServerMsg::Completions { seq, .. } | ServerMsg::Formatted { seq, .. } if seq != self.req => {}
            ServerMsg::Completions { start, items, .. } => {
                if items.is_empty() {
                    // Tab neco udelal: rict, ze neni co doplnit.
                    self.notice = Some(tasks::Notice::new("nic k doplnění", false, self.tick));
                } else if items.len() == 1 {
                    // Jedina moznost: doplnit rovnou.
                    self.apply_completion(start as usize, &items[0].0);
                } else if !items.is_empty() {
                    self.menu = Some(Menu { start: start as usize, items, sel: 0 });
                }
            }
            ServerMsg::Highlighted { seq, spans, error } => {
                let h = Editor::decode_spans(&spans, &error);
                self.editor.set_semantic(seq, h);
            }
            ServerMsg::Formatted { text, .. } => self.editor.set(&text),
            ServerMsg::Fail { message } => {
                // Zobrazi se jako samostatny blok bez prikazu.
                let id = u64::MAX - self.blocks.len() as u64;
                self.blocks.push(UiBlock {
                    id,
                    line: String::new(),
                    items: alloc::vec![Item::Error { code: "TERM".into(), message: message.clone(), report: message }],
                    ok: Some(false),
                    duration: 0,
                    collapsed: false,
                    table: None,
                    table_item: None,
                    tracks: Vec::new(),
                    started: None,
                });
            }
            ServerMsg::SessionList { sessions } => self.set_sessions(sessions),
            ServerMsg::Cwd { session, cwd } => {
                if session == self.session {
                    self.cwd = cwd;
                }
            }
            ServerMsg::Tasks { tasks } => self.set_tasks(tasks),
        }
    }

    fn apply_completion(&mut self, start: usize, text: &str) {
        let start = start.min(self.editor.cursor);
        let mut t = self.editor.text.clone();
        t.replace_range(start..self.editor.cursor, text);
        let cur = start + text.len();
        self.editor.set(&t);
        self.editor.cursor = cur.min(self.editor.text.len());
        self.highlight_request();
    }

    fn highlight_request(&mut self) {
        if self.session != 0 {
            self.out.push(ClientMsg::Highlight {
                session: self.session,
                line: self.editor.text.clone(),
                seq: self.editor.seq,
            });
            // Na pozici argumentu sedy navrh podle typu argumentu (cd =
            // adresare, jinak cesty, pole tabulky...) z doplnovani.
            self.hint = None;
            if self.arg_position() && self.editor.cursor == self.editor.text.len() {
                self.hint_req = HINT_SEQ | (self.hint_req.wrapping_add(1) & !HINT_SEQ);
                self.out.push(ClientMsg::Complete {
                    session: self.session,
                    line: self.editor.text.clone(),
                    cursor: self.editor.cursor as u32,
                    seq: self.hint_req,
                });
            }
        }
    }

    /// Kurzor je za jmenem prikazu (pise se argument)?
    fn arg_position(&self) -> bool {
        let before = &self.editor.text[..self.editor.cursor];
        let stage = before.rsplit(['|', ';']).next().unwrap_or("");
        stage.trim_start().contains(char::is_whitespace)
    }

    /// Sedy navrh na konci radku: u argumentu z doplnovani (podle typu),
    /// u jmena prikazu z historie.
    pub fn suggestion(&self) -> Option<&str> {
        if self.editor.cursor != self.editor.text.len() {
            return None;
        }
        if self.arg_position() {
            return self.hint.as_ref().filter(|(s, _)| *s == self.editor.seq).map(|(_, h)| h.as_str());
        }
        self.editor.suggestion()
    }

    fn accept_suggestion(&mut self) -> bool {
        match self.suggestion().map(String::from) {
            Some(s) => {
                self.editor.insert(&s);
                true
            }
            None => false,
        }
    }

    fn run(&mut self, line: String) {
        if line.trim().is_empty() {
            return;
        }
        if self.session == 0 {
            // Relace jeste neni (sluzba ji zaklada): radek pockat.
            self.pending.push(line);
        } else if self.fg_busy() {
            // Bezi prikaz: radek do viditelne fronty (Esc ji zrusi, sipka
            // nahoru vrati posledni k uprave).
            self.queued.push(line);
        } else {
            self.expect_start = true;
            self.out.push(ClientMsg::Run { session: self.session, line });
        }
    }

    /// Bezi radek odeslany z okna (popredi)? Jen kdyz jeho blok opravdu
    /// zacal: neodpovi-li sluzba, fronta se nezasekne (sluzba radky
    /// stejne provadi po sobe).
    pub fn fg_busy(&self) -> bool {
        self.fg.is_some()
    }

    /// Konec bloku: skoncil-li radek popredi, spustit dalsi z fronty.
    fn block_ended(&mut self, block: u64) {
        if self.fg == Some(block) {
            self.fg = None;
            if !self.queued.is_empty() {
                let next = self.queued.remove(0);
                self.run(next);
            }
        }
    }

    /// Posledni blok s tabulkou (pro PgUp z editoru).
    fn last_table(&self) -> Option<usize> {
        self.blocks.iter().rposition(|b| b.table.is_some() && !b.collapsed)
    }

    /// Klavesa.
    pub fn key(&mut self, k: Key) {
        // Posun vystupu funguje vzdy (i s nabidkou nebo v tabulce).
        let page = self.rows.saturating_sub(6).max(1);
        match k {
            Key::ScrollPageUp => return self.scroll_by(page as isize),
            Key::ScrollPageDown => return self.scroll_by(-(page as isize)),
            Key::Wheel(d) => return self.scroll_by(d as isize * 3),
            // Psani (a dalsi klavesy editoru) vraci dolu.
            _ if self.focus == Focus::Editor => self.scroll = 0,
            _ => {}
        }
        if self.prompt.is_some() {
            self.key_prompt(k);
            return;
        }
        if self.key_tasks(k) {
            return;
        }
        if k == Key::Ctrl('t') {
            self.toggle_tasks();
            return;
        }
        match self.focus {
            Focus::Editor => self.key_editor(k),
            Focus::Block(i) => self.key_block(i, k),
            Focus::Filter(i) => self.key_filter(i, k),
        }
    }

    /// Znak ukazatele cinnosti (cas z okna, jinak z tiku).
    fn spinner(&self) -> char {
        let ms = self.now.map_or(self.tick as i64 * TICK_MS, |n| n / 1_000_000);
        crate::activity::spinner(self.glyphs, ms)
    }

    fn scroll_by(&mut self, d: isize) {
        let max = self.max_scroll.get();
        self.scroll = (self.scroll as isize + d).clamp(0, max as isize) as usize;
    }

    fn key_editor(&mut self, k: Key) {
        if let Some(m) = self.menu.as_mut() {
            match k {
                Key::Up => {
                    m.sel = (m.sel + m.items.len() - 1) % m.items.len();
                    return;
                }
                Key::Down | Key::Tab => {
                    m.sel = (m.sel + 1) % m.items.len();
                    return;
                }
                Key::Enter => {
                    let (start, text) = (m.start, m.items[m.sel].0.clone());
                    self.menu = None;
                    self.apply_completion(start, &text);
                    return;
                }
                Key::Esc => {
                    self.menu = None;
                    return;
                }
                // Psani a mazani nabidku zuzi podle napsaneho.
                Key::Char(_) | Key::Backspace => {}
                _ => self.menu = None,
            }
        }
        let before = self.editor.seq;
        if !self.queued.is_empty() && self.editor.text.is_empty() {
            match k {
                Key::Esc => {
                    self.queued.clear();
                    self.notice = Some(tasks::Notice::new("fronta příkazů zrušena", false, self.tick));
                    return;
                }
                Key::Up => {
                    let l = self.queued.pop().unwrap();
                    self.editor.set(&l);
                    self.highlight_request();
                    return;
                }
                _ => {}
            }
        }
        match k {
            Key::AltLeft | Key::AltRight if self.editor.text.is_empty() => {
                self.run(String::from(if k == Key::AltLeft { "cd -" } else { "cd +" }));
            }
            Key::Char(c) => {
                let mut b = [0u8; 4];
                self.editor.insert(c.encode_utf8(&mut b));
            }
            Key::Enter => {
                let line = self.editor.take();
                self.run(line);
            }
            Key::Backspace => self.editor.backspace(),
            Key::Delete => self.editor.delete(),
            Key::Left => self.editor.left(),
            Key::Right if self.editor.cursor == self.editor.text.len() => {
                self.accept_suggestion();
            }
            Key::Right => self.editor.right(),
            Key::Home | Key::Ctrl('a') => self.editor.home(),
            Key::End | Key::Ctrl('e') => {
                if self.editor.cursor == self.editor.text.len() {
                    self.accept_suggestion();
                }
                self.editor.cursor = self.editor.text.len();
            }
            Key::Up => self.editor.history_prev(),
            Key::Down => self.editor.history_next(),
            Key::Ctrl('w') => self.editor.kill_word(),
            Key::Ctrl('u') => self.editor.clear(),
            Key::Ctrl('c') => {
                if self.busy() {
                    self.out.push(ClientMsg::Interrupt { session: self.session });
                } else {
                    self.editor.clear();
                }
            }
            // Ctrl+Z: radek v poprede na pozadi (bezi dal jako uloha),
            // vyzva se vrati a fronta radku pokracuje.
            Key::Ctrl('z') if self.fg_busy() => {
                self.out.push(ClientMsg::Background { session: self.session });
                self.fg = None;
                self.notice = Some(tasks::Notice::new("příkaz běží na pozadí (jobs, fg)", false, self.tick));
                if !self.queued.is_empty() {
                    let next = self.queued.remove(0);
                    self.run(next);
                }
            }
            Key::Ctrl('l') => {
                for b in &mut self.blocks {
                    b.collapsed = true;
                }
            }
            Key::Ctrl('f') => self.set_notation(!self.notation_c),
            Key::Tab => {
                self.req = self.req.wrapping_add(1);
                self.out.push(ClientMsg::Complete {
                    session: self.session,
                    line: self.editor.text.clone(),
                    cursor: self.editor.cursor as u32,
                    seq: self.req,
                });
            }
            Key::PageUp | Key::BackTab => {
                if let Some(i) = self.last_table() {
                    self.focus = Focus::Block(i);
                }
            }
            _ => {}
        }
        if self.editor.seq != before {
            self.filter_menu();
            self.highlight_request();
        }
    }

    /// Nabidka doplnovani jen s polozkami, ktere zacinaji napsanym slovem
    /// (bez ohledu na velikost pismen); zadna = zavrit.
    fn filter_menu(&mut self) {
        let Some(m) = self.menu.as_mut() else { return };
        if self.editor.cursor < m.start {
            self.menu = None;
            return;
        }
        let word = self.editor.text[m.start..self.editor.cursor].to_lowercase();
        m.items.retain(|(t, _, _)| t.to_lowercase().starts_with(&word));
        if m.items.is_empty() {
            self.menu = None;
        } else {
            m.sel = m.sel.min(m.items.len() - 1);
        }
    }

    fn key_block(&mut self, i: usize, k: Key) {
        let Some(b) = self.blocks.get_mut(i) else {
            self.focus = Focus::Editor;
            return;
        };
        let page = self.rows.saturating_sub(8).max(3);
        match k {
            Key::Esc | Key::Char('q') => self.focus = Focus::Editor,
            Key::Char(' ') | Key::Char('z') => b.collapsed = !b.collapsed,
            Key::Tab | Key::BackTab => {
                // Dalsi / predchozi blok s tabulkou.
                let tables: Vec<usize> =
                    self.blocks.iter().enumerate().filter(|(_, b)| b.table.is_some()).map(|(k, _)| k).collect();
                if let Some(p) = tables.iter().position(|x| *x == i) {
                    let n = tables.len();
                    let next = if k == Key::Tab { (p + 1) % n } else { (p + n - 1) % n };
                    self.focus = Focus::Block(tables[next]);
                }
            }
            _ => {
                let Some(t) = b.table.as_mut() else {
                    self.focus = Focus::Editor;
                    return;
                };
                match k {
                    Key::Up => t.move_by(-1, page),
                    Key::Down => t.move_by(1, page),
                    Key::PageUp => t.move_by(-(page as isize), page),
                    Key::PageDown => t.move_by(page as isize, page),
                    Key::Home => t.move_by(isize::MIN / 2, page),
                    Key::End => t.move_by(isize::MAX / 2, page),
                    Key::Char('s') => t.sort_next(),
                    Key::Char('S') => t.sort_reverse(),
                    Key::Char('/') => self.focus = Focus::Filter(i),
                    Key::Enter => {
                        if let Some(line) = row_action(t.selected(), Action::Open) {
                            self.focus = Focus::Editor;
                            self.run(line);
                        }
                    }
                    Key::Char('c') => {
                        if let Some(p) = row_path(t.selected()) {
                            self.focus = Focus::Editor;
                            self.editor.insert(&quote(&p));
                            self.highlight_request();
                        }
                    }
                    Key::Delete | Key::Char('d') => {
                        if let Some(line) = row_action(t.selected(), Action::Delete) {
                            self.focus = Focus::Editor;
                            self.run(line);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn key_filter(&mut self, i: usize, k: Key) {
        let Some(t) = self.blocks.get_mut(i).and_then(|b| b.table.as_mut()) else {
            self.focus = Focus::Editor;
            return;
        };
        let mut f = t.filter.clone();
        match k {
            Key::Char(c) => f.push(c),
            Key::Backspace => {
                f.pop();
            }
            Key::Enter | Key::Down => {
                self.focus = Focus::Block(i);
                return;
            }
            Key::Esc => {
                f.clear();
                self.focus = Focus::Block(i);
            }
            _ => return,
        }
        t.set_filter(&f);
    }

    /// Text napovedy klaves podle stavu.
    pub fn hints(&self) -> &'static str {
        if let Some(p) = &self.prompt {
            if p.input.is_some() {
                return "pište odpověď · Enter potvrdit · Esc zrušit";
            }
            return if p.choices.is_empty() { "a ano · n ne · šipky výběr · Enter potvrdit" } else { "1-9 volba · šipky výběr · Enter potvrdit · Esc zrušit" };
        }
        if self.tasks_open() {
            return "šipky úloha · c zrušit · p pozastavit · r pokračovat · Esc zavřít";
        }
        match self.focus {
            _ if self.scroll > 0 => "Shift+PgUp/PgDn posun výstupu · kolečko · psaní dolů",
            Focus::Editor if self.menu.is_some() => "šipky výběr · Enter/Tab doplnit · Esc zavřít",
            Focus::Editor => "Tab doplnit · šipka nahoru historie · PgUp tabulka · Shift+PgUp výstup · Ctrl-C přerušit · Ctrl-F převod B/C · Ctrl-T úlohy",
            Focus::Block(_) => "šipky řádek · s řadit · / filtr · Enter otevřít · c cesta do řádku · d smazat s náhledem · Esc zpět",
            Focus::Filter(_) => "pište filtr · Enter hotovo · Esc zrušit filtr",
        }
    }

    /// Snimek obrazovky jako radky.
    pub fn draw(&self) -> Vec<Line> {
        let t = &self.theme;
        let mut bottom: Vec<Line> = Vec::new();
        // Vyzva.
        self.draw_prompt(&mut bottom);
        // Nabidka doplnovani.
        if let Some(m) = &self.menu {
            // Okno kolem vyberu (dlouhy seznam se posouva se sipkami).
            let first = m.sel.saturating_sub(5);
            let w = m.items.iter().map(|(t, _, _)| t.chars().count()).max().unwrap_or(0).min(40);
            for (i, (text, kind, help)) in m.items.iter().enumerate().skip(first).take(6) {
                let mut l = Line::new();
                // Typ polozky: znacka a popis (slozka, soubor, prikaz...).
                let (mark, what, col) = match *kind {
                    k if k == shlang::complete::CandKind::Dir as u8 => ("\u{25B8} ", "složka", t.ui.accent),
                    k if k == shlang::complete::CandKind::File as u8 => ("\u{2022} ", "soubor", t.ui.fg),
                    k if k == shlang::complete::CandKind::Command as u8 => ("  ", "příkaz", t.syntax(shlang::highlight::Class::Command).fg),
                    k if k == shlang::complete::CandKind::Flag as u8 => ("  ", "přepínač", t.ui.fg),
                    k if k == shlang::complete::CandKind::Var as u8 => ("  ", "proměnná", t.ui.fg),
                    _ => ("  ", "sloupec", t.ui.fg),
                };
                l.push(Some(Style::fg(t.ui.muted)), &alloc::format!("  {mark}"));
                l.push(Some(Style::fg(col)), &alloc::format!("{text:<w$}"));
                let help = if help.is_empty() { what } else { help.as_str() };
                l.push(Some(Style::fg(t.ui.muted)), &alloc::format!("  {help}"));
                bottom.push(if i == m.sel { l.fill(t.ui.select) } else { l });
            }
            if m.items.len() > 6 {
                bottom.push(Line::new().text(Some(Style::fg(t.ui.muted)), &alloc::format!("    {} z {} · pište pro zúžení", m.sel + 1, m.items.len())));
            }
        }
        // Fronta radku (pobezi po aktualnim prikazu).
        for q in &self.queued {
            bottom.push(Line::new().text(Some(Style::fg(t.ui.muted)), &alloc::format!("  ve frontě: {q}")));
        }
        if !self.queued.is_empty() {
            bottom.push(Line::new().text(Some(Style::fg(t.ui.muted)), "  Esc zrušit frontu · šipka nahoru upravit"));
        }
        // Panel uloh.
        self.draw_tasks(&mut bottom);
        // Editor.
        bottom.push(self.draw_editor());
        // Napoveda klaves (okno GUI ji kresli jako stitky).
        bottom.push(
            Line::new()
                .text(Some(Style::fg(t.ui.muted)), self.hints())
                .fill(t.ui.select)
                .role(draw::Role::Hints(draw::hint_chips(self.hints()))),
        );

        let status = {
            let running = self.blocks.iter().filter(|b| b.ok.is_none()).count();
            let mut l = Line::new();
            l.push(Some(Style::fg(t.ui.muted)), &alloc::format!(" relace {} · {} ", self.session, self.cwd));
            if running > 0 {
                l.push(Some(Style::fg(t.ui.accent)), &alloc::format!("· běží {running} {} ", self.spinner()));
            }
            let jobs = self.tasks_running();
            if jobs > 0 {
                l.push(Some(Style::fg(t.ui.accent)), &alloc::format!("· úlohy {jobs} "));
            }
            if let Some(n) = self.notice() {
                l.push(Some(Style::fg(if n.error { t.ui.err } else { t.ui.ok })), &alloc::format!("· {} ", n.text));
            }
            if !self.connected {
                l.push(Some(Style::fg(t.ui.warn)), "· nepřipojeno ");
            }
            l.fill(t.ui.select).role(draw::Role::Status)
        };

        let area = self.rows.saturating_sub(bottom.len() + 1);
        // Bloky od konce, kolik se vejde (s posunem nahoru o `scroll`;
        // historie nejvys SCROLL_MAX radku).
        let want = area + SCROLL_MAX;
        let mut body: Vec<Line> = Vec::new();
        for (bi, b) in self.blocks.iter().enumerate().rev() {
            let mut lines = self.draw_block(bi, b);
            lines.push(Line::new());
            let room = (want + 1).saturating_sub(body.len());
            if room == 0 {
                break;
            }
            if lines.len() > room {
                // Blok se cely nevejde: jeho konec (nejnovejsi radky).
                let cut = lines.len() - room;
                lines.drain(..cut);
            }
            lines.append(&mut body);
            body = lines;
        }
        // Posun: zahodit spodnich `scroll` radku; nahore nic navic neni.
        let max = body.len().saturating_sub(area);
        self.max_scroll.set(max);
        let scroll = self.scroll.min(max);
        if scroll > 0 {
            body.truncate(body.len() - scroll);
            let start = body.len().saturating_sub(area);
            body.drain(..start);
            // Ukazatel: nejsem dole (posledni radek oblasti).
            if let Some(last) = body.last_mut() {
                *last = Line::new()
                    .text(Some(Style { bold: true, ..Style::fg(t.ui.accent) }), &alloc::format!(" \u{2193} níže {scroll} řádků výstupu · Shift+PgDn, kolečko nebo psaní dolů "))
                    .fill(t.ui.select);
            }
        } else if body.len() > area {
            let start = body.len() - area;
            body.drain(..start);
        }
        let mut out = alloc::vec![status];
        while body.len() < area {
            body.insert(0, Line::new());
        }
        out.extend(body);
        out.extend(bottom);
        out
    }

    fn draw_editor(&self) -> Line {
        let t = &self.theme;
        let mut l = Line::new();
        // Vyzva: kde jsem (tucne, zvyraznene), pak oddelovac.
        if self.icon_room {
            l.push(None, "  ");
        }
        l.push(Some(Style { bold: true, ..Style::fg(t.ui.accent) }), &self.cwd_short());
        l.push(Some(Style { bold: true, ..Style::fg(t.ui.fg) }), " \u{276F} ");
        let text = &self.editor.text;
        let h = self.editor.highlight();
        let cursor = self.editor.cursor;
        // Znaky se stylem podle useku; kurzor obracene (pozadi).
        let mut i = 0;
        let style_at = |p: usize| {
            let mut st = h.spans.iter().find(|s| s.start <= p && p < s.end).map(|s| t.syntax(s.class));
            if let Some(e) = &h.error
                && e.start <= p
                && p < e.end
            {
                st = Some(Style { underline: true, ..Style::fg(t.ui.err) });
            }
            st
        };
        for (p, c) in text.char_indices() {
            let mut b = [0u8; 4];
            let s = c.encode_utf8(&mut b);
            if p == cursor && self.focus == Focus::Editor && self.prompt.is_none() {
                l.push_bg(Some(Style::fg(t.ui.bg)), t.ui.fg, s);
            } else {
                l.push(style_at(p), s);
            }
            i = p + c.len_utf8();
        }
        let _ = i;
        if let Some(s) = self.suggestion() {
            let mut chars = s.chars();
            if cursor == text.len() && self.focus == Focus::Editor && let Some(first) = chars.next() {
                l.push_bg(Some(Style::fg(t.ui.bg)), t.ui.muted, first.encode_utf8(&mut [0u8; 4]));
                l.push(Some(Style::fg(t.ui.muted)), chars.as_str());
            } else {
                l.push(Some(Style::fg(t.ui.muted)), s);
            }
        } else if cursor == text.len() && self.focus == Focus::Editor && self.prompt.is_none() {
            l.push_bg(None, t.ui.fg, " ");
        }
        if let Some(e) = &h.error {
            l.push(Some(Style::fg(t.ui.err)), &alloc::format!("   {}", e.message));
        }
        l.role(draw::Role::Input { prefix: if self.icon_room { 3 } else { 2 }, cwd: self.cwd_short() })
    }

    /// Zvoli zapis B (`false`) nebo C (`true`) a rozepsany radek do nej
    /// prevede (odpoved `Formatted`). Stejny zapis = nic.
    pub fn set_notation(&mut self, c: bool) {
        if self.notation_c == c {
            return;
        }
        self.notation_c = c;
        if self.editor.text.trim().is_empty() {
            return;
        }
        self.req = self.req.wrapping_add(1);
        self.out.push(ClientMsg::Format { session: self.session, line: self.editor.text.clone(), style: c as u8, seq: self.req });
    }

    fn cwd_short(&self) -> String {
        short_path(&self.cwd, (self.cols / 3).max(12))
    }

    fn draw_block(&self, bi: usize, b: &UiBlock) -> Vec<Line> {
        let t = &self.theme;
        let focused = matches!(self.focus, Focus::Block(i) | Focus::Filter(i) if i == bi);
        let mark_color = match b.ok {
            None => t.ui.accent,
            Some(true) => t.ui.ok,
            Some(false) => t.ui.err,
        };
        let mark = match b.ok {
            None => draw::Mark::Run,
            Some(true) => draw::Mark::Ok,
            Some(false) => draw::Mark::Err,
        };
        let block = b.id;
        let body = |error: bool, table_head: bool| draw::Role::Body { mark, error, table_head, block };
        let mut out = Vec::new();
        if !b.line.is_empty() {
            let mut l = Line::new();
            l.push(Some(Style { bold: true, ..Style::fg(mark_color) }), if focused { "> " } else { "| " });
            let h = shlang::highlight::highlight(&b.line, None);
            let mut at = 0;
            // Prikaz tucne (navrh: hlavicka bloku).
            let plain = Some(Style { bold: true, ..Style::fg(t.ui.fg) });
            for s in &h.spans {
                l.push(plain, &b.line[at..s.start]);
                l.push(Some(Style { bold: true, ..t.syntax(s.class) }), &b.line[s.start..s.end]);
                at = s.end;
            }
            l.push(plain, &b.line[at..]);
            // Od tohoto useku cas a vysledek (okno GUI je kresli vpravo).
            let meta = l.parts_len();
            match b.ok {
                None => l.push(Some(Style::fg(t.ui.accent)), &alloc::format!("  {}", self.spinner())),
                Some(ok) if b.duration > 0 || !ok => l.push(
                    Some(Style::fg(if ok { t.ui.muted } else { t.ui.err })),
                    &alloc::format!(
                        "  {}{}",
                        if ok { "" } else { "neúspěch " },
                        if b.duration > 0 { shlang::value::human_duration(b.duration) } else { String::new() }
                    ),
                ),
                _ => {}
            }
            out.push(l.role(draw::Role::Head { mark, meta, block }));
        }
        if b.collapsed {
            out.push(Line::new().text(Some(Style::fg(t.ui.muted)), &alloc::format!("  ({} položek, mezera rozbalí)", b.items.len())).role(body(false, false)));
            return out;
        }
        for (ii, it) in b.items.iter().enumerate() {
            match it {
                Item::Value(_) if b.table_item == Some(ii) => {
                    if let Some(t) = &b.table {
                        for (k, l) in self.draw_table(t, focused).into_iter().enumerate() {
                            out.push(l.role(body(false, k == 0)));
                        }
                    }
                }
                Item::Progress { done, total, label } => {
                    let track = b.tracks.iter().find(|(k, _)| *k == ii).map(|(_, tr)| tr);
                    out.push(self.draw_progress(*done, *total, label, track, b.ok.is_none()).role(body(false, false)));
                }
                other => {
                    let error = matches!(other, Item::Error { .. });
                    let text = crate::vt::item_at(other, Some(t), self.now);
                    // Dlouhe radky (uvitani, hlaseni) zalomit do sirky okna.
                    // Styl nastaveny na zacatku viceradkoveho textu (barva
                    // chyby) plati i na dalsich radcich a jejich zalomeni.
                    let mut carry = String::new();
                    for line in draw::lines_of(&text) {
                        let line = alloc::format!("{carry}{line}");
                        carry = draw::active_sgr(&line);
                        for part in draw::wrap(&line, self.cols.saturating_sub(2)) {
                            out.push(Line::new().text(None, &alloc::format!("  {part}")).role(body(error, false)));
                        }
                    }
                }
            }
        }
        // Bezici radek popredi dele nez 1 s: cas a jak dal.
        if b.ok.is_none()
            && self.fg == Some(b.id)
            && let (Some(s), Some(n)) = (b.started, self.now)
            && n - s >= 1_000_000_000
        {
            let secs = (n - s) / 1_000_000_000;
            let mut l = Line::new();
            l.push(Some(Style::fg(t.ui.accent)), &alloc::format!("  {} ", self.spinner()));
            l.push(Some(Style::fg(t.ui.muted)), &alloc::format!("běží {} · Ctrl+C přerušit · Ctrl+Z na pozadí", progress::human_eta(secs * 1_000_000_000)));
            if !self.queued.is_empty() {
                l.push(Some(Style::fg(t.ui.muted)), &alloc::format!(" · ve frontě {}", self.queued.len()));
            }
            out.push(l.role(body(false, false)));
        }
        out
    }

    fn draw_table(&self, tv: &TableView, focused: bool) -> Vec<Line> {
        let t = &self.theme;
        let height = if focused { self.rows.saturating_sub(10).max(3) } else { TABLE_ROWS };
        // Sirky sloupcu z viditelnych radku.
        let rows: Vec<(bool, &Value)> = tv.visible(height).collect();
        let mut w: Vec<usize> = tv.cols.iter().map(|c| c.chars().count()).collect();
        for (_, r) in &rows {
            for (i, c) in tv.cols.iter().enumerate() {
                let n = r.field(c).map_or(0, |v| shlang::table::cell_text(&v).chars().count());
                w[i] = w[i].max(n).min(40);
            }
        }
        let mut out = Vec::new();
        let mut head = Line::new();
        head.push(None, "  ");
        for (i, c) in tv.cols.iter().enumerate() {
            let mark = match tv.sort {
                Some((s, d)) if s == i => if d { " v" } else { " ^" },
                _ => "",
            };
            head.push(Some(Style { bold: true, ..Style::fg(t.ui.muted) }), &alloc::format!("{:<width$}  ", alloc::format!("{c}{mark}"), width = w[i]));
        }
        out.push(head);
        for (sel, r) in rows {
            let mut l = Line::new();
            l.push(None, "  ");
            for (i, c) in tv.cols.iter().enumerate() {
                let v = r.field(c).unwrap_or(Value::Nothing);
                let text: String = shlang::table::cell_text(&v).chars().take(w[i]).collect();
                let st = cell_style(c, &v, r, t, self.now);
                let pad = w[i] - text.chars().count();
                let cell = if shlang::table::is_numeric(&v) {
                    alloc::format!("{}{text}  ", " ".repeat(pad))
                } else {
                    alloc::format!("{text}{}  ", " ".repeat(pad))
                };
                l.push(st, &cell);
            }
            out.push(if sel && focused { l.fill(t.ui.select) } else { l });
        }
        let more = tv.view.len().saturating_sub(tv.top + height);
        let mut foot = alloc::format!("  {}", tv.status());
        if more > 0 {
            foot.push_str(&alloc::format!(" · další {more} (PgUp/PgDn)"));
        }
        if matches!(self.focus, Focus::Filter(_)) && focused {
            foot = alloc::format!("  filtr: {}_", tv.filter);
        }
        out.push(Line::new().text(Some(Style::fg(t.ui.muted)), &foot));
        out
    }

    fn draw_progress(&self, done: u64, total: u64, label: &str, track: Option<&Track>, running: bool) -> Line {
        let t = &self.theme;
        let mut l = Line::new();
        l.push(None, "  ");
        let width = (self.cols / 3).clamp(10, 40);
        if total == 0 && running {
            // Neznamy celek: ukazatel cinnosti a pocet hotovych.
            l.push(Some(Style::fg(t.ui.accent)), &alloc::format!("{} ", self.spinner()));
            l.push(Some(Style::fg(t.ui.muted)), &alloc::format!("{done} {label}"));
            return l;
        }
        let pct = (done.min(total) * 100 / total) as usize;
        // Blok skoncil pred 100 % (break, chyba, preruseni): zastaveny pruh.
        let color = if pct >= 100 {
            t.ui.ok
        } else if running {
            t.ui.accent
        } else {
            t.ui.muted
        };
        // Vyhlazeny pruh (osminy bunky) ze spolecneho modulu.
        let frac = done.min(total) as f64 / total as f64;
        l.push_bg(Some(Style::fg(color)), t.ui.select, &crate::activity::bar(self.glyphs, width, frac));
        l.push(Some(Style { bold: true, ..Style::fg(t.ui.fg) }), &alloc::format!("  {pct} %"));
        let mut info = alloc::format!("  {done} / {total} {label}");
        if !running && pct < 100 {
            info.push_str(" · zastaveno");
        }
        if running && let Some(tr) = track {
            if let Some(r) = tr.rate() {
                let rate = progress::human_rate(r, label);
                if !rate.is_empty() {
                    info.push_str(&alloc::format!(" · {rate}"));
                }
            }
            if let Some(now) = self.now
                && let Some(eta) = tr.eta_text(now, total)
            {
                info.push_str(&alloc::format!(" · zbývá {eta}"));
            }
        }
        l.push(Some(Style::fg(t.ui.muted)), &info);
        l
    }

    /// Snimek jako text s ANSI pro [`crate::screen::Screen`] (smaze a kresli
    /// od leveho horniho rohu).
    pub fn frame(&self) -> String {
        self.frame_lines().0
    }

    /// Snimek jako text (pro obrazovku) i radky s rolemi (okno GUI podle
    /// nich dokresli prvky navrhu); radek `i` = radek obrazovky `i`.
    pub fn frame_lines(&self) -> (String, Vec<Line>) {
        let mut s = String::from("\x1b[2J\x1b[H");
        let mut lines = self.draw();
        lines.truncate(self.rows);
        let n = lines.len();
        for (i, l) in lines.iter().enumerate() {
            s.push_str(&l.ansi(self.cols));
            if i + 1 < n {
                s.push('\n');
            }
        }
        (s, lines)
    }
}

/// Styl bunky (stejne jako barevny vypis).
fn cell_style(col: &str, v: &Value, rec: &Value, t: &Theme, now: Option<i64>) -> Option<Style> {
    // Barvy z `render` pres jednoradkovou tabulku by byly drahe; tady
    // stejna pravidla primo.
    match v {
        Value::Size(b) => Some(Style::fg(t.scale[render::size_level(*b)])),
        Value::Time(m) => now.map(|n| Style::fg(t.scale[render::age_level(*m, n)])),
        Value::Bool(true) => Some(Style::fg(t.ui.ok)),
        Value::Bool(false) | Value::Error(_) => Some(Style::fg(t.ui.err)),
        Value::Null | Value::Nothing => Some(Style::fg(t.ui.muted)),
        _ if col == "name" => render::file_class(rec, t).map(|c| t.file(c)),
        _ if matches!(col, "kind" | "ext" | "access" | "dataset") => Some(Style::fg(t.ui.muted)),
        _ => None,
    }
}

/// Akce nad vybranym radkem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Slozka: `cd` a vypis; soubor: `open`.
    Open,
    /// Smazat s nahledem (`preview { rm ... }`).
    Delete,
}

fn str_field(v: &Value, f: &str) -> Option<String> {
    match v.field(f) {
        Some(Value::Str(s)) => Some(s),
        _ => None,
    }
}

/// Cesta radku (`path`, jinak `name`).
pub fn row_path(row: Option<&Value>) -> Option<String> {
    let r = row?;
    str_field(r, "path").or_else(|| str_field(r, "name"))
}

/// Slovo zapisu B v uvozovkach, kdyz je treba. Jmeno souboru je data,
/// ne kod: escapuje se i vlozeny vyraz `{...}` (jinak by jmeno
/// `x{exec(...)}` pri akci nad radkem spustilo kod).
pub fn quote(p: &str) -> String {
    if p.chars().all(|c| c.is_alphanumeric() || "/._-~".contains(c)) {
        return p.into();
    }
    let mut out = String::from("\"");
    for c in p.chars() {
        if matches!(c, '\\' | '"' | '{' | '}') {
            out.push('\\');
        }
        match c {
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Kolik radku vystupu jde posunout nahoru (starsi bloky se stejne
/// nekresli, okno je jen nahled poslednich bloku).
const SCROLL_MAX: usize = 2000;

/// Priznak `seq` automatickeho pozadavku na sedy navrh (Tab a format
/// pouzivaji cisla bez nej).
const HINT_SEQ: u32 = 0x8000_0000;

/// Cesta pro vyzvu: domov `/home/<jmeno>` jako `~`, dlouha cesta jen
/// s poslednimi slozkami (`.../a/b`) do `max` znaku.
pub fn short_path(cwd: &str, max: usize) -> String {
    let mut p = String::from(cwd);
    if let Some(rest) = cwd.strip_prefix("/home/") {
        let (_, tail) = rest.split_once('/').unwrap_or((rest, ""));
        p = if tail.is_empty() { "~".into() } else { alloc::format!("~/{tail}") };
    }
    if p.chars().count() <= max {
        return p;
    }
    // Od konce cele slozky, dokud se vejdou.
    let parts: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
    let mut out = String::new();
    for part in parts.iter().rev() {
        let next = if out.is_empty() { String::from(*part) } else { alloc::format!("{part}/{out}") };
        if next.chars().count() + 2 > max && !out.is_empty() {
            break;
        }
        out = next;
    }
    alloc::format!("\u{2026}/{out}")
}

/// Prikaz akce nad radkem (spusti se jako bezny radek - vidi ho historie
/// a plati pro nej nahled, zamek i undo).
pub fn row_action(row: Option<&Value>, a: Action) -> Option<String> {
    let path = row_path(row)?;
    let dir = row.and_then(|r| str_field(r, "kind")).is_some_and(|k| k == "dir");
    let p = quote(&path);
    Some(match a {
        Action::Open if dir => alloc::format!("cd {p}; ls"),
        Action::Open => alloc::format!("open {p}"),
        Action::Delete if dir => alloc::format!("preview {{ rm -r {p} }}"),
        Action::Delete => alloc::format!("preview {{ rm {p} }}"),
    })
}
