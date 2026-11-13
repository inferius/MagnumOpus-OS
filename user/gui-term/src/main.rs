//! gui-term: okno terminalu v GUI nad sluzbou relaci `termd`.
//!
//! Okno s vlastnim titulkem: aplikace kresli plochu titulku (kartu
//! relace) a deklaruje oblasti pro presun; odznak duvery a tlacitka okna
//! kresli system. Obsah je interaktivni terminal `term::ui` (editor se
//! zvyraznenim, bloky vystupu, tabulky, prubehy, vyzvy) v mrizce
//! `term::screen`; bunky kresli okno pismem JetBrains Mono a barvami
//! `term_*` z tokenu (`design/terminal.html`).
//!
//! Startovni zprava (kind USER): [kanal okna ke kompozitoru, kanal
//! pripojeni k termd od initu]. Na `connect` init spusti termd a vrati
//! kanal k nemu; kdyz termd skonci nebo spadne, okno pozada znovu a
//! pripoji se ke stejne relaci (relace bezi ve vlastnim procesu dal).
//! Bez kanalu pripojeni okno jen ukaze, ze terminal neni dostupny.

#![no_std]
#![no_main]

extern crate alloc;

mod blockact;
#[path = "../../console/src/connect.rs"]
mod connect;
mod keys;
mod motion;
mod panel;
mod rows;
mod tabs;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use gfx::text::Family;
use gfx::{Canvas, Rect};
use guiproto::a11y::{Node, Role, actions};
use guiproto::action::Value;
use guiproto::window::{ClientMsg, InputEvent, InputRecord, Mode, RegionKind};
use inputproto::code::BTN_LEFT;
use libos::{Channel, Error, Handle, Signals, Startup, println, time};
use term::proto::ServerMsg;
use term::screen::Screen;
use term::theme::{Rgb, Theme};
use theme::Mode as ColorMode;
use term::ui::App;
use ui::client::{Event, Link, Window};

libos::entry!(main);

/// Vzhled z tokenu (`design/tokens.toml`): titulek, mrizka, barvy.
struct Look {
    t: theme::Theme,
    mono: Family,
    ui: Family,
    /// Ikony z prototypu (`design/assets/mo.js`).
    icons: gfx::icon::Icons,
}

impl Look {
    fn new() -> Self {
        let t = theme::Theme::builtin(ColorMode::Light);
        // Terminal bez ligatur (`font.mono_features` je vypina); zapnute
        // vlastnosti zadne.
        let mono = Family::new(font::mono(), &[]);
        Self { t, mono, ui: ui::Look::ui_fonts(), icons: gfx::icon::Icons::new() }
    }

    fn title_h(&self) -> u32 {
        self.t.size.title_h
    }

    /// Okraj mrizky (`s4`).
    fn pad(&self) -> u32 {
        self.t.space[4]
    }

    /// Bunka mrizky: sirka znaku a vyska radku (`type.mono`).
    fn cell(&self) -> (u32, u32) {
        let s = self.t.text.mono;
        let f = self.mono.face_ref(s.weight);
        let w = f.font().advance(f.font().glyph_index('0'), s.size);
        ((w + 0.5) as u32, s.line)
    }

    /// Mrizka bunek v okne (pocatek pod titulkem s okrajem).
    fn grid(&self, cols: usize) -> rows::Grid {
        let (cw, ch) = self.cell();
        let pad = self.pad() as i32;
        rows::Grid { x: pad, y: self.title_h() as i32 + pad, cw, ch, cols }
    }

    /// Prepinac zapisu B / C v titulku vlevo od tlacitek okna (ta kresli
    /// system: okraj `s3`, tri tlacitka `title_btn` s mezerou `s1`).
    /// Vraci celou plochu a obe volby (sirka volby = cil kliknuti
    /// `hit_min`).
    fn notation(&self, w: u32) -> (Rect, [Rect; 2]) {
        let sp = &self.t.space;
        let z = &self.t.size;
        let sys = sp[3] + 3 * z.title_btn + 2 * sp[1];
        let h = z.control_h_compact;
        let pad = sp[1];
        let sw = 2 * z.hit_min + 2 * pad;
        let x = w as i32 - (sys + sp[4] + sw) as i32;
        let y = (self.title_h().saturating_sub(h) / 2) as i32;
        let b = |i: i32| Rect::new(x + pad as i32 + i * z.hit_min as i32, y + pad as i32, z.hit_min, h - 2 * pad);
        (Rect::new(x, y, sw, h), [b(0), b(1)])
    }

    /// Karta relace v titulku: za odznakem systemu (misto podle
    /// `design/terminal.html`).
    fn tab(&self) -> Rect {
        let sp = &self.t.space;
        let h = self.t.size.control_h_compact + sp[2];
        Rect::new((self.t.size.hit_min * 4) as i32, (self.title_h() - h) as i32, self.t.size.hit_min * 4, h)
    }

