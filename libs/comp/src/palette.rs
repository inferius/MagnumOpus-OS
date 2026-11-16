//! Paleta prikazu v2 (`docs/cs/gui.md`, Pravidla palety prikazu;
//! `design/prostredi-menu.html`): systemova zkratka Ctrl+K nebo Super+K,
//! hledani psanim. Kresli ji kompozitor nad vsemi okny (aplikace ji
//! neprekryje ani nezachyti), klavesnice i mys, Esc zavre (fokus zustava
//! v puvodnim okne).
//!
//! Sekce: bez psani Nedavne (5), Tato aplikace (5 nejcastejsich), System;
//! pri psani akce aktivni aplikace, Okna, Aplikace, System, nejvys 5 v sekci
//! a "dalsi...". Rozsah: Tab prepina vse / tato aplikace / system,
//! predpona `>` = jen akce aplikace. Akce aplikace jsou polozky jejiho menu
//! (`MENU_UPDATE`, `menu.rs`) a spousti se pres API akci jako volani
//! `actions` (overeni argumentu, potvrzeni u `confirm`).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::Canvas;
use guiproto::Rect;
use inputproto::code::{KEY_DOWN, KEY_ENTER, KEY_ESC, KEY_K, KEY_KPENTER, KEY_TAB, KEY_UP};
use inputproto::mods;

use crate::Comp;

/// Nejvic polozek v sekci (zbytek jako "dalsi...").
pub const SECTION: usize = 5;
/// Nejvic radku palety (vcetne nadpisu).
pub const ROWS: usize = 14;
/// Kolik nedavnych polozek si paleta pamatuje.
const RECENT: usize = 8;
/// Volajici "paleta" pro API akci (odpoved nikam nejde).
pub const PALETTE_PEER: crate::PeerId = u32::MAX;

/// Systemova akce palety.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysAction {
    Shutdown,
    Reboot,
    Snapshot,
    /// Cela obrazovka okna s fokusem (prepinac).
    Fullscreen,
}

/// Polozka palety.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// Spustit (nebo aktivovat) aplikaci: id z manifestu, jmeno.
    App(String, String),
    /// Prepnout na otevrene okno.
    Window(u32),
    System(SysAction),
    /// Akce aplikace z menu okna: okno, index v `Window::menu`.
    AppAction(u32, usize),
    /// Dalsi okno bezici aplikace: id, jmeno.
    NewWindow(String, String),
}

/// Radek palety: nadpis sekce, polozka, nebo pocet dalsich.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// Nadpis: text, ikona, `true` = sekce System (stit duvery).
    Header(String, &'static str, bool),
    Item(Item),
    More(usize),
}

/// Rozsah hledani (Tab).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    #[default]
    All,
    App,
    System,
}

/// Otevrena paleta.
#[derive(Debug, Clone, Default)]
pub struct Palette {
    pub query: String,
    /// Vybrana polozka (index mezi spustitelnymi radky).
    pub sel: usize,
    pub scope: Scope,
    /// Spoustec (varianta D, design/spoustec.html): vlevo nad listou,
    /// prazdny dotaz = sekce Bezi, Aplikace, System; psani = stejne
    /// vysledky jako paleta.
    pub launcher: bool,
}

/// Zadost palety, kterou vyridi sluzba (init): vypnout, restartovat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemRequest {
    Shutdown,
    Reboot,
}

/// Mala pismena bez diakritiky ("ulozit" najde "Ulozit").
pub(crate) fn fold(s: &str) -> String {
    s.chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| match c {
            'á' | 'ä' => 'a',
            'č' => 'c',
            'ď' => 'd',
            'é' | 'ě' | 'ë' => 'e',
            'í' => 'i',
            'ň' => 'n',
            'ó' | 'ö' => 'o',
            'ř' => 'r',
            'š' => 's',
            'ť' => 't',
            'ú' | 'ů' | 'ü' => 'u',
            'ý' => 'y',
            'ž' => 'z',
            c => c,
        })
        .collect()
}

