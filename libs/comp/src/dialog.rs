//! Systemove dialogy (opravneni, potvrzeni akce, heslo) a bezpecna
//! obrazovka. Kresli je jen kompozitor ve vrstve nad ztlumenymi okny;
//! prijimaji jen fyzicky vstup (umely vstup se ignoruje), takze dialog
//! nepotvrdi automat ani aplikace s capability "vkladat vstup".

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::text::Text;
use gfx::{Canvas, rgb};
use guiproto::Rect;
use guiproto::control::{DialogKind, DialogReply, DialogRequest};
use inputproto::Action;
use inputproto::code::*;

use crate::render::{SMALL_PX, UI_PX, color};
use crate::{Comp, Out, PeerId};

/// Kdo dialog vyvolal (komu patri odpoved).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// Sluzba s capability `dialog`.
    Peer(PeerId),
    /// Potvrzeni volani akce (vnitrni id volani).
    Action(u32),
    /// Bezpecna obrazovka po bezpecne zkratce.
    Secure,
    /// Nove vstupni zarizeni ceka na overeni (id zdroje v `inputd`).
    Device(u16),
}

#[derive(Debug, Clone)]
pub struct Dialog {
    pub owner: Owner,
    pub id: u32,
    pub kind: DialogKind,
    pub app: String,
    pub text: String,
    pub options: Vec<String>,
    pub sel: usize,
    pub password: String,
}

/// Jak dlouho po zobrazeni dialog vstup ignoruje.
pub const INPUT_DELAY: u64 = 300_000_000;

/// Nejvic cekajicich dialogu (dalsi se odmitnou).
const MAX_QUEUE: usize = 32;
const MAX_PASSWORD: usize = 256;

#[derive(Default)]
pub struct Dialogs {
    queue: VecDeque<Dialog>,
    /// Od kdy je aktivni dialog videt (vstup tesne po zobrazeni se
    /// ignoruje - ochrana proti podstrceni kliku).
    pub(crate) since: u64,
    /// Bezpecnostni fraze uzivatele (aplikace ji nezna).
    phrase: String,
}

/// Rozvrzeni dialogu na obrazovce.
pub struct Layout {
    pub frame: Rect,
    pub buttons: Vec<Rect>,
    pub field: Option<Rect>,
    pub lines: Vec<String>,
}