    /// Motiv `term::ui` s barvami `term_*` z tokenu.
    fn term_theme(&self) -> Theme {
        let c = &self.t.color;
        let rgb = |v: u32| {
            let (r, g, b, _) = theme::unpremul(v);
            Rgb(r as u8, g as u8, b as u8)
        };
        // Vysoky kontrast ma tmave pozadi: barvy syntaxe z tmaveho motivu.
        let mut th = if self.t.mode == ColorMode::Light { Theme::light() } else { Theme::dark() };
        th.ui.bg = rgb(c.term_bg);
        th.ui.fg = rgb(c.term_ink);
        th.ui.muted = rgb(c.term_dim);
        th.ui.accent = rgb(c.accent);
        th.ui.ok = rgb(c.term_green);
        th.ui.warn = rgb(c.term_yellow);
        th.ui.err = rgb(c.term_red);
        th.ui.select = rgb(c.accent_soft);
        th
    }
}
/// Animace pruhu a tociciho znaku (jen kdyz neco bezi).
/// Tik okna (pruh, odhad, delka hlasek).
const TICK: Duration = Duration::from_millis(150);
/// Snimek krouzku cinnosti (~16 snimku/s, jen kdyz neco bezi).
const ANIM: Duration = Duration::from_millis(60);

/// Vychozi velikost okna: s panelem uloh (1040 = terminal ~760 + panel
/// 280). Kompozitor ji orizne na obrazovku; omezeni na 85 % sirky
/// obrazovky potrebuje od kompozitoru zmenu velikosti po otevreni (zatim
/// neni).
const WIN_W: u32 = 1040;
const WIN_H: u32 = 520;

/// Id uzlu pristupnosti prvniho tlacitka vyzvy (dalsi po jedne).
const BUTTON_NODE: u32 = 100;
/// Id uzlu seznamu uloh v panelu (polozka k = +1+2k, jeji Zrusit +2+2k).
const PANEL_NODE: u32 = 1000;
/// Kolik zprav od termd okno zpracuje pred dalsim snimkem (prehrani
/// relace po prepnuti karty nezasekne okno).
const MSG_BATCH: usize = 64;

/// Id uzlu prvni akce bloku (dalsi po jedne: sbalit, spustit znovu, ...).
const ACT_NODE: u32 = 3000;

/// Id uzlu prvni karty relace (dalsi po jedne, posledni = nova relace).
const TAB_NODE: u32 = 20;
/// Id uzlu volby zapisu B (C je dalsi).
const NOTATION_NODE: u32 = 10;

struct Term {
    win: Window,
    look: Look,
    screen: Screen,
    app: App,
    /// Pripojeni k termd pres init (`conn: None` = terminal bez sluzby).
    conn: connect::Connector,
    termd: Option<Channel>,
    /// Radky posledniho snimku (role pro klik na prvky navrhu).
    lines: Vec<term::ui::draw::Line>,
    /// Prolnuti novych bloku.
    fade: motion::Fade,
    /// Od termd cekaji dalsi zpravy (davka byla plna).
    more: bool,
    /// Blok pod ukazatelem (akce bloku v hlavicce).
    hover: Option<u64>,
}

/// Mrizka v okne `w` x `h`; s panelem uloh je o jeho sirku uzsi.
fn grid(look: &Look, w: u32, h: u32, panel: bool) -> (usize, usize) {
    let w = if panel { w.saturating_sub(look.t.size.side_panel) } else { w };
    let (cw, ch) = look.cell();
    let p = 2 * look.pad();
    let cols = (w.saturating_sub(p) / cw.max(1)).max(1);
    let rows = (h.saturating_sub(look.title_h() + p) / ch.max(1)).max(1);
    (cols as usize, rows as usize)
}

fn now() -> Option<i64> {
    time::wall().map(|t| t.secs.saturating_mul(1_000_000_000).saturating_add(t.nanos as i64))
}


impl Term {
    fn new(win: Window, conn: Option<Channel>) -> Self {
        let look = Look::new();
        let (c, r) = grid(&look, win.w, win.h, panel::shown(win.screen.0, win.w));
        let th = look.term_theme();
        let mut app = App::new(th, c, r);
        // Krouzek a dilci bunky pruhu kresli okno samo (term::activity).
        app.glyphs = term::activity::Glyphs::Drawn;
        // Pred slozku ve vyzve okno kresli ikonu (rows.rs).
        app.icon_room = true;
        let mut t = Self {
            win,
            look,
            screen: Screen::new(c, r),
            app,
            conn: connect::Connector::new(conn, "gui-term"),
            termd: None,
            lines: Vec::new(),
            fade: motion::Fade::default(),
            more: false,
            hover: None,
        };
        t.app.welcome = true;
        t.conn.start();
        t
    }