/// Shoda dotazu `q` (uz `fold`) s textem: 3 = zacatek, 2 = zkratka ze
/// zacatku slov ("nzk" = "Nova zalozka"), 1 = uvnitr / klicove slovo.
fn score(label: &str, keywords: &[String], q: &str) -> Option<u32> {
    if q.is_empty() {
        return Some(1);
    }
    let l = fold(label);
    if l.starts_with(q) || l.split([' ', '›']).any(|w| w.starts_with(q)) {
        return Some(3);
    }
    let initials: String = l.split(|c: char| !c.is_alphanumeric()).filter_map(|w| w.chars().next()).collect();
    // Zkratka smi zacit u kterehokoli slova ("nz" v "Soubor › Nova zalozka").
    if q.chars().count() >= 2 && initials.contains(q) {
        return Some(2);
    }
    if l.contains(q) || keywords.iter().any(|k| fold(k).contains(q)) {
        return Some(1);
    }
    None
}

impl Item {
    /// Klic pro nedavne a cetnost (akce podle aplikace a id akce).
    fn key(&self, c: &Comp) -> String {
        match self {
            Item::App(id, _) => format!("app:{id}"),
            Item::NewWindow(id, _) => format!("new:{id}"),
            Item::Window(g) => format!("win:{g}"),
            Item::System(a) => format!("sys:{a:?}"),
            Item::AppAction(g, i) => {
                let w = c.windows.get(g);
                let app = w.and_then(|w| c.clients.get(&w.client)).map_or("", |cl| cl.info.id.as_str());
                let act = w.and_then(|w| w.menu.get(*i)).map_or("", |f| f.item.action.as_str());
                format!("act:{app}:{act}")
            }
        }
    }
}

impl Comp {
    /// Je klavesa zkratkou palety (Ctrl+K nebo Super+K)?
    pub(crate) fn is_palette_shortcut(code: u16, m: u16) -> bool {
        code == KEY_K && m & (mods::CTRL | mods::META) != 0 && m & (mods::ALT | mods::SHIFT) == 0
    }

    pub fn palette_open(&self) -> bool {
        self.palette.is_some()
    }

    pub(crate) fn set_palette(&mut self, p: Option<Palette>) {
        if let Some(r) = self.palette_visual() {
            self.damage(r);
        }
        let open = p.is_some();
        self.palette = p;
        // Pohled nad knihovnou ui: sestavit znovu, zavrenim zahodit.
        if open {
            self.sys_rebuild();
        } else {
            self.sysui = None;
        }
        if let Some(r) = self.palette_visual() {
            self.damage(r);
        }
    }

    /// Popis polozky pro pohled (`sysui.rs`).
    pub(crate) fn palette_item_view(&self, it: &Item) -> (String, &'static str, String, Option<String>) {
        self.item_view(it)
    }

    /// Popis polozky: text, ikona, vpravo (zkratka / skupina), duvod zakazu.
    fn item_view(&self, it: &Item) -> (String, &'static str, String, Option<String>) {
        match it {
            Item::App(id, name) => {
                let n = self.app_windows(id).len();
                let mut what = match n {
                    0 => String::new(),
                    1 => String::from("běží"),
                    n => format!("{n} okna"),
                };
                // Multi: tlacitko nove okno vpravo (Ctrl+Enter).
                if self.is_launchable(id) {
                    what = if what.is_empty() { String::from("+ nové okno (Ctrl+Enter)") } else { format!("{what}, + nové okno (Ctrl+Enter)") };
                }
                (name.clone(), crate::render::app_icon(id), what, None)
            }
            Item::NewWindow(id, name) => (format!("Nové okno: {name}"), crate::render::app_icon(id), String::from("Ctrl+Shift+N"), None),
            Item::Window(g) => {
                let w = self.windows.get(g);
                let title = w.map_or(String::new(), |w| w.title.clone());
                let icon = w.and_then(|w| self.clients.get(&w.client)).map_or("window", |c| crate::render::app_icon(&c.info.id));
                (title, icon, String::new(), None)
            }
            Item::System(SysAction::Shutdown) => (String::from("Vypnout"), "power", String::new(), None),
            Item::System(SysAction::Reboot) => (String::from("Restartovat"), "restart", String::new(), None),
            Item::System(SysAction::Snapshot) => (String::from("Diagnostický snímek"), "snapshot", String::from("Print Screen"), None),
            Item::System(SysAction::Fullscreen) => {
                let on = self.focus.is_some_and(|g| self.is_fullscreen(g));
                let label = if on { "Opustit celou obrazovku" } else { "Celá obrazovka" };
                let dis = self.focus.is_none().then(|| String::from("žádné okno"));
                (String::from(label), "fullscreen", String::from("Super+Shift+F"), dis)
            }
            Item::AppAction(g, i) => {
                let Some(f) = self.windows.get(g).and_then(|w| w.menu.get(*i)) else {
                    return (String::new(), "command", String::new(), None);
                };
                let check = match f.item.checked {
                    Some(true) => " (zapnuto)",
                    Some(false) => " (vypnuto)",
                    None => "",
                };
                let short = f.item.role.system_text().map_or(f.item.shortcut.clone(), |(_, s)| String::from(s));
                let dis = (!f.item.disabled.is_empty()).then(|| f.item.disabled.clone());
                (format!("{}{check}", f.path), "command", short, dis)
            }
        }
    }

