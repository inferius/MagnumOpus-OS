//! Paleta prikazu v1 (`design/prostredi-menu.html`): systemova zkratka
//! Ctrl+K nebo Super+K, hledani psanim. Kresli ji kompozitor nad vsemi
//! okny (aplikace ji neprekryje ani nezachyti), klavesnice i mys, Esc
//! zavre. Polozky: spusteni aplikaci, otevrena okna (prepnuti), systemove
//! akce; nedavne nahore. Akce aplikaci (menu pres API akci) prijdou
//! v dalsi verzi: misto pro ne je `Item::AppAction`.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::Canvas;
use guiproto::Rect;
use inputproto::code::{KEY_BACKSPACE, KEY_DOWN, KEY_ENTER, KEY_ESC, KEY_K, KEY_KPENTER, KEY_UP};
use inputproto::mods;

use crate::Comp;

/// Nejvic radku seznamu najednou.
pub const ROWS: usize = 8;
/// Kolik nedavnych polozek si paleta pamatuje.
const RECENT: usize = 8;

/// Systemova akce palety.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysAction {
    Shutdown,
    Reboot,
    Snapshot,
}

/// Polozka palety.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// Spustit (nebo aktivovat) aplikaci: id z manifestu, jmeno.
    App(String, String),
    /// Prepnout na otevrene okno.
    Window(u32),
    System(SysAction),
    /// Akce aplikace z API akci (v2): okno, id akce, popisek.
    #[allow(dead_code, reason = "v2: menu aplikaci pres API akci")]
    AppAction(u32, String, String),
}

impl Item {
    /// Klic pro seznam nedavnych (okno se pamatuje podle id).
    fn key(&self) -> String {
        match self {
            Item::App(id, _) => format!("app:{id}"),
            Item::Window(g) => format!("win:{g}"),
            Item::System(a) => format!("sys:{a:?}"),
            Item::AppAction(g, id, _) => format!("act:{g}:{id}"),
        }
    }
}

/// Otevrena paleta.
#[derive(Debug, Clone, Default)]
pub struct Palette {
    pub query: String,
    /// Vybrany radek (index ve vyfiltrovanem seznamu).
    pub sel: usize,
}

/// Zadost palety, kterou vyridi sluzba (init): vypnout, restartovat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemRequest {
    Shutdown,
    Reboot,
}