    /// Odpoved initu na `connect`: pripojit se (prvni start i po padu
    /// termd: stejna relace). `true` = pripojeno.
    fn poll_connect(&mut self) -> bool {
        let Some(ch) = self.conn.poll() else { return false };
        self.termd = Some(ch);
        let session = self.app.session;
        self.app.start(session);
        self.send_out();
        true
    }

    fn lost(&mut self) {
        self.termd = None;
        self.app.disconnected();
        self.conn.lost();
    }

    fn send_out(&mut self) {
        let out = self.app.take_out();
        let Some(t) = &self.termd else { return };
        for m in out {
            if t.write(&m.encode(), Vec::new()).is_err() {
                self.lost();
                return;
            }
        }
    }

    /// Zpravy od termd; `true` = neco se zmenilo.
    fn termd_messages(&mut self) -> bool {
        let mut changed = false;
        self.more = false;
        for _ in 0..MSG_BATCH {
            let Some(t) = &self.termd else { return changed };
            match t.try_read() {
                Ok(m) => {
                    if let Ok(sm) = ServerMsg::decode(&m.data) {
                        self.app.on_msg(sm);
                        changed = true;
                    }
                }
                Err(Error::ShouldWait) => return changed,
                Err(_) => {
                    println!("gui-term: termd skoncil, pripojim se znovu");
                    self.lost();
                    return true;
                }
            }
        }
        // Davka plna (prehrani dlouhe historie): nejdriv nakreslit, zbytek
        // v dalsim pruchodu smyckou bez cekani.
        self.more = true;
        changed
    }

    fn resize(&mut self) {
        let (c, r) = grid(&self.look, self.win.w, self.win.h, self.panel());
        self.screen = Screen::new(c, r);
        self.app.resize(c, r);
    }

    /// Akce bloku (klik nebo pristupnost).
    fn block_act(&mut self, block: u64, act: blockact::Act) {
        match act {
            blockact::Act::Fold => self.app.toggle_block(block),
            blockact::Act::Again => self.app.rerun_block(block),
        }
        self.send_out();
    }

    /// Karty relaci v titulku (az k prepinaci zapisu).
    fn tabs(&self) -> Vec<tabs::Tab> {
        if self.app.session == 0 || self.termd.is_none() {
            return Vec::new();
        }
        let limit = self.look.notation(self.win.w).0.x - self.look.t.space[4] as i32;
        tabs::layout(&self.look, &self.app.session_tabs(), self.app.session, limit)
    }

    /// Plocha vsech karet (oblast bez presunu okna): od prvni karty az
    /// k prepinaci zapisu.
    fn tabs_area(&self) -> Rect {
        let first = self.look.tab();
        let end = self.look.notation(self.win.w).0.x;
        Rect::new(first.x, 0, (end - first.x).max(0) as u32, self.look.title_h())
    }

    /// Je videt panel uloh vpravo?
    fn panel(&self) -> bool {
        panel::shown(self.win.screen.0, self.win.w)
    }

    fn status(&self) -> String {
        if self.conn.conn.is_none() {
            String::from("Bez služby")
        } else if self.termd.is_none() {
            String::from("Připojuji...")
        } else if self.app.session == 0 {
            String::from("Relace ...")
        } else {
            alloc::format!("Relace {}", self.app.session)
        }
    }

