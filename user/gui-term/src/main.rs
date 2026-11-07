//! gui-term: okno terminalu v GUI nad sluzbou relaci `termd`.
//!
//! Okno s vlastnim titulkem: aplikace kresli plochu titulku (kartu
//! relace) a deklaruje oblasti pro presun; odznak duvery a tlacitka okna
//! kresli system. Obsah je interaktivni terminal `term::ui` (editor se
//! zvyraznenim, bloky vystupu, tabulky, prubehy, vyzvy) kresleny do
//! mrizky `term::screen` - stejne jako zalozni konzole.
//!
//! Startovni zprava (kind USER): [kanal okna ke kompozitoru, kanal
//! pripojeni k termd od initu]. Na `connect` init spusti termd a vrati
//! kanal k nemu; kdyz termd skonci nebo spadne, okno pozada znovu a
//! pripoji se ke stejne relaci (relace bezi ve vlastnim procesu dal).
//! Bez kanalu pripojeni okno jen ukaze, ze terminal neni dostupny.

#![no_std]
#![no_main]

extern crate alloc;

mod keys;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use gfx::text::Text;
use gfx::{Canvas, Image, Rect, rgb};
use guiproto::a11y::{Node, Role, actions};
use guiproto::action::Value;
use guiproto::window::{ClientMsg, Mode, RegionKind};
use libos::{Channel, Error, Handle, Signals, Startup, println, time};
use term::proto::ServerMsg;
use term::screen::{self, Screen, Surface};
use term::theme::Theme;
use term::ui::App;
use ui::client::{Event, Window};

libos::entry!(main);

const TITLE_H: u32 = 32;
/// Karta relace v titulku (vpravo od odznaku systemu).
const TAB: Rect = Rect::new(240, 4, 150, 28);
/// Animace pruhu a tociciho znaku (jen kdyz neco bezi).
const TICK: Duration = Duration::from_millis(150);
/// Prodleva pred novym pripojenim po padu termd (pak dvojnasobna).
const RETRY: Duration = Duration::from_millis(300);
const RETRY_MAX: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

struct Term {
    win: Window,
    text: Text,
    screen: Screen,
    app: App,
    /// Kanal pripojeni od initu (`None` = terminal bez sluzby).
    conn: Option<Channel>,
    termd: Option<Channel>,
    /// Kdy zkusit znovu pripojit (po padu termd) a soucasna prodleva.
    retry_at: Option<u64>,
    retry: Duration,
}

fn grid(w: u32, h: u32) -> (usize, usize) {
    screen::grid_for(w as usize, h.saturating_sub(TITLE_H) as usize)
}

fn now() -> Option<i64> {
    time::wall().map(|t| t.secs.saturating_mul(1_000_000_000).saturating_add(t.nanos as i64))
}

/// Pozada init o termd; vrati kanal k nemu. Odpoved `err CANCELED` patri
/// drivejsimu pokusu (init ho nahradil novejsim) - cist dal.
fn connect(conn: &Channel) -> Result<Channel, Error> {
    // Zastarale odpovedi (pokus, ktery vyprsel) zahodit: jejich termd init
    // pri dalsim `connect` ukonci.
    while conn.try_read().is_ok() {}
    conn.write(b"connect", Vec::new())?;
    let end = time::deadline_after(CONNECT_TIMEOUT);
    loop {
        let m = conn.read(Some(end))?;
        if m.data.starts_with(b"ok") {
            return m.handles.into_iter().next().map(Channel::from_handle).ok_or(Error::Corrupted);
        }
        if m.data.starts_with(b"err CANCELED") {
            continue;
        }
        println!("gui-term: termd nejde spustit: {}", String::from_utf8_lossy(&m.data));
        return Err(Error::Unreachable);
    }
}

impl Term {
    fn new(win: Window, conn: Option<Channel>) -> Self {
        let (c, r) = grid(win.w, win.h);
        let mut t = Self {
            win,
            text: Text::system(),
            screen: Screen::new(c, r),
            app: App::new(Theme::dark(), c, r),
            conn,
            termd: None,
            retry_at: None,
            retry: RETRY,
        };
        t.app.welcome = true;
        t.reconnect();
        t
    }