/// Mala pismena bez diakritiky (hledani "spr" najde "Správce").
fn fold(s: &str) -> String {
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
        self.palette = p;
        if let Some(r) = self.palette_visual() {
            self.damage(r);
        }
    }

    /// Popisek, ikona a skupina polozky.
    fn item_label(&self, it: &Item) -> (String, &'static str, &'static str) {
        match it {
            Item::App(id, name) => {
                let what = if self.app_window(id).is_some() { "Aplikace (běží)" } else { "Aplikace" };
                (name.clone(), crate::render::app_icon(id), what)
            }
            Item::Window(g) => {
                let w = self.windows.get(g);
                let title = w.map_or(String::new(), |w| w.title.clone());
                let icon = w.and_then(|w| self.clients.get(&w.client)).map_or("window", |c| crate::render::app_icon(&c.info.id));
                (title, icon, "Okno")
            }
            Item::System(SysAction::Shutdown) => (String::from("Vypnout"), "power", "Systém"),
            Item::System(SysAction::Reboot) => (String::from("Restartovat"), "restart", "Systém"),
            Item::System(SysAction::Snapshot) => (String::from("Diagnostický snímek"), "snapshot", "Systém"),
            Item::AppAction(_, _, label) => (label.clone(), "command", "Akce"),
        }
    }

    /// Vsechny polozky: nedavne, okna, aplikace, systemove akce.
    fn palette_all(&self) -> Vec<Item> {
        let mut all: Vec<Item> = Vec::new();
        for &g in self.z.iter().rev() {
            if self.windows.get(&g).is_some_and(|w| w.mapped) {
                all.push(Item::Window(g));
            }
        }
        for (id, name) in &self.launchable {
            all.push(Item::App(id.clone(), name.clone()));
        }
        all.extend([SysAction::Snapshot, SysAction::Reboot, SysAction::Shutdown].map(Item::System));
        // Nedavne nahore (v poradi pouziti), zbytek beze zmeny.
        let mut out: Vec<Item> = self.palette_recent.iter().filter_map(|k| all.iter().find(|i| &i.key() == k).cloned()).collect();
        for i in all {
            if !out.contains(&i) {
                out.push(i);
            }
        }
        out
    }

    /// Polozky odpovidajici hledani.
    pub fn palette_items(&self) -> Vec<Item> {
        let q = self.palette.as_ref().map(|p| fold(&p.query)).unwrap_or_default();
        self.palette_all()
            .into_iter()
            .filter(|i| {
                let (label, _, group) = self.item_label(i);
                q.is_empty() || fold(&label).contains(&q) || fold(group).contains(&q)
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
        let w = t.size.dialog_w.max(480).min(self.w.saturating_sub(2 * t.space[5]));
        let rows = self.palette_items().len().clamp(1, ROWS) as u32;
        let h = self.palette_input_h() + rows * self.palette_row_h() + 3 * t.space[3];
        Some(Rect::new((self.w as i32 - w as i32) / 2, (self.h / 6) as i32, w, h))
    }

    fn palette_input_h(&self) -> u32 {
        self.style.theme.size.control_h + self.style.theme.space[3]
    }

    fn palette_row_h(&self) -> u32 {
        self.style.theme.size.control_h
    }

    /// Radek seznamu na vysce `y` (obrazovka).
    fn palette_row_at(&self, y: i32) -> Option<usize> {
        let r = self.palette_rect()?;
        let top = r.y + (self.style.theme.space[3] + self.palette_input_h()) as i32;
        let i = (y - top).div_euclid(self.palette_row_h() as i32);
        (y >= top && (i as usize) < self.palette_items().len().min(ROWS)).then_some(i as usize)
    }

    /// Klavesa pri otevrene palete (vse ji konci, aplikaci nejde nic).
    pub(crate) fn palette_key(&mut self, code: u16, press: bool, time: u64) {
        if !press {
            return;
        }
        let n = self.palette_items().len();
        let Some(p) = self.palette.clone() else { return };
        let mut p2 = p.clone();
        match code {
            KEY_ESC => return self.set_palette(None),
            KEY_ENTER | KEY_KPENTER => {
                if let Some(it) = self.palette_items().get(p.sel).cloned() {
                    self.palette_run(it, time);
                }
                return;
            }
            KEY_DOWN if n > 0 => p2.sel = (p.sel + 1).min(n - 1),
            KEY_UP => p2.sel = p.sel.saturating_sub(1),
            KEY_BACKSPACE => {
                p2.query.pop();
                p2.sel = 0;
            }
            _ => return,
        }
        self.set_palette(Some(p2));
    }

    /// Psany znak do hledani.
    pub(crate) fn palette_text(&mut self, ch: char) {
        if ch.is_control() {
            return;
        }
        if let Some(mut p) = self.palette.clone() {
            p.query.push(ch);
            p.sel = 0;
            self.set_palette(Some(p));
        }
    }

    /// Klik pri otevrene palete: radek provede, mimo paletu zavre.
    pub(crate) fn palette_click(&mut self, x: i32, y: i32, time: u64) {
        let inside = self.palette_rect().is_some_and(|r| r.contains(x, y));
        if !inside {
            return self.set_palette(None);
        }
        if let Some(i) = self.palette_row_at(y)
            && let Some(it) = self.palette_items().get(i).cloned()
        {
            self.palette_run(it, time);
        }
    }

    /// Provede polozku a paletu zavre.
    fn palette_run(&mut self, it: Item, time: u64) {
        self.set_palette(None);
        let key = it.key();
        self.palette_recent.retain(|k| k != &key);
        self.palette_recent.insert(0, key);
        self.palette_recent.truncate(RECENT);
        self.trace(time, format_args!("paleta: {it:?}"));
        match it {
            Item::App(id, _) => self.launch_or_activate(&id, time),
            Item::Window(g) => {
                if let Some(w) = self.windows.get_mut(&g) {
                    w.minimized = false;
                }
                self.damage_window(g);
                self.set_focus(Some(g));
                self.damage_panel();
            }
            Item::System(SysAction::Snapshot) => self.request_snapshot("paleta"),
            Item::System(SysAction::Shutdown) => self.system_requests.push(SystemRequest::Shutdown),
            Item::System(SysAction::Reboot) => self.system_requests.push(SystemRequest::Reboot),
            Item::AppAction(..) => {}
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
        let pad = t.space[3] as i32;
        // Pole hledani.
        let input = Rect::new(r.x + pad, r.y + pad, r.w - 2 * pad as u32, t.size.control_h);
        c.round_fill_bordered(input, t.radius.control, col.surface_2, col.focus, 2);
        let is = t.size.icon_m;
        self.style.icons.draw(c, "search", input.x + t.space[3] as i32, input.y + (input.h - is) as i32 / 2, is, t.icon_stroke, col.ink_muted);
        let bs = t.text.body;
        let tx = input.x + (t.space[3] * 2 + is) as i32;
        let (text, ink) = if p.query.is_empty() { (String::from("Hledat aplikace, okna a akce..."), col.ink_faint) } else { (p.query.clone(), col.ink) };
        let face = self.style.face(bs);
        let base = input.y + face.baseline_in(bs.size, input.h);
        let end = face.draw(c, tx, base, &text, bs.size, ink);
        if !p.query.is_empty() {
            // Kurzor za textem.
            c.fill(Rect::new(end + 1, input.y + t.space[2] as i32, 2, input.h - 2 * t.space[2]), col.accent);
        }
        // Seznam.
        let items = self.palette_items();
        let mut y = r.y + pad + self.palette_input_h() as i32;
        let rh = self.palette_row_h();
        if items.is_empty() {
            let face = self.style.face(bs);
            let base = y + face.baseline_in(bs.size, rh);
            face.draw(c, input.x + t.space[3] as i32, base, "Nic nenalezeno", bs.size, col.ink_muted);
        }
        for (i, it) in items.iter().take(ROWS).enumerate() {
            let row = Rect::new(input.x, y, input.w, rh);
            if i == p.sel {
                c.round_fill(row, t.radius.control, col.accent_soft);
            }
            let (label, icon, group) = self.item_label(it);
            self.style.icons.draw(c, icon, row.x + t.space[3] as i32, row.y + (rh - is) as i32 / 2, is, t.icon_stroke, col.ink);
            let face = self.style.face(bs);
            let base = row.y + face.baseline_in(bs.size, rh);
            face.draw(c, row.x + (t.space[3] * 2 + is) as i32, base, &label, bs.size, col.ink);
            let gs = t.text.caption;
            let gw = self.style.width(group, gs) as i32;
            let face = self.style.face(gs);
            let base = row.y + face.baseline_in(gs.size, rh);
            face.draw(c, row.right() as i32 - gw - t.space[3] as i32, base, group, gs.size, col.ink_muted);
            y += rh as i32;
        }
    }
}