    fn a11y(&mut self) -> Vec<Node> {
        let mut win = Node::new(1, 0, Role::Window, "Terminál");
        win.bounds = Rect::new(0, 0, self.win.w, self.win.h);
        let mut bar = Node::new(2, 1, Role::Titlebar, "Terminál");
        bar.bounds = Rect::new(0, 0, self.win.w, self.look.title_h());
        let mut tab = Node::new(3, 2, Role::Button, &self.status());
        tab.bounds = self.look.tab();
        tab.actions = actions::ACTIVATE;
        let mut t = Node::new(4, 1, Role::Terminal, "Výstup");
        let th = self.look.title_h();
        t.bounds = Rect::new(0, th as i32, self.win.w, self.win.h.saturating_sub(th));
        let mut v = String::new();
        for y in 0..self.screen.rows {
            let l = self.screen.line(y);
            if !l.is_empty() {
                v.push_str(&l);
                v.push('\n');
            }
        }
        t.value = v;
        t.state = guiproto::a11y::state::FOCUSED;
        t.actions = actions::SET_VALUE;
        let mut nodes = vec![win, bar, tab, t];
        // Akce bloku u vsech viditelnych hlavicek (Sbalit, Spustit znovu).
        let g = self.look.grid(self.screen.cols);
        for (k, b) in blockact::layout(&self.look, &self.lines, &g, |_| true).iter().enumerate() {
            let Some(blk) = self.app.blocks.iter().find(|x| x.id == b.block) else { continue };
            let name = match b.act {
                blockact::Act::Fold if blk.collapsed => alloc::format!("Rozbalit blok {}", blk.line),
                blockact::Act::Fold => alloc::format!("Sbalit blok {}", blk.line),
                blockact::Act::Again => alloc::format!("Spustit znovu {}", blk.line),
            };
            let mut n = Node::new(ACT_NODE + k as u32, 4, Role::Button, &name);
            n.bounds = b.rect;
            n.actions = actions::ACTIVATE;
            nodes.push(n);
        }
        // Karty relaci (aktualni SELECTED) a nova relace; ACTIVATE prepne.
        for (k, tb) in self.tabs().iter().enumerate() {
            let name = if tb.id == 0 {
                String::from("Nová relace")
            } else if tb.current {
                tabs::label(tb.id)
            } else {
                alloc::format!("{}, odpojená", tabs::label(tb.id))
            };
            let mut n = Node::new(TAB_NODE + k as u32, 2, Role::Button, &name);
            n.bounds = tb.rect;
            n.actions = actions::ACTIVATE;
            if tb.current {
                n.state = guiproto::a11y::state::SELECTED;
            }
            nodes.push(n);
        }
        // Panel uloh: seznam, uloha = polozka (jmeno a stav), Zrusit.
        if self.panel() {
            let area = panel::area(&self.look, self.win.w, self.win.h);
            let mut list = Node::new(PANEL_NODE, 1, Role::List, "Úlohy");
            list.bounds = area;
            nodes.push(list);
            for (k, card) in panel::cards(&self.look, &self.app.tasks, area).iter().enumerate() {
                let task = &self.app.tasks[card.task];
                let id = PANEL_NODE + 1 + 2 * k as u32;
                let name = alloc::format!("{}: {}", task.name, term::ui::tasks::state_text(&task.state));
                let mut item = Node::new(id, PANEL_NODE, Role::ListItem, &name);
                item.bounds = card.rect;
                nodes.push(item);
                if let Some(r) = card.cancel {
                    let mut b = Node::new(id + 1, id, Role::Button, &alloc::format!("Zrušit {}", task.name));
                    b.bounds = r;
                    b.actions = actions::ACTIVATE;
                    nodes.push(b);
                }
            }
        }
        // Prepinac zapisu (ACTIVATE vybere, zvoleny ma CHECKED).
        let (_, nb) = self.look.notation(self.win.w);
        for (i, (r, name)) in nb.iter().zip(["Zápis B (příkazový)", "Zápis C (výrazový)"]).enumerate() {
            let mut n = Node::new(NOTATION_NODE + i as u32, 2, Role::Button, name);
            n.bounds = *r;
            n.actions = actions::ACTIVATE;
            if self.app.notation_c == (i == 1) {
                n.state = guiproto::a11y::state::CHECKED;
            }
            nodes.push(n);
        }
        // Volby vyzvy jako tlacitka (cteckou i testem: ACTIVATE).
        let g = self.look.grid(self.screen.cols);
        for (b, label, ..) in rows::buttons(&mut self.look, &self.lines, &g) {
            let mut n = Node::new(BUTTON_NODE + b.index as u32, 4, Role::Button, &label);
            n.bounds = b.rect;
            n.actions = actions::ACTIVATE;
            nodes.push(n);
        }
        for n in &mut nodes {
            n.layer = ui::ROOT_LAYER;
        }
        nodes
    }