impl Dialogs {
    pub fn active(&self) -> Option<&Dialog> {
        self.queue.front()
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn secure_active(&self) -> bool {
        self.queue.iter().any(|d| d.owner == Owner::Secure)
    }

    /// Odebere dialogy vlastniku podle `f`; `true` = zmizel aktivni.
    pub(crate) fn remove_where(&mut self, f: impl Fn(Owner) -> bool) -> bool {
        let front = self.active().is_some_and(|d| f(d.owner));
        self.queue.retain(|d| !f(d.owner));
        front
    }

    pub fn next_wakeup(&self) -> Option<u64> {
        None
    }

    /// Rozvrzeni aktivniho dialogu.
    pub fn layout(&self, text: &Text, sw: u32, sh: u32) -> Option<Layout> {
        let d = self.active()?;
        let w = 520u32.min(sw.saturating_sub(20));
        let lines = wrap(text, &d.text, UI_PX, w - 40);
        let (lh, _) = text.line(UI_PX);
        let mut h = 44 + 26 + lines.len() as u32 * (lh + 2) + 16 + 24 + 52;
        if d.kind == DialogKind::Password {
            h += 44;
        }
        let x = (sw as i32 - w as i32) / 2;
        let y = (sh as i32 - h as i32) / 2;
        let frame = Rect::new(x, y, w, h);
        let field = (d.kind == DialogKind::Password).then(|| Rect::new(x + 20, y + h as i32 - 52 - 24 - 44, w - 40, 32));
        let mut buttons = Vec::new();
        let mut bx = x + w as i32 - 20;
        for o in d.options.iter().rev() {
            let bw = (text.width(o, UI_PX) + 28).max(90);
            bx -= bw as i32;
            buttons.push(Rect::new(bx, y + h as i32 - 46, bw, 32));
            bx -= 10;
        }
        buttons.reverse();
        Some(Layout { frame, buttons, field, lines })
    }

    pub fn draw(&self, c: &mut Canvas<'_>, text: &mut Text, sw: u32, sh: u32) {
        let Some(l) = self.layout(text, sw, sh) else { return };
        let d = self.active().unwrap();
        let f = l.frame;
        c.rounded(Rect::new(f.x + 4, f.y + 6, f.w, f.h), 10, gfx::rgba(0, 0, 0, 120));
        c.rounded(f, 10, rgb(0x22, 0x28, 0x31));
        c.outline(f, 1, color::BADGE);
        // Systemovy pruh: stit, nazev a bezpecnostni fraze.
        let bar = Rect::new(f.x, f.y, f.w, 36);
        c.rounded(bar, 10, color::BADGE);
        c.fill(Rect::new(f.x, f.y + 26, f.w, 10), color::BADGE);
        let (lh, base) = text.line(UI_PX);
        let title = match (d.owner, d.kind) {
            (Owner::Secure, _) => "Bezpečná obrazovka systému",
            (_, DialogKind::Permission) => "Systém: oprávnění",
            (_, DialogKind::Confirm) => "Systém: potvrzení",
            (_, DialogKind::Password) => "Systém: heslo",
        };
        text.draw(c, f.x + 16, f.y + (36 - lh as i32) / 2 + base, title, UI_PX, color::BADGE_TEXT);
        let mut y = f.y + 44;
        let (slh, sbase) = text.line(SMALL_PX);
        if !d.app.is_empty() {
            text.draw(c, f.x + 20, y + sbase, &d.app, SMALL_PX, color::TITLE_TEXT_DIM);
        }
        y += 26;
        for line in &l.lines {
            text.draw(c, f.x + 20, y + base, line, UI_PX, color::TITLE_TEXT);
            y += lh as i32 + 2;
        }
        if let Some(fr) = l.field {
            c.rounded(fr, 6, rgb(0x14, 0x18, 0x1E));
            c.outline(fr, 1, color::BORDER_FOCUSED);
            let dots: String = d.password.chars().map(|_| '\u{2022}').collect();
            text.draw(c, fr.x + 10, fr.y + (32 - lh as i32) / 2 + base, &dots, UI_PX, color::TITLE_TEXT);
        }
        let phrase = if self.phrase.is_empty() { String::from("Fráze: (nenastavena)") } else { alloc::format!("Fráze: {}", self.phrase) };
        let py = f.y + f.h as i32 - 46 - 4 - slh as i32;
        text.draw(c, f.x + 20, py + sbase, &phrase, SMALL_PX, color::BADGE_TEXT);
        for (i, (b, o)) in l.buttons.iter().zip(&d.options).enumerate() {
            let sel = i == d.sel;
            c.rounded(*b, 6, if sel { color::PANEL_ACTIVE } else { color::BUTTON });
            if sel {
                c.outline(*b, 1, color::TITLE_TEXT);
            }
            let tw = text.width(o, UI_PX) as i32;
            text.draw(c, b.x + (b.w as i32 - tw) / 2, b.y + (32 - lh as i32) / 2 + base, o, UI_PX, color::TITLE_TEXT);
        }
    }
}

/// Zalomi text na slova do sirky `w` (slovo delsi nez radek zustane cele).
pub fn wrap(text: &Text, s: &str, px: f32, w: u32) -> Vec<String> {
    let mut out = Vec::new();
    for para in s.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let cand = if line.is_empty() { String::from(word) } else { alloc::format!("{line} {word}") };
            if !line.is_empty() && text.width(&cand, px) > w {
                out.push(core::mem::take(&mut line));
                line = String::from(word);
            } else {
                line = cand;
            }
        }
        out.push(line);
        if out.len() >= 12 {
            break;
        }
    }
    out
}

impl Comp {
    /// Zadost na kanalu `dialog`.
    pub fn dialog_request(&mut self, peer: PeerId, req: DialogRequest, now: u64) {
        self.input_now = now;
        match req {
            DialogRequest::Ask {
                id,
                kind,
                app,
                text,
                options,
            } => {
                if self.dialogs.queue.len() >= MAX_QUEUE {
                    self.out.push(Out::Dialog(peer, DialogReply::Cancelled { id }));
                    return;
                }
                self.open_dialog(Dialog {
                    owner: Owner::Peer(peer),
                    id,
                    kind,
                    app,
                    text,
                    options,
                    sel: 0,
                    password: String::new(),
                });
            }
            DialogRequest::Cancel { id } => {
                let before = self.dialogs.queue.len();
                let was_front = self.dialogs.active().is_some_and(|d| d.owner == Owner::Peer(peer) && d.id == id);
                self.dialogs.queue.retain(|d| !(d.owner == Owner::Peer(peer) && d.id == id));
                if self.dialogs.queue.len() != before {
                    self.out.push(Out::Dialog(peer, DialogReply::Cancelled { id }));
                    if was_front {
                        self.damage_all();
                    }
                }
            }
            DialogRequest::Phrase { text } => {
                self.dialogs.phrase = text.chars().take(64).collect();
            }
        }
    }

    pub(crate) fn open_dialog(&mut self, d: Dialog) {
        let first = self.dialogs.queue.is_empty();
        if first || d.owner == Owner::Secure {
            self.dialogs.since = self.input_now;
        }
        if d.owner == Owner::Secure {
            // Bezpecna obrazovka predbehne vse.
            self.dialogs.queue.push_front(d);
        } else {
            self.dialogs.queue.push_back(d);
        }
        if first || self.dialogs.active().is_some_and(|a| a.owner == Owner::Secure) {
            self.damage_all();
        }
    }