    fn keywords(&self, it: &Item) -> Vec<String> {
        match it {
            Item::AppAction(g, i) => self.windows.get(g).and_then(|w| w.menu.get(*i)).map_or(Vec::new(), |f| f.item.keywords.clone()),
            Item::System(SysAction::Shutdown) => alloc::vec![String::from("vypnuti"), String::from("power")],
            Item::NewWindow(..) => alloc::vec![String::from("nový"), String::from("instance")],
            Item::System(SysAction::Snapshot) => alloc::vec![String::from("diagnostika"), String::from("screenshot")],
            _ => Vec::new(),
        }
    }

    fn usage(&self, it: &Item) -> u32 {
        self.palette_usage.get(&it.key(self)).copied().unwrap_or(0)
    }

    /// Okno, ke kteremu patri "Tato aplikace" (fokus; paleta ho nemeni).
    fn palette_app(&self) -> Option<(u32, String, &'static str)> {
        let g = self.focus?;
        let w = self.windows.get(&g)?;
        let cl = self.clients.get(&w.client)?;
        Some((g, cl.info.name.clone(), crate::render::app_icon(&cl.info.id)))
    }

    /// Radky palety podle dotazu a rozsahu.
    pub fn palette_rows(&self) -> Vec<Row> {
        let Some(p) = &self.palette else { return Vec::new() };
        let (mut scope, mut q) = (p.scope, p.query.as_str());
        if let Some(rest) = q.strip_prefix('>') {
            scope = Scope::App;
            q = rest.trim_start();
        }
        let q = fold(q);
        let app = self.palette_app();
        let app_items: Vec<Item> = app.as_ref().map_or(Vec::new(), |(g, ..)| {
            self.windows.get(g).map_or(Vec::new(), |w| (0..w.menu.len()).map(|i| Item::AppAction(*g, i)).collect())
        });
        let windows: Vec<Item> = self.z.iter().rev().filter(|g| self.windows.get(g).is_some_and(|w| w.mapped)).map(|&g| Item::Window(g)).collect();
        let apps: Vec<Item> = self.launchable.iter().map(|(id, n)| Item::App(id.clone(), n.clone())).collect();
        // Pod aplikaci jeji otevrena okna (podradky, pri vic oknech).
        let with_windows = |list: Vec<Item>| -> Vec<Item> {
            let mut out = Vec::new();
            for it in list {
                let wins = match &it {
                    Item::App(id, _) if self.app_windows(id).len() > 1 => self.app_windows(id),
                    _ => Vec::new(),
                };
                out.push(it);
                out.extend(wins.into_iter().map(Item::Window));
            }
            out
        };
        let system: Vec<Item> = [SysAction::Fullscreen, SysAction::Snapshot, SysAction::Reboot, SysAction::Shutdown].map(Item::System).to_vec();
        let app_head = app.as_ref().map(|(_, n, ic)| Row::Header(n.clone(), ic, false));
        let sys_head = Row::Header(String::from("Systém"), "shieldcheck", true);
        // Sekce: (nadpis, polozky).
        let mut sections: Vec<(Row, Vec<Item>)> = Vec::new();
        let ranked = |items: Vec<Item>| -> Vec<Item> {
            let mut v: Vec<(u32, u32, usize, Item)> = items
                .into_iter()
                .enumerate()
                .filter_map(|(k, it)| {
                    let (label, ..) = self.item_view(&it);
                    score(&label, &self.keywords(&it), &q).map(|s| (s, self.usage(&it), k, it))
                })
                .collect();
            v.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
            v.into_iter().map(|x| x.3).collect()
        };
        if q.is_empty() && p.launcher {
            // Spoustec (spoustec.html, varianta d): Bezi, Aplikace, Nedavne;
            // napajeni a ucet jsou v pate, ne v seznamu.
            let running: Vec<Item> = apps.iter().filter(|i| matches!(i, Item::App(id, _) if !self.app_windows(id).is_empty())).cloned().collect();
            let rest: Vec<Item> = apps.iter().filter(|i| !running.contains(i)).cloned().collect();
            let all: Vec<Item> = apps.iter().chain(&windows).cloned().collect();
            // Nedavne: co neni vyse (okna, akce), at se radky neopakuji.
            let recent: Vec<Item> = self.palette_recent.iter().filter_map(|k| all.iter().find(|i| &i.key(self) == k).cloned()).filter(|i| !matches!(i, Item::App(..))).take(3).collect();
            sections.push((Row::Header(String::from("Běží"), "window", false), running));
            sections.push((Row::Header(String::from("Aplikace"), "apps", false), rest));
            sections.push((Row::Header(String::from("Nedávné"), "history", false), recent));
        } else if q.is_empty() {
            match scope {
                Scope::All => {
                    let all: Vec<Item> = app_items.iter().chain(&windows).chain(&apps).chain(&system).cloned().collect();
                    let recent: Vec<Item> = self.palette_recent.iter().filter_map(|k| all.iter().find(|i| &i.key(self) == k).cloned()).collect();
                    if !recent.is_empty() {
                        sections.push((Row::Header(String::from("Nedávné"), "history", false), recent));
                    }
                    if let Some(h) = app_head.clone() {
                        let mut top = app_items.clone();
                        top.sort_by_key(|i| core::cmp::Reverse(self.usage(i)));
                        sections.push((h, top));
                    }
                    sections.push((sys_head.clone(), system.clone()));
                }
                Scope::App => {
                    if let Some(h) = app_head.clone() {
                        sections.push((h, app_items.clone()));
                    }
                }
                Scope::System => sections.push((sys_head.clone(), system.clone())),
            }
        } else {
            if scope != Scope::System
                && let Some(h) = app_head.clone()
            {
                sections.push((h, ranked(app_items.clone())));
            }
            if scope == Scope::All {
                sections.push((Row::Header(String::from("Okna"), "window", false), ranked(windows)));
                sections.push((Row::Header(String::from("Aplikace"), "apps", false), with_windows(ranked(apps))));
            }
            if scope != Scope::App {
                sections.push((sys_head, ranked(system)));
            }
        }
        let mut rows = Vec::new();
        for (head, items) in sections {
            if items.is_empty() {
                continue;
            }
            rows.push(head);
            let n = items.len();
            // Spoustec ukaze cele sekce ("dalsi..." v nem nema kam vest).
            let cap = if p.launcher { ROWS } else { SECTION };
            rows.extend(items.into_iter().take(cap).map(Row::Item));
            if n > cap {
                rows.push(Row::More(n - cap));
            }
        }
        rows.truncate(ROWS);
        rows
    }