    fn render(&mut self) -> Result<(), Error> {
        // Text bez vlastni barvy (sloupce tabulky, vystup) barvou motivu.
        let ink = self.app.theme.ui.fg;
        self.screen.set_ink((ink.0, ink.1, ink.2));
        let (frame, lines) = self.app.frame_lines();
        self.screen.write(&frame);
        if self.conn.conn.is_none() {
            self.screen.write("\r\n\x1b[31mTerminál nemá službu termd (spusťte ho přes init).\x1b[39m");
        } else if let Some(why) = &self.conn.gave_up {
            // Opakovani by nepomohlo (pamet, limit restartu): chyba a volba.
            let text = alloc::format!("\r\n\x1b[31mSlužbu termd nejde spustit ({why}). Enter to zkusí znovu.\x1b[39m");
            self.screen.write(&text);
        } else if self.termd.is_none() {
            self.screen.write("\r\n\x1b[33mSlužba termd není připojená, připojuji znovu...\x1b[39m");
        }
        let label = self.status();
        let screen = &self.screen;
        let show_panel = self.panel();
        let tab_list = self.tabs();
        let collapsed: Vec<u64> = self.app.blocks.iter().filter(|b| b.collapsed).map(|b| b.id).collect();
        let hover = self.hover;
        let look = &mut self.look;
        let c_on = self.app.notation_c;
        let tasks = &self.app.tasks;
        let t_now = time::now();
        self.fade.update(&lines, t_now);
        let veils = self.fade.veils(&lines, t_now, &look.t.motion);
        self.win.draw(|c: &mut Canvas<'_>| {
            paint(c, look, screen, &label, &lines, &veils, &tab_list, (hover, &collapsed));
            if show_panel {
                let area = panel::area(look, c.w, c.h);
                panel::paint(c, look, tasks, area);
            }
            paint_notation(c, look, c_on);
        })?;
        self.lines = lines;
        let nodes = self.a11y();
        self.win.set_a11y(&nodes)?;
        self.win.commit_redrawn(false)
    }

    /// Deklarovane oblasti: titulek tahne okno, karta ne. Jdou pres
    /// `send`, takze je po padu kompozitoru obnova posle znovu.
    fn regions(&mut self) -> Result<(), Error> {
        let msg = ClientMsg::Regions {
            window: 1,
            regions: vec![
                (Rect::new(0, 0, self.win.w, self.look.title_h()), RegionKind::Drag),
                (self.look.tab(), RegionKind::NoDrag),
                (self.tabs_area(), RegionKind::NoDrag),
                (self.look.notation(self.win.w).0, RegionKind::NoDrag),
            ],
        };
        self.win.send(msg)
    }

    /// Udalost okna; `Some(true)` = prekreslit, `None` = konec aplikace.
    fn event(&mut self, ev: Event) -> Option<bool> {
        Some(match ev {
            // Ukazatel nad blokem: akce bloku v hlavicce.
            Event::Input(InputRecord { event: InputEvent::PointerMove { y, .. }, .. }) => {
                let g = self.look.grid(self.screen.cols);
                let row = if y >= g.y { ((y - g.y) as u32 / g.ch.max(1)) as usize } else { usize::MAX };
                let hb = self.lines.get(row).and_then(|l| blockact::block_of(&l.role));
                let changed = hb != self.hover;
                self.hover = hb;
                changed
            }
            Event::Input(InputRecord { event: InputEvent::PointerLeave, .. }) => self.hover.take().is_some(),
            // Klik na tlacitko vyzvy (prvky navrhu nad mrizkou).
            Event::Input(InputRecord { event: InputEvent::Button { code: BTN_LEFT, pressed: true, x, y }, .. }) => {
                // Zrusit ulohu v panelu.
                if self.panel() {
                    let area = panel::area(&self.look, self.win.w, self.win.h);
                    let hit = panel::cards(&self.look, &self.app.tasks, area)
                        .into_iter()
                        .find(|c| c.cancel.is_some_and(|r| r.contains(x, y)))
                        .map(|c| self.app.tasks[c.task].id);
                    if let Some(id) = hit {
                        self.app.task_action(id, 0);
                        self.send_out();
                        return Some(true);
                    }
                }
                // Akce bloku pod ukazatelem.
                if let Some(hb) = self.hover {
                    let g = self.look.grid(self.screen.cols);
                    let hit = blockact::layout(&self.look, &self.lines, &g, |b| b == hb).into_iter().find(|b| b.rect.contains(x, y));
                    if let Some(b) = hit {
                        self.block_act(b.block, b.act);
                        return Some(true);
                    }
                }
                // Karta relace nebo nova relace (+).
                if let Some(tab) = self.tabs().into_iter().find(|t| t.rect.contains(x, y)) {
                    self.app.switch_session(tab.id);
                    self.send_out();
                    return Some(true);
                }
                // Prepinac zapisu B / C v titulku.
                let (_, nb) = self.look.notation(self.win.w);
                if let Some(i) = nb.iter().position(|r| r.contains(x, y)) {
                    self.app.set_notation(i == 1);
                    self.send_out();
                    return Some(true);
                }
                let g = self.look.grid(self.screen.cols);
                let hit = rows::buttons(&mut self.look, &self.lines, &g).into_iter().find(|(b, ..)| b.rect.contains(x, y));
                match hit {
                    Some((b, ..)) => {
                        self.app.prompt_pick(b.index);
                        true
                    }
                    None => false,
                }
            }
            Event::Input(r) => match keys::key(&r) {
                // termd nejde spustit (pamet): Enter to zkusi znovu (rucne).
                Some(term::ui::Key::Enter) if self.conn.gave_up.is_some() && self.termd.is_none() => {
                    self.conn.start();
                    true
                }
                Some(k) => {
                    self.app.key(k);
                    true
                }
                None => false,
            },
            Event::Resized { .. } => {
                self.resize();
                let _ = self.regions();
                true
            }
            Event::Close => return None,
            // Kompozitor spadl a okno je obnovene: nakreslit znovu.
            Event::Restored { .. } => true,
            Event::Theme(m) => {
                self.look.t = theme::Theme::builtin(m);
                self.app.theme = self.look.term_theme();
                true
            }
            Event::A11yAction { node, action, .. } if action & actions::ACTIVATE != 0 && node >= ACT_NODE => {
                let g = self.look.grid(self.screen.cols);
                let b = blockact::layout(&self.look, &self.lines, &g, |_| true).into_iter().nth((node - ACT_NODE) as usize);
                if let Some(b) = b {
                    self.block_act(b.block, b.act);
                }
                true
            }
            Event::A11yAction { node, action, .. } if action & actions::ACTIVATE != 0 && (TAB_NODE..TAB_NODE + 50).contains(&node) => {
                if let Some(tb) = self.tabs().get((node - TAB_NODE) as usize) {
                    self.app.switch_session(tb.id);
                    self.send_out();
                }
                true
            }
            Event::A11yAction { node, action, .. } if action & actions::ACTIVATE != 0 && (NOTATION_NODE..NOTATION_NODE + 2).contains(&node) => {
                self.app.set_notation(node == NOTATION_NODE + 1);
                self.send_out();
                true
            }
            // Zrusit ulohu v panelu (uzel tlacitka = polozka + 1).
            Event::A11yAction { node, action, .. } if action & actions::ACTIVATE != 0 && (PANEL_NODE + 1..ACT_NODE).contains(&node) && (node - PANEL_NODE) % 2 == 0 => {
                let k = ((node - PANEL_NODE - 2) / 2) as usize;
                let area = panel::area(&self.look, self.win.w, self.win.h);
                if let Some(c) = panel::cards(&self.look, &self.app.tasks, area).get(k) {
                    let id = self.app.tasks[c.task].id;
                    self.app.task_action(id, 0);
                    self.send_out();
                }
                true
            }
            Event::A11yAction { node, action, .. } if action & actions::ACTIVATE != 0 && (BUTTON_NODE..PANEL_NODE).contains(&node) => {
                self.app.prompt_pick((node - BUTTON_NODE) as usize);
                true
            }
            Event::A11yAction { action, value, .. } if action & actions::SET_VALUE != 0 => {
                // Ctecka obrazovky nebo test "napise" radek a odesle ho.
                self.app.run_line(&value);
                true
            }
            Event::Action { call, id, args, .. } if id == "terminal.run" => {
                let cmd = args.iter().find_map(|(k, v)| match (k.as_str(), v) {
                    ("command", Value::Text(s)) => Some(s.clone()),
                    _ => None,
                });
                match cmd {
                    Some(c) => {
                        self.app.run_line(&c);
                        let _ = self.win.action_result(call, Ok(None));
                    }
                    None => {
                        let _ = self.win.action_result(call, Err("chybi command".into()));
                    }
                }
                true
            }
            Event::Action { call, .. } => {
                let _ = self.win.action_result(call, Err("neznama akce".into()));
                false
            }
            _ => false,
        })
    }
}

