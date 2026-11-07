//! gui-term: ukazka okna terminalu v GUI (milnik 6c).
//!
//! Okno s vlastnim titulkem: aplikace kresli plochu titulku (kartu
//! relace) a deklaruje oblasti pro presun; odznak duvery a tlacitka okna
//! kresli system. Obsah je mrizka znaku `term::screen` (pismo Noto Sans
//! Mono) - stejna, jakou terminal 6b kresli do framebufferu.
//!
//! Napojeni na sluzbu relaci `termd` (nativni protokol `term::proto`:
//! prikazy, bloky vystupu, vyzvy) dela vlakno terminalu; tady je jen
//! mistni radkovy editor s nekolika prikazy, aby slo okno vyzkouset.
//!
//! Startovni zprava (kind USER): kanal okna ke kompozitoru.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gfx::text::Text;
use gfx::{Canvas, Image, Rect, rgb};
use guiproto::a11y::{Node, Role, actions};
use guiproto::action::Value;
use guiproto::window::{ClientMsg, InputEvent, Mode, RegionKind};
use inputproto::code::*;
use libos::{Channel, Error, Startup, println};
use term::screen::{self, Screen, Surface};
use ui::client::{Event, Window};

libos::entry!(main);

const TITLE_H: u32 = 32;
const PROMPT: &str = "> ";
/// Karta relace v titulku (vpravo od odznaku systemu).
const TAB: Rect = Rect::new(240, 4, 150, 28);

struct Term {
    win: Window,
    text: Text,
    screen: Screen,
    /// Vystup (radky) - pri zmene velikosti se prepise do nove mrizky.
    history: Vec<String>,
    line: String,
}

fn grid(w: u32, h: u32) -> (usize, usize) {
    screen::grid_for(w as usize, h.saturating_sub(TITLE_H) as usize)
}

impl Term {
    fn new(win: Window) -> Self {
        let (c, r) = grid(win.w, win.h);
        let mut t = Self {
            win,
            text: Text::system(),
            screen: Screen::new(c, r),
            history: vec![String::from("Terminál MagnumOpus (ukázka okna 6c). Napište help.")],
            line: String::new(),
        };
        t.refill();
        t
    }

    /// Prekresli mrizku z historie a rozepsaneho radku.
    fn refill(&mut self) {
        let (c, r) = grid(self.win.w, self.win.h);
        self.screen = Screen::new(c, r);
        let keep = self.history.len().saturating_sub(r.saturating_sub(1));
        for l in &self.history[keep..] {
            self.screen.write(l);
            self.screen.write("\n");
        }
        self.screen.write(PROMPT);
        self.screen.write(&self.line);
    }

    fn out(&mut self, s: &str) {
        for l in s.split('\n') {
            self.history.push(l.into());
        }
        if self.history.len() > 500 {
            self.history.drain(..100);
        }
    }

    fn run(&mut self, cmd: &str) {
        self.out(&alloc::format!("{PROMPT}{cmd}"));
        let (name, rest) = cmd.trim().split_once(' ').unwrap_or((cmd.trim(), ""));
        match name {
            "" => {}
            "help" => self.out("Příkazy: help, echo <text>, clear, ver.\nShell běží ve službě termd; okno se na ni napojí přes protokol term::proto."),
            "echo" => self.out(rest),
            "clear" => self.history.clear(),
            "ver" => self.out("MagnumOpus OS, milník 6c (GUI)"),
            _ => self.out(&alloc::format!("\x1b[31mneznámý příkaz:\x1b[39m {name}")),
        }
        self.refill();
    }

    fn a11y(&self) -> Vec<Node> {
        let mut win = Node::new(1, 0, Role::Window, "Terminál");
        win.bounds = Rect::new(0, 0, self.win.w, self.win.h);
        let mut bar = Node::new(2, 1, Role::Titlebar, "Terminál");
        bar.bounds = Rect::new(0, 0, self.win.w, TITLE_H);
        let mut tab = Node::new(3, 2, Role::Button, "Relace 1");
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
        let text = &mut self.text;
        self.win.draw(|c: &mut Canvas<'_>| {
            // Plochu titulku kresli aplikace (vlastni titulek).
            c.fill(Rect::new(0, 0, w, TITLE_H), rgb(0x22, 0x27, 0x30));
            c.rounded(TAB, 6, rgb(0x10, 0x18, 0x20));
            let (lh, base) = text.line(13.0);
            text.draw(c, TAB.x + 12, TAB.y + (TAB.h as i32 - lh as i32) / 2 + base, "Relace 1", 13.0, rgb(0xE6, 0xEA, 0xEE));
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

    fn key(&mut self, code: u16) -> bool {
        match code {
            KEY_ENTER | KEY_KPENTER => {
                let l = core::mem::take(&mut self.line);
                self.run(&l);
            }
            KEY_BACKSPACE => {
                self.line.pop();
                self.refill();
            }
            _ => return false,
        }
        true
    }
}

fn main(startup: Startup) -> i64 {
    let Some(h) = startup.handles.into_iter().next() else {
        println!("gui-term: chybi kanal okna");
        return 1;
    };
    let win = match Window::open(Channel::from_handle(h), Mode::CustomTitle, 720, 420, "Terminál") {
        Ok(w) => w,
        Err(e) => {
            println!("gui-term: okno nelze otevrit: {e}");
            return 2;
        }
    };
    let mut t = Term::new(win);
    if t.regions().and_then(|_| t.render()).is_err() {
        return 3;
    }
    loop {
        let Ok(ev) = t.win.next(None) else { return 0 };
        let redraw = match ev {
            Event::Input(r) => match r.event {
                InputEvent::Text(ch) => {
                    t.line.push(ch);
                    t.refill();
                    true
                }
                InputEvent::Key { code, action, .. } if action != 0 => t.key(code),
                _ => false,
            },
            Event::Resized { .. } => {
                t.refill();
                let _ = t.regions();
                true
            }
            Event::Close => return 0,
            Event::A11yAction { action, value, .. } if action & actions::SET_VALUE != 0 => {
                // Ctecka obrazovky nebo test "napise" radek a odesle ho.
                t.run(&value);
                true
            }
            Event::Action { call, id, args, .. } if id == "terminal.run" => {
                let cmd = args.iter().find_map(|(k, v)| match (k.as_str(), v) {
                    ("command", Value::Text(s)) => Some(s.clone()),
                    _ => None,
                });
                match cmd {
                    Some(c) => {
                        t.run(&c);
                        let _ = t.win.action_result(call, Ok(None));
                    }
                    None => {
                        let _ = t.win.action_result(call, Err("chybi command".into()));
                    }
                }
                true
            }
            Event::Action { call, .. } => {
                let _ = t.win.action_result(call, Err("neznama akce".into()));
                false
            }
            _ => false,
        };
        if redraw && t.render().is_err() {
            return 4;
        }
    }
}
