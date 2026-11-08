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

#[path = "../../console/src/connect.rs"]
mod connect;
mod keys;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use gfx::text::Family;
use gfx::{Canvas, Rect};
use guiproto::a11y::{Node, Role, actions};
use guiproto::action::Value;
use guiproto::window::{ClientMsg, Mode, RegionKind};
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
}

impl Look {
    fn new() -> Self {
        let t = theme::Theme::builtin(ColorMode::Light);
        // Terminal bez ligatur (`font.mono_features` je vypina); zapnute
        // vlastnosti zadne.
        let mono = Family::new(font::mono(), &[]);
        Self { t, mono, ui: ui::Look::ui_fonts() }
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
        let mut th = if self.t.mode == ColorMode::Dark { Theme::dark() } else { Theme::light() };
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
const TICK: Duration = Duration::from_millis(150);

struct Term {
    win: Window,
    look: Look,
    screen: Screen,
    app: App,
    /// Pripojeni k termd pres init (`conn: None` = terminal bez sluzby).
    conn: connect::Connector,
    termd: Option<Channel>,
}

fn grid(look: &Look, w: u32, h: u32) -> (usize, usize) {
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
        let (c, r) = grid(&look, win.w, win.h);
        let th = look.term_theme();
        let mut t = Self {
            win,
            look,
            screen: Screen::new(c, r),
            app: App::new(th, c, r),
            conn: connect::Connector::new(conn, "gui-term"),
            termd: None,
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
        loop {
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
    }

    fn resize(&mut self) {
        let (c, r) = grid(&self.look, self.win.w, self.win.h);
        self.screen = Screen::new(c, r);
        self.app.resize(c, r);
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

    fn a11y(&self) -> Vec<Node> {
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
        for n in [&mut win, &mut bar, &mut tab, &mut t] {
            n.layer = ui::ROOT_LAYER;
        }
        vec![win, bar, tab, t]
    }

    fn render(&mut self) -> Result<(), Error> {
        self.screen.write(&self.app.frame());
        if self.conn.conn.is_none() {
            self.screen.write("\r\n\x1b[31mTerminál nemá službu termd (spusťte ho přes init).\x1b[39m");
        } else if self.termd.is_none() {
            self.screen.write("\r\n\x1b[33mSlužba termd není připojená, připojuji znovu...\x1b[39m");
        }
        let label = self.status();
        let screen = &self.screen;
        let look = &mut self.look;
        self.win.draw(|c: &mut Canvas<'_>| paint(c, look, screen, &label))?;
        self.win.set_a11y(&self.a11y())?;
        self.win.commit_redrawn(false)
    }

    /// Deklarovane oblasti: titulek tahne okno, karta ne. Jdou pres
    /// `send`, takze je po padu kompozitoru obnova posle znovu.
    fn regions(&mut self) -> Result<(), Error> {
        let msg = ClientMsg::Regions {
            window: 1,
            regions: vec![(Rect::new(0, 0, self.win.w, self.look.title_h()), RegionKind::Drag), (self.look.tab(), RegionKind::NoDrag)],
        };
        self.win.send(msg)
    }

    /// Udalost okna; `Some(true)` = prekreslit, `None` = konec aplikace.
    fn event(&mut self, ev: Event) -> Option<bool> {
        Some(match ev {
            Event::Input(r) => match keys::key(&r) {
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
    let win = match Window::open(link, Mode::CustomTitle, 760, 460, "Terminál") {
        Ok(w) => w,
        Err(e) => {
            println!("gui-term: okno nelze otevrit: {e}");
            return 2;
        }
    };
    let mut t = Term::new(win, conn);
    if t.regions().and_then(|_| t.render()).is_err() {
        return 3;
    }
    let mut next_tick = time::deadline_after(TICK);
    loop {
        // Cekat na okno, termd nebo termin (animace, nove pripojeni).
        if !t.win.has_queued() {
            let animate = t.app.busy() || t.app.prompt.is_some();
            let deadline = [animate.then_some(next_tick), t.conn.deadline()].into_iter().flatten().min();
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
fn paint(c: &mut Canvas<'_>, look: &mut Look, screen: &Screen, label: &str) {
    let t = look.t.clone();
    let col = &t.color;
    let (w, h) = (c.w, c.h);
    let th = look.title_h();
    // Titulek (rezim vlastni titulek: plocha `surface`, karta `surface_2`).
    c.fill(Rect::new(0, 0, w, th), col.surface);
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
    // Mrizka.
    let body = Rect::new(0, th as i32, w, h.saturating_sub(th));
    c.fill(body, col.term_bg);
    let (cw, ch) = look.cell();
    let pad = look.pad() as i32;
    let ms = t.text.mono;
    for row in 0..screen.rows {
        for colm in 0..screen.cols {
            let cell = screen.cell(colm, row);
            let (ox, oy) = (pad + (colm as u32 * cw) as i32, th as i32 + pad + (row as u32 * ch) as i32);
            if let Some(b) = cell.bg {
                c.fill(Rect::new(ox, oy, cw, ch), px(b));
            }
            if cell.ch == ' ' {
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
    // Kurzor (blok, barva akcentu, text pod nim zustane citelny).
    if screen.x < screen.cols && screen.y < screen.rows {
        let (ox, oy) = (pad + (screen.x as u32 * cw) as i32, th as i32 + pad + (screen.y as u32 * ch) as i32);
        c.blend(Rect::new(ox, oy + 2, 2, ch - 4), col.accent);
    }
}