    /// Spustitelne polozky v poradi radku (vyber, Enter).
    pub fn palette_items(&self) -> Vec<Item> {
        self.palette_rows()
            .into_iter()
            .filter_map(|r| match r {
                Row::Item(i) => Some(i),
                _ => None,
            })
            .collect()
    }

    /// Paleta i se stinem (poskozeni).
    pub(crate) fn palette_visual(&self) -> Option<Rect> {
        let r = self.palette_rect()?;
        let mut v = r;
        for s in &self.style.theme.shadow.popover {
            let e = s.blur as i32 + s.spread;
            v = v.union(&Rect::new(r.x + s.x - e, r.y + s.y - e, r.w + 2 * e.max(0) as u32, r.h + 2 * e.max(0) as u32));
        }
        Some(v)
    }

    /// Obdelnik palety: nahore uprostred obrazovky.
    pub(crate) fn palette_rect(&self) -> Option<Rect> {
        self.palette.as_ref()?;
        let t = &self.style.theme;
        let w = t.size.dialog_w.max(520).min(self.w.saturating_sub(2 * t.space[5]));
        let n = self.palette_rows().len();
        let rows = n.clamp(1, ROWS) as u32;
        // Nic nenalezeno: misto na prazdny stav (`ui::empty_state`: kruh
        // s ikonou, nadpis, popis, odsazeni).
        let list_h = if n == 0 && self.palette_query_set() {
            let xl = t.size.icon_xl;
            (2 * xl + xl / 4) + t.text.title.line + t.text.body.line + 2 * t.space[3] + 2 * t.space[6]
        } else {
            rows * self.palette_row_h()
        };
        // Okraje pohledu knihovny ui (pad) a mezery mezi polem a seznamem.
        let h = self.palette_input_h() + list_h + 3 * t.space[3] + 2 * t.space[5];
        if self.palette.as_ref().is_some_and(|p| p.launcher) {
            // Spoustec: vlevo nad listou (u tlacitka mrizky), sirka podle
            // tokenu `size.launcher_w`; pata (ucet, napajeni) navic.
            let w = t.size.launcher_w.min(self.w.saturating_sub(2 * t.space[3]));
            let h = h + self.palette_row_h() + t.space[3];
            let bottom = self.panel_rect().y - t.space[3] as i32;
            let h = h.min((bottom - t.space[3] as i32).max(1) as u32);
            return Some(Rect::new(self.panel_rect().x, bottom - h as i32, w, h));
        }
        Some(Rect::new((self.w as i32 - w as i32) / 2, (self.h / 8) as i32, w, h))
    }