fn main(startup: Startup) -> i64 {
    // Handly: 0 = okno, 1 = listek (obnova po padu kompozitoru), 2 = kanal
    // pripojeni k termd.
    let mut hs = startup.handles.into_iter();
    let Some(link) = Link::from_handles(&mut hs) else {
        println!("gui-term: chybi kanal okna");
        return 1;
    };
    let conn = hs.next().map(Channel::from_handle);
    if conn.is_none() {
        println!("gui-term: chybi kanal pripojeni k termd");
    }
    let win = match Window::open(link, Mode::CustomTitle, WIN_W, WIN_H, "Terminál") {
        Ok(w) => w,
        Err(e) => {
            println!("gui-term: okno nelze otevrit: {e}");
            return 2;
        }
    };
    let mut t = Term::new(win, conn);
    if let Err(e) = t.regions().and_then(|_| t.render()) {
        println!("gui-term: prvni vykresleni selhalo: {e}");
        return 3;
    }
    let mut next_tick = time::deadline_after(TICK);
    let mut next_anim = time::deadline_after(ANIM);
    loop {
        // Cekat na okno, termd nebo termin (animace, nove pripojeni).
        if !t.win.has_queued() && !t.more {
            let fading = t.fade.active(time::now(), &t.look.t.motion);
            let animate = t.app.busy() || t.app.prompt.is_some();
            let spin = t.app.busy() && t.app.glyphs == term::activity::Glyphs::Drawn;
            let deadline =
                [animate.then_some(next_tick), (spin || fading).then_some(next_anim), t.conn.deadline()].into_iter().flatten().min();
            let rd = Signals::READABLE | Signals::PEER_CLOSED;
            let termd = &t.termd;
            let conn = &t.conn;
            // Kanal okna se po obnove kompozitoru meni: jen v ramci volani.
            let _ = t.win.with_channel(|ch| {
                let mut items: Vec<(&Handle, Signals)> = vec![(ch.handle(), rd)];
                if let Some(d) = termd {
                    items.push((d.handle(), rd));
                }
                if let Some(h) = conn.handle() {
                    items.push((h, rd));
                }
                libos::wait_many(&items, deadline)
            });
        }
        let mut redraw = t.termd_messages();
        // Udalosti okna (bez cekani).
        loop {
            match t.win.next(Some(time::now())) {
                Ok(ev) => match t.event(ev) {
                    Some(r) => redraw |= r,
                    None => return 0,
                },
                Err(Error::PeerClosed) => return 0,
                // Obnova kompozitoru uprostred: okno je zpet, nakreslit.
                Err(Error::ServiceRestarted) => {
                    let _ = t.regions();
                    redraw = true;
                }
                Err(_) => break,
            }
        }
        if t.conn.waiting() && t.poll_connect() {
            redraw = true;
        }
        t.send_out();
        if time::now() >= next_tick {
            next_tick = time::deadline_after(TICK);
            t.app.tick(now());
            if t.app.busy() {
                redraw = true;
            }
        }
        // Plynuly krouzek a prolnuti bloku: jen cas pro animaci, bez tiku
        // okna.
        let fading = t.fade.active(time::now(), &t.look.t.motion);
        if (t.app.busy() || fading) && time::now() >= next_anim {
            // Prolnuti v taktu snimku (`motion.frame_ms`), krouzek pomaleji.
            let step = if fading { Duration::from_micros((t.look.t.motion.frame_ms * 1000.0) as u64) } else { ANIM };
            next_anim = time::deadline_after(step);
            t.app.now = now();
            redraw = true;
        }
        if redraw && t.render().is_err() {
            return 4;
        }
    }
}