    /// Pripoji se k termd (prvni start i po padu: stejna relace).
    fn reconnect(&mut self) {
        let Some(conn) = &self.conn else { return };
        match connect(conn) {
            Ok(ch) => {
                self.termd = Some(ch);
                self.retry_at = None;
                self.retry = RETRY;
                let session = self.app.session;
                self.app.start(session);
                self.send_out();
            }
            Err(Error::PeerClosed) => {
                println!("gui-term: init zavrel kanal pripojeni");
                self.conn = None;
            }
            Err(e) => {
                println!("gui-term: pripojeni k termd: {e}");
                self.retry_at = Some(time::deadline_after(self.retry));
                self.retry = (self.retry * 2).min(RETRY_MAX);
            }
        }
    }

    fn lost(&mut self) {
        self.termd = None;
        self.app.disconnected();
        if self.conn.is_some() {
            self.retry_at = Some(time::deadline_after(self.retry));
        }
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
        let (c, r) = grid(self.win.w, self.win.h);
        self.screen = Screen::new(c, r);
        self.app.resize(c, r);
    }

    fn status(&self) -> String {
        if self.conn.is_none() {
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
        bar.bounds = Rect::new(0, 0, self.win.w, TITLE_H);
        let mut tab = Node::new(3, 2, Role::Button, &self.status());
        tab.bounds = TAB;
        tab.actions = actions::ACTIVATE;
        let mut t = Node::new(4, 1, Role::Terminal, "Výstup");
        t.bounds = Rect::new(0, TITLE_H as i32, self.win.w, self.win.h.saturating_sub(TITLE_H));
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
        if self.conn.is_none() {
            self.screen.write("\r\n\x1b[31mTerminál nemá službu termd (spusťte ho přes init).\x1b[39m");
        } else if self.termd.is_none() {
            self.screen.write("\r\n\x1b[33mSlužba termd není připojená, připojuji znovu...\x1b[39m");
        }
        let (w, h) = (self.win.w, self.win.h);
        let gh = h.saturating_sub(TITLE_H);
        // Mrizka do pomocneho bufferu (term::screen kresli do linearniho fb).
        let mut grid = vec![0u32; (w * gh) as usize];
        let mut s = Surface {
            buf: &mut grid,
            width: w as usize,
            height: gh as usize,
            stride: w as usize,
            format: screen::PixelFormat::Bgr,
        };
        screen::paint(&self.screen, &mut s);
        for p in &mut grid {
            *p |= 0xFF00_0000;
        }
        let label = self.status();
        let text = &mut self.text;
        self.win.draw(|c: &mut Canvas<'_>| {
            // Plochu titulku kresli aplikace (vlastni titulek).
            c.fill(Rect::new(0, 0, w, TITLE_H), rgb(0x22, 0x27, 0x30));
            c.rounded(TAB, 6, rgb(0x10, 0x18, 0x20));
            let (lh, base) = text.line(13.0);
            text.draw(c, TAB.x + 12, TAB.y + (TAB.h as i32 - lh as i32) / 2 + base, &label, 13.0, rgb(0xE6, 0xEA, 0xEE));
            if let Some(img) = Image::new(&grid, w, gh, w) {
                c.draw_image(&img, Rect::new(0, 0, w, gh), Rect::new(0, TITLE_H as i32, w, gh), 255, true);
            }
        })?;
        self.win.set_a11y(&self.a11y())?;
        self.win.commit_redrawn(false)
    }

    /// Deklarovane oblasti: titulek tahne okno, karta ne.
    fn regions(&self) -> Result<(), Error> {
        let msg = ClientMsg::Regions {
            window: 1,
            regions: vec![(Rect::new(0, 0, self.win.w, TITLE_H), RegionKind::Drag), (TAB, RegionKind::NoDrag)],
        };
        self.win.channel().write(&msg.encode(), Vec::new())
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
    let mut hs = startup.handles.into_iter();
    let Some(h) = hs.next() else {
        println!("gui-term: chybi kanal okna");
        return 1;
    };
    let conn = hs.next().map(Channel::from_handle);
    if conn.is_none() {
        println!("gui-term: chybi kanal pripojeni k termd");
    }
    let win = match Window::open(Channel::from_handle(h), Mode::CustomTitle, 760, 460, "Terminál") {
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
            let deadline = [animate.then_some(next_tick), t.retry_at].into_iter().flatten().min();
            let rd = Signals::READABLE | Signals::PEER_CLOSED;
            let mut items: Vec<(&Handle, Signals)> = vec![(t.win.channel().handle(), rd)];
            if let Some(d) = &t.termd {
                items.push((d.handle(), rd));
            }
            let _ = libos::wait_many(&items, deadline);
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
                Err(_) => break,
            }
        }
        if t.retry_at.is_some_and(|at| time::now() >= at) {
            t.reconnect();
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