    /// Je v palete napsany dotaz?
    pub(crate) fn palette_query_set(&self) -> bool {
        self.palette.as_ref().is_some_and(|p| !p.query.trim().is_empty())
    }

    fn palette_input_h(&self) -> u32 {
        self.style.theme.size.control_h + self.style.theme.space[3]
    }

    fn palette_row_h(&self) -> u32 {
        self.style.theme.size.control_h
    }

    /// Radek palety na vysce `y` (obrazovka): index v `palette_rows`.
    pub(crate) fn palette_row_at(&self, y: i32) -> Option<usize> {
        let r = self.palette_rect()?;
        let top = r.y + (self.style.theme.space[3] + self.palette_input_h()) as i32;
        let i = (y - top).div_euclid(self.palette_row_h() as i32);
        (y >= top && (i as usize) < self.palette_rows().len()).then_some(i as usize)
    }

    /// Klavesa pri otevrene palete (vse ji konci, aplikaci nejde nic).
    pub(crate) fn palette_key(&mut self, code: u16, press: bool, time: u64) {
        if !press {
            return;
        }
        // Otevrena nabidka v pohledu (napajeni): klavesy patri ji.
        if self.sys_popup_open() {
            let m = if self.palette_ctrl { mods::CTRL } else { 0 };
            self.sys_event(guiproto::window::InputEvent::Key { code, action: 1, ch: None }, m, time);
            return;
        }
        let n = self.palette_items().len();
        let Some(p) = self.palette.clone() else { return };
        let mut p2 = p.clone();
        match code {
            KEY_ESC => return self.set_palette(None),
            KEY_ENTER | KEY_KPENTER => {
                // Ctrl+Enter na aplikaci (multi) = nove okno.
                let it = self.palette_items().get(p.sel).cloned();
                if self.palette_ctrl
                    && let Some(Item::App(id, n)) = &it
                    && self.is_launchable(id)
                {
                    self.palette_run(Item::NewWindow(id.clone(), n.clone()), time);
                } else if let Some(it) = it {
                    self.palette_run(it, time);
                }
                return;
            }
            KEY_TAB => {
                p2.scope = match p.scope {
                    Scope::All => Scope::App,
                    Scope::App => Scope::System,
                    Scope::System => Scope::All,
                };
                p2.sel = 0;
            }
            KEY_DOWN if n > 0 => p2.sel = (p.sel + 1).min(n - 1),
            KEY_UP => p2.sel = p.sel.saturating_sub(1),
            // Editace pole hledani (Backspace, sipky, Ctrl+Z...) v pohledu.
            _ => {
                let m = if self.palette_ctrl { mods::CTRL } else { 0 };
                self.sys_event(guiproto::window::InputEvent::Key { code, action: 1, ch: None }, m, time);
                return;
            }
        }
        self.set_palette(Some(p2));
    }

    /// Psany znak do hledani (nikam se neposila ani neloguje).
    pub(crate) fn palette_text(&mut self, ch: char) {
        if ch.is_control() {
            return;
        }
        if self.palette.is_some() {
            let t = self.input_now;
            self.sys_event(guiproto::window::InputEvent::Text(ch), 0, t);
        }
    }