    /// Kanal `dialog` zavren: jeho dialogy zmizi.
    pub fn dialog_peer_gone(&mut self, peer: PeerId) {
        let front = self.dialogs.active().map(|d| d.owner);
        self.dialogs.queue.retain(|d| d.owner != Owner::Peer(peer));
        if front == Some(Owner::Peer(peer)) {
            self.damage_all();
        }
    }

    /// Bezpecna zkratka (Ctrl+Alt+Delete) od `inputd`.
    pub fn secure_attention(&mut self, now: u64) {
        self.input_now = now;
        if self.dialogs.secure_active() {
            return;
        }
        self.open_dialog(Dialog {
            owner: Owner::Secure,
            id: 0,
            kind: DialogKind::Confirm,
            app: String::new(),
            text: String::from("Systém převzal klávesnici a myš. Tuto obrazovku nemůže žádná aplikace napodobit ani zavřít."),
            options: alloc::vec![String::from("Zpět")],
            sel: 0,
            password: String::new(),
        });
    }

    fn answer(&mut self, choice: usize) {
        let Some(d) = self.dialogs.queue.pop_front() else { return };
        self.dialogs.since = self.input_now;
        self.damage_all();
        match d.owner {
            Owner::Peer(p) => self.out.push(Out::Dialog(
                p,
                DialogReply::Answer {
                    id: d.id,
                    choice: choice as u32,
                    text: d.password,
                },
            )),
            Owner::Action(call) => self.action_confirmed(call, choice == 0),
            Owner::Secure => self.out.push(Out::SecureRelease),
            Owner::Device(source) => {
                self.input.untrusted.retain(|&s| s != source);
                self.out.push(Out::Device { source, trust: choice == 0 });
            }
        }
    }

    /// Nova klavesnice nebo mys ceka na overeni (proti BadUSB): jeji
    /// udalosti se do rozhodnuti ignoruji, potvrdit ji musi stavajici vstup.
    pub(crate) fn new_device(&mut self, source: u16, state: inputproto::DeviceTrust) {
        use inputproto::DeviceTrust::*;
        match state {
            Pending => {
                if self.input.untrusted.contains(&source) {
                    return;
                }
                self.input.untrusted.push(source);
                self.open_dialog(Dialog {
                    owner: Owner::Device(source),
                    id: source as u32,
                    kind: DialogKind::Confirm,
                    app: String::new(),
                    text: alloc::format!(
                        "Bylo připojeno nové vstupní zařízení (zdroj {source}). Povolte ho jen tehdy, když jste ho připojili vy. Zařízení do té doby nic nepíše."
                    ),
                    options: alloc::vec![String::from("Povolit zařízení"), String::from("Odmítnout")],
                    sel: 1,
                    password: String::new(),
                });
            }
            Trusted | Rejected => {
                self.input.untrusted.retain(|&s| s != source);
                let front = self.dialogs.active().map(|d| d.owner) == Some(Owner::Device(source));
                self.dialogs.queue.retain(|d| d.owner != Owner::Device(source));
                if front {
                    self.damage_all();
                }
            }
        }
    }

    pub(crate) fn dialog_key(&mut self, code: u16, action: Action, _ch: Option<char>, _mods: u16, _now: u64) {
        if action == Action::Release || _now < self.dialogs.since + INPUT_DELAY {
            return;
        }
        let Some(d) = self.dialogs.queue.front_mut() else { return };
        let n = d.options.len().max(1);
        match code {
            KEY_ENTER | KEY_KPENTER => {
                let s = d.sel;
                return self.answer(s);
            }
            KEY_ESC => return self.answer(n - 1),
            KEY_TAB | KEY_RIGHT => d.sel = (d.sel + 1) % n,
            KEY_LEFT => d.sel = (d.sel + n - 1) % n,
            KEY_BACKSPACE => {
                d.password.pop();
            }
            _ => return,
        }
        self.damage_all();
    }

    pub(crate) fn dialog_text(&mut self, ch: char) {
        let Some(d) = self.dialogs.queue.front_mut() else { return };
        if d.kind == DialogKind::Password && d.password.chars().count() < MAX_PASSWORD {
            d.password.push(ch);
            self.damage_all();
        }
    }

    pub(crate) fn dialog_click(&mut self, x: i32, y: i32, now: u64) {
        if now < self.dialogs.since + INPUT_DELAY {
            return;
        }
        let Some(l) = self.dialogs.layout(&self.text, self.w, self.h) else { return };
        if let Some(i) = l.buttons.iter().position(|b| b.contains(x, y)) {
            self.answer(i);
        }
    }

    pub(crate) fn dialogs_timers(&mut self, _now: u64) {}

    /// Rozvrzeni aktivniho dialogu (testy, inspekce).
    pub fn dialog_layout(&self) -> Option<Layout> {
        self.dialogs.layout(&self.text, self.w, self.h)
    }
}