/// Premultiplied barva z RGB bunky.
fn px((r, g, b): (u8, u8, u8)) -> u32 {
    0xFF00_0000 | (r as u32) << 16 | (g as u32) << 8 | b as u32
}

/// Okno terminalu: titulek s kartou relace (odznak a tlacitka okna kresli
/// system pres nej) a mrizka bunek pismem JetBrains Mono.
#[allow(clippy::too_many_arguments)]
fn paint(c: &mut Canvas<'_>, look: &mut Look, screen: &Screen, label: &str, lines: &[term::ui::draw::Line], veils: &[u32], tabs: &[tabs::Tab], hover: (Option<u64>, &[u64])) {
    let t = look.t.clone();
    let col = &t.color;
    let (w, h) = (c.w, c.h);
    let th = look.title_h();
    // Titulek (rezim vlastni titulek: plocha `surface`, karta `surface_2`).
    c.fill(Rect::new(0, 0, w, th), col.surface);
    if tabs.iter().any(|t| t.id != 0) {
        tabs::paint(c, look, tabs);
    } else {
        paint_status_tab(c, look, label);
    }
    // Mrizka.
    let body = Rect::new(0, th as i32, w, h.saturating_sub(th));
    c.fill(body, col.term_bg);
    let (cw, ch) = look.cell();
    let pad = look.pad() as i32;
    paint_grid(c, look, screen, lines, veils, cw, ch, pad, th, w, hover);
}

/// Karta se stavem pripojeni (jeste neni relace: "Pripojuji...").
fn paint_status_tab(c: &mut Canvas<'_>, look: &mut Look, label: &str) {
    let t = look.t.clone();
    let col = &t.color;
    let tab = look.tab();
    let rad = t.radius.control;
    let old = c.clip();
    c.set_clip(old.intersect(&tab));
    c.round_fill(Rect::new(tab.x, tab.y, tab.w, tab.h + rad), rad, col.surface_2);
    c.set_clip(old);
    let s = t.text.small;
    let strong = theme::TextStyle { weight: t.text.body_strong.weight, ..s };
    let is = t.size.icon_s;
    let f = look.ui.face(strong.weight);
    let tw = f.width(label, strong.size).min(tab.w.saturating_sub(2 * t.space[4] + is + t.space[2]));
    let x = tab.x + t.space[4] as i32;
    let base = tab.y + f.baseline_in(strong.size, tab.h);
    f.draw_ellipsized(c, x, base, label, strong.size, col.ink, tw);
}