    /// Klik pri otevrene palete: radek provede, mimo paletu zavre.
    pub(crate) fn palette_click(&mut self, x: i32, y: i32, time: u64) {
        let inside = self.palette_rect().is_some_and(|r| r.contains(x, y));
        if !inside {
            return self.set_palette(None);
        }
        // Klik do pole hledani, do paty nebo do otevrene nabidky: pohled ui.
        if let Some(r) = self.palette_rect()
            && (y < r.y + (self.style.theme.space[3] + self.palette_input_h()) as i32 || self.palette_row_at(y).is_none() || self.sys_popup_open())
        {
            let pad = self.style.theme.space[3] as i32;
            let (lx, ly) = (x - r.x - pad, y - r.y - pad);
            for pressed in [true, false] {
                self.sys_event(guiproto::window::InputEvent::Button { code: inputproto::code::BTN_LEFT, pressed, x: lx, y: ly }, 0, time);
            }
            return;
        }
        if let Some(i) = self.palette_row_at(y)
            && let Some(Row::Item(it)) = self.palette_rows().get(i).cloned()
        {
            // Tlacitko nove okno vpravo na radku aplikace (multi).
            let plus = self.palette_rect().is_some_and(|r| x >= r.right() as i32 - 3 * self.style.theme.size.control_h as i32);
            match &it {
                Item::App(id, n) if plus && self.is_launchable(id) => self.palette_run(Item::NewWindow(id.clone(), n.clone()), time),
                _ => self.palette_run(it, time),
            }
        }
    }

    /// Provede polozku a paletu zavre; zakazana akce se nespusti.
    pub(crate) fn palette_run(&mut self, it: Item, time: u64) {
        if self.item_view(&it).3.is_some() {
            return;
        }
        self.set_palette(None);
        let key = it.key(self);
        self.palette_recent.retain(|k| k != &key);
        self.palette_recent.insert(0, key.clone());
        self.palette_recent.truncate(RECENT);
        *self.palette_usage.entry(key).or_insert(0) += 1;
        match it {
            Item::App(id, _) => self.launch_or_activate(&id, time),
            Item::NewWindow(id, _) => self.launch_new(&id, time),
            Item::Window(g) => {
                if let Some(w) = self.windows.get_mut(&g) {
                    w.minimized = false;
                }
                self.damage_window(g);
                self.set_focus(Some(g));
                self.damage_panel();
            }
            Item::System(SysAction::Snapshot) => self.request_snapshot("paleta"),
            Item::System(SysAction::Fullscreen) => {
                if let Some(g) = self.focus {
                    self.toggle_fullscreen(g, time);
                }
            }
            // Nevratne: systemove potvrzeni (pravidlo 6).
            Item::System(SysAction::Shutdown) => self.confirm_power(false),
            Item::System(SysAction::Reboot) => self.confirm_power(true),
            Item::AppAction(g, i) => {
                let Some(w) = self.windows.get(&g) else { return };
                let (Some(cl), Some(f)) = (self.clients.get(&w.client), w.menu.get(i)) else { return };
                let msg = guiproto::control::CallerMsg::Call {
                    call: 0,
                    app: cl.info.id.clone(),
                    action: f.item.action.clone(),
                    args: f.item.args.clone(),
                    synthetic: false,
                };
                let (act, app) = (f.item.action.clone(), cl.info.id.clone());
                self.trace(time, format_args!("paleta: akce {act} aplikace {app}"));
                // Stejna cesta jako volani `actions`: overeni argumentu,
                // potvrzeni u `confirm`.
                self.caller_msg(PALETTE_PEER, "Paleta příkazů", false, msg, time);
            }
        }
    }

    /// Zadosti palety pro init (vypnout, restartovat) od posledniho volani.
    pub fn take_system_requests(&mut self) -> Vec<SystemRequest> {
        core::mem::take(&mut self.system_requests)
    }

    /// Kresli paletu (vrstva nad okny).
    pub(crate) fn draw_palette(&mut self, c: &mut Canvas<'_>) {
        let (Some(p), Some(r)) = (self.palette.clone(), self.palette_rect()) else { return };
        let t = self.style.theme.clone();
        let col = &t.color;
        let pop = t.shadow.popover.clone();
        self.style.draw_shadow(c, &pop, r, t.radius.card, Rect::default());
        let fw = if t.mode == theme::Mode::Contrast { 2 } else { 1 };
        c.round_fill_bordered(r, t.radius.card, col.surface, col.border_strong, fw);
        // Obsah (pole hledani a seznam se sekcemi) kresli knihovna ui.
        let _ = p;
        self.draw_sysui(c, r);
    }
}

/// Cetnost pouziti polozek (per uzivatel v pameti kompozitoru).
pub type Usage = BTreeMap<String, u32>;