/// Mrizka bunek s prvky navrhu, prolnutim novych bloku a kurzorem.
#[allow(clippy::too_many_arguments)]
fn paint_grid(c: &mut Canvas<'_>, look: &mut Look, screen: &Screen, lines: &[term::ui::draw::Line], veils: &[u32], cw: u32, ch: u32, pad: i32, th: u32, w: u32, (hover, collapsed): (Option<u64>, &[u64])) {
    let t = look.t.clone();
    let col = &t.color;
    let ms = t.text.mono;
    // Prvky navrhu podle roli radku: pod textem, preskocene bunky, nad textem.
    let grid = look.grid(screen.cols);
    let skips = rows::skips(lines);
    rows::under(c, look, lines, &grid);
    for row in 0..screen.rows {
        let skip = skips.get(row).map(|v| v.as_slice()).unwrap_or(&[]);
        for colm in 0..screen.cols {
            if skip.iter().any(|&(a, b)| colm >= a && colm < b) {
                continue;
            }
            let cell = screen.cell(colm, row);
            let (ox, oy) = (pad + (colm as u32 * cw) as i32, th as i32 + pad + (row as u32 * ch) as i32);
            if let Some(b) = cell.bg {
                c.fill(Rect::new(ox, oy, cw, ch), px(b));
            }
            if cell.ch == ' ' {
                continue;
            }
            // Vlastni prvky (ukazatel cinnosti, dilci bunka pruhu).
            if let Some(d) = term::activity::drawn(cell.ch) {
                draw_activity(c, Rect::new(ox, oy, cw, ch), d, px(cell.fg));
                continue;
            }
            let weight = if cell.bold { t.text.body_strong.weight } else { ms.weight };
            let f = look.mono.face(weight);
            let base = oy + f.baseline_in(ms.size, ch);
            let mut pen = gfx::text::Pen::at(ox);
            f.draw_char(c, &mut pen, base, cell.ch, ms.size, px(cell.fg));
            if cell.underline {
                c.fill(Rect::new(ox, oy + ch as i32 - 3, cw, 1), px(cell.fg));
            }
        }
    }
    rows::over(c, look, lines, &grid, hover);
    if let Some(hb) = hover {
        let buttons = blockact::layout(look, lines, &grid, |b| b == hb);
        blockact::paint(c, look, &buttons, collapsed);
    }
    // Prolnuti noveho bloku: prekryv barvou pozadi (i pres levy pruh).
    for (row, &a) in veils.iter().enumerate().take(screen.rows) {
        if a > 0 {
            let y = th as i32 + pad + (row as u32 * ch) as i32;
            c.blend(Rect::new(0, y, w, ch), alpha(col.term_bg, a));
        }
    }
    // Kurzor (blok, barva akcentu, text pod nim zustane citelny).
    if screen.x < screen.cols && screen.y < screen.rows {
        let (ox, oy) = (pad + (screen.x as u32 * cw) as i32, th as i32 + pad + (screen.y as u32 * ch) as i32);
        c.blend(Rect::new(ox, oy + 2, 2, ch - 4), col.accent);
    }
}

/// Prepinac zapisu B / C v titulku (`.seg` z navrhu): plocha `surface_2`,
/// zvolena volba `surface` s okrajem, text tucne.
fn paint_notation(c: &mut Canvas<'_>, look: &mut Look, c_on: bool) {
    let t = look.t.clone();
    let col = &t.color;
    let (all, opts) = look.notation(c.w);
    let rad = t.radius.control;
    c.round_fill(all, rad, col.surface_2);
    let s = t.text.small;
    let f = look.ui.face(t.text.body_strong.weight);
    for (i, (r, label)) in opts.iter().zip(["B", "C"]).enumerate() {
        let on = c_on == (i == 1);
        if on {
            c.round_fill_bordered(*r, rad.saturating_sub(2), col.surface, col.border, 1);
        }
        let tw = f.width(label, s.size);
        let base = r.y + f.baseline_in(s.size, r.h);
        f.draw(c, r.x + (r.w.saturating_sub(tw) / 2) as i32, base, label, s.size, if on { col.ink } else { col.ink_muted });
    }
}

/// Barva (premultiplied) s kryti `a` / 255.
pub(crate) fn alpha(c: u32, a: u32) -> u32 {
    let ch = |s: u32| ((c >> s & 0xFF) * a / 255) << s;
    ch(24) | ch(16) | ch(8) | ch(0)
}

/// Krouzek cinnosti nebo dilci bunka pruhu: vlastni kresleni misto znaku
/// (krytí pixelu ze spolecneho [`term::activity::coverage`]).
fn draw_activity(c: &mut Canvas<'_>, cell: Rect, d: term::activity::Drawn, color: u32) {
    for y in 0..cell.h {
        for x in 0..cell.w {
            let a = term::activity::coverage(d, cell.w, cell.h, x, y) as u32;
            if a > 0 {
                c.blend(Rect::new(cell.x + x as i32, cell.y + y as i32, 1, 1), alpha(color, a));
            }
        }
    }
}
