//! Prihlasovaci a zamykaci obrazovka (`docs/cs/ucty.md`, navrh
//! `design/prihlaseni.html`).
//!
//! Kresli ji jen kompozitor pres celou obrazovku (nepruhledne, okna pod ni
//! nejsou videt) a bere vsechen fyzicky vstup; umely vstup ignoruje.
//! Overeni dela sluzba uctu: obrazovka jen vyda [`Request`] (sluzba
//! `compd` ho posle po kanalu `login` asynchronne, snimek neceka) a dostane
//! [`Reply`]. Jestli je nekdo prihlaseny nebo relace zamcena, rika
//! kompozitoru init (`session ...`), ne obrazovka sama.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::Canvas;
use guiproto::Rect;
use inputproto::Action;
use inputproto::code::*;
use theme::TextStyle;

use crate::Comp;
use crate::style::Style;

/// Ucet ve vyberu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub uid: u64,
    pub name: String,
    pub display: String,
    /// Bity metod (heslo 1, PIN 2, klic 4, vyvoj 8, obnova 16).
    pub methods: u8,
    pub flags: u32,
}

/// Metoda overeni na obrazovce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Password,
    Pin,
    Key,
    Recovery,
}

impl Method {
    pub const fn bit(self) -> u8 {
        match self {
            Method::Password => 1,
            Method::Pin => 2,
            Method::Key => 4,
            Method::Recovery => 16,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Method::Password => "Heslo",
            Method::Pin => "PIN",
            Method::Key => "Bezpečnostní klíč",
            Method::Recovery => "Zapomenuté heslo",
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            Method::Password => "Heslo",
            Method::Pin => "PIN",
            Method::Key => "PIN klíče (když ho má)",
            Method::Recovery => "Obnovovací klíč",
        }
    }
}

const METHODS: [Method; 4] = [Method::Password, Method::Pin, Method::Key, Method::Recovery];

/// Pozadavek na sluzbu uctu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    List,
    Phrase(u64),
    Auth { uid: u64, method: Method, secret: String },
    Unlock { session: u64, method: Method, secret: String },
    Lock { session: u64 },
}

/// Odpoved sluzby uctu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Accounts(Vec<Account>),
    Phrase(u64, String),
    Ok,
    Denied { retry_after_ms: u64, pin_left: u8 },
    LockedOut { retry_after_ms: u64 },
    Error(String),
}

/// Relace od initu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: u64,
    pub uid: u64,
    pub name: String,
    pub locked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Nekdo je prihlaseny a relace odemcena: obrazovka neni.
    Off,
    /// Prihlaseni (vyber uctu).
    Login,
    /// Zamcena relace: odemknout jen stejny ucet.
    Locked(u64),
}

/// Nejdelsi pole (znaky).
const MAX_FIELD: usize = 256;

#[derive(Default)]
pub struct Greeter {
    /// Kanal `login` od initu je pripojeny (bez nej se obrazovka neukaze).
    pub(crate) enabled: bool,
    pub(crate) accounts: Vec<Account>,
    pub(crate) sessions: Vec<Session>,
    pub(crate) sel: usize,
    pub(crate) method: Option<Method>,
    pub(crate) field: String,
    pub(crate) phrase: Option<(u64, String)>,
    /// Zprava pod polem (text, chyba).
    pub(crate) msg: Option<(String, bool)>,
    /// Ceka se na odpoved overeni.
    pub(crate) busy: bool,
    /// Do kdy se dalsi pokus neposle (ns, zpomaleni od sluzby).
    pub(crate) wait_until: u64,
    pub(crate) requests: Vec<Request>,
    /// Od kdy je obrazovka videt (vstup tesne po zobrazeni se ignoruje).
    pub(crate) since: u64,
}

pub(crate) fn zero(s: &mut String) {
    // Bajty hesla prepsat drive, nez se pamet uvolni.
    let mut v = core::mem::take(s).into_bytes();
    for b in v.iter_mut() {
        // SAFETY: `b` je platny jedinecny odkaz na bajt; volatile zapis
        // prekladac nevynecha.
        unsafe { core::ptr::write_volatile(b, 0) };
    }
    drop(v);
}

impl Greeter {
    pub fn mode(&self) -> Mode {
        if self.sessions.iter().any(|s| !s.locked) {
            return Mode::Off;
        }
        if let Some(s) = self.sessions.iter().rev().find(|s| s.locked) {
            return Mode::Locked(s.id);
        }
        if self.enabled { Mode::Login } else { Mode::Off }
    }

    pub fn visible(&self) -> bool {
        self.mode() != Mode::Off
    }

    /// Ucet, ke kteremu se obrazovka hlasi.
    fn target(&self) -> Option<&Account> {
        match self.mode() {
            Mode::Locked(id) => {
                let uid = self.sessions.iter().find(|s| s.id == id)?.uid;
                self.accounts.iter().find(|a| a.uid == uid)
            }
            _ => self.accounts.get(self.sel),
        }
    }

    /// Metody, ktere vybrany ucet ma (vzdy aspon heslo).
    fn methods(&self) -> Vec<Method> {
        let m = self.target().map_or(1, |a| a.methods);
        METHODS.iter().copied().filter(|x| m & x.bit() != 0 || *x == Method::Password).collect()
    }

    fn current_method(&self) -> Method {
        let ms = self.methods();
        match self.method {
            Some(m) if ms.contains(&m) => m,
            // Vychozi: PIN, ma-li ho ucet (rychlejsi), jinak heslo.
            _ if ms.contains(&Method::Pin) => Method::Pin,
            _ => Method::Password,
        }
    }

    fn clear(&mut self) {
        zero(&mut self.field);
        self.msg = None;
    }
}

/// Rozvrzeni obrazovky.
pub struct Layout {
    pub avatar: Rect,
    pub users: Vec<(usize, Rect)>,
    pub name_y: i32,
    pub field: Rect,
    pub go: Rect,
    pub phrase: Rect,
    pub msg_y: i32,
    pub methods: Vec<(Method, Rect)>,
    pub clock_y: i32,
}

impl Comp {
    /// Kanal `login` od initu pripojeny (`true`) nebo zavreny.
    pub fn set_login(&mut self, on: bool, now: u64) {
        self.greeter.enabled = on;
        if on {
            self.greeter.requests.push(Request::List);
        }
        self.greeter_changed(now);
    }

    /// Relace otevrena (init).
    pub fn session_opened(&mut self, id: u64, uid: u64, name: &str, now: u64) {
        self.greeter.sessions.retain(|s| s.id != id);
        self.greeter.sessions.push(Session { id, uid, name: String::from(name), locked: false });
        self.greeter_changed(now);
    }

    /// Relace zamcena nebo odemcena (init).
    pub fn session_locked(&mut self, id: u64, locked: bool, now: u64) {
        if let Some(s) = self.greeter.sessions.iter_mut().find(|s| s.id == id) {
            s.locked = locked;
        }
        if locked {
            // Ucty se mohly zmenit (PIN, klic): nacist znovu.
            self.greeter.requests.push(Request::List);
        }
        self.greeter_changed(now);
    }

    /// Relace skoncila (odhlaseni).
    pub fn session_closed(&mut self, id: u64, now: u64) {
        self.greeter.sessions.retain(|s| s.id != id);
        if self.greeter.enabled {
            self.greeter.requests.push(Request::List);
        }
        self.greeter_changed(now);
    }

    /// Obdelniky poli s tajemstvim na obrazovce (pole prihlaseni, pole
    /// dialogu s heslem, pole s roli `PasswordInput` ve stromu
    /// pristupnosti zobrazenych oken). Diagnosticky snimek je v obrazku
    /// zakryje: ani pocet tecek (delka tajemstvi) se nezaznamena.
    pub fn secure_fields(&self) -> Vec<Rect> {
        let mut v = Vec::new();
        if self.greeter.visible() {
            v.push(self.greeter_layout().field);
        }
        if let Some(f) = self.dialog_layout().and_then(|l| l.field) {
            v.push(f);
        }
        for &g in &self.z {
            let Some(w) = self.windows.get(&g).filter(|w| w.mapped && !w.minimized) else { continue };
            let area = w.rect;
            for (n, layer) in self.screen_nodes(g).unwrap_or_default() {
                if n.role != guiproto::a11y::Role::PasswordInput {
                    continue;
                }
                let r = self.clip_rect(g, layer, n.bounds).intersect(&area);
                if r.w > 0 && r.h > 0 {
                    v.push(r);
                }
            }
        }
        v
    }

    /// Popis obrazovky pro diagnosticky snimek (bez tajemstvi a jeho delky).
    pub(crate) fn greeter_summary(&self) -> Option<String> {
        let g = &self.greeter;
        let mode = match g.mode() {
            Mode::Off => return None,
            Mode::Login => String::from("prihlaseni"),
            Mode::Locked(s) => format!("zamceno (relace {s})"),
        };
        Some(format!(
            "obrazovka {mode}, ucet {}, metoda {:?}, overuje se {}, pole zabezpecene (obsah ani delka se nezapisuji)",
            g.target().map_or(String::from("zadny"), |a| a.name.clone()),
            g.current_method(),
            g.busy
        ))
    }

    /// Prihlasovaci obrazovka je videt (kreslit pres vse, vstup jen ji).
    pub fn greeter_visible(&self) -> bool {
        self.greeter.visible()
    }

    /// Zamknout aktivni relaci (bezpecna obrazovka, klavesa).
    pub fn lock_session(&mut self) {
        if let Some(s) = self.greeter.sessions.iter().rev().find(|s| !s.locked) {
            let id = s.id;
            self.greeter.requests.push(Request::Lock { session: id });
        }
    }

    /// Je nekdo prihlaseny s odemcenou relaci?
    pub fn has_active_session(&self) -> bool {
        self.greeter.sessions.iter().any(|s| !s.locked)
    }

    /// Pozadavky pro sluzbu uctu (vyprazdni frontu).
    pub fn take_login_requests(&mut self) -> Vec<Request> {
        core::mem::take(&mut self.greeter.requests)
    }

    /// Odpoved sluzby uctu.
    pub fn login_reply(&mut self, r: Reply, now: u64) {
        let g = &mut self.greeter;
        match r {
            Reply::Accounts(list) => {
                let keep = g.accounts.get(g.sel).map(|a| a.uid);
                g.accounts = list;
                g.sel = keep.and_then(|u| g.accounts.iter().position(|a| a.uid == u)).unwrap_or(0);
                if let Some(a) = g.target() {
                    let uid = a.uid;
                    g.requests.push(Request::Phrase(uid));
                }
            }
            Reply::Phrase(uid, text) => g.phrase = Some((uid, text)),
            Reply::Ok => {
                g.busy = false;
                g.clear();
            }
            Reply::Denied { retry_after_ms, pin_left } => {
                g.busy = false;
                zero(&mut g.field);
                g.wait_until = now + retry_after_ms * 1_000_000;
                let what = match g.current_method() {
                    Method::Pin if pin_left == 0 => String::from("PIN je zablokovaný. Přihlas se heslem nebo klíčem."),
                    Method::Pin => format!("Nesprávný PIN. Zbývá pokusů: {pin_left}."),
                    Method::Key => String::from("Klíč nebyl ověřen (dotek, PIN klíče nebo jiný klíč)."),
                    Method::Recovery => String::from("Nesprávný obnovovací klíč."),
                    Method::Password => String::from("Nesprávné heslo. Zkus to znovu."),
                };
                g.msg = Some((what, true));
            }
            Reply::LockedOut { retry_after_ms } => {
                g.busy = false;
                zero(&mut g.field);
                g.wait_until = now + retry_after_ms * 1_000_000;
                g.msg = Some((String::from("Moc pokusů za sebou."), true));
            }
            Reply::Error(e) => {
                g.busy = false;
                zero(&mut g.field);
                g.msg = Some((format!("Přihlášení nejde: {e}"), true));
            }
        }
        self.damage_all();
    }

    fn greeter_changed(&mut self, now: u64) {
        if self.greeter.visible() && self.greeter.since == 0 {
            self.greeter.since = now.max(1);
        }
        if !self.greeter.visible() {
            self.greeter.since = 0;
            self.greeter.busy = false;
            self.greeter.clear();
        }
        self.damage_all();
    }

    /// Dalsi probuzeni (odpocet zpomaleni).
    pub(crate) fn greeter_wakeup(&self, now: u64) -> Option<u64> {
        let g = &self.greeter;
        (g.visible() && g.wait_until > now).then(|| now + 1_000_000_000.min(g.wait_until - now))
    }

    fn submit(&mut self, now: u64) {
        let g = &mut self.greeter;
        if g.busy || now < g.wait_until {
            return;
        }
        let method = g.current_method();
        if g.field.is_empty() && method != Method::Key {
            return;
        }
        let secret = core::mem::take(&mut g.field);
        let req = match g.mode() {
            Mode::Locked(session) => Request::Unlock { session, method, secret },
            Mode::Login => match g.accounts.get(g.sel) {
                Some(a) => Request::Auth { uid: a.uid, method, secret },
                None => return,
            },
            Mode::Off => return,
        };
        g.busy = true;
        g.msg = Some((
            String::from(if method == Method::Key { "Dotkni se bezpečnostního klíče…" } else { "Ověřuji…" }),
            false,
        ));
        g.requests.push(req);
        self.damage_all();
    }

    fn select_user(&mut self, i: usize) {
        let g = &mut self.greeter;
        if i >= g.accounts.len() || g.busy || g.sel == i {
            return;
        }
        g.sel = i;
        g.method = None;
        g.clear();
        let uid = g.accounts[i].uid;
        g.requests.push(Request::Phrase(uid));
        self.damage_all();
    }

    fn set_method(&mut self, m: Method) {
        if self.greeter.busy {
            return;
        }
        self.greeter.method = Some(m);
        self.greeter.clear();
        self.damage_all();
    }

    fn input_ready(&self, now: u64) -> bool {
        now >= self.greeter.since + crate::dialog::INPUT_DELAY
    }

    /// Klavesa (jen fyzicky vstup).
    pub(crate) fn greeter_key(&mut self, code: u16, action: Action, now: u64) {
        if action == Action::Release || !self.input_ready(now) {
            return;
        }
        match code {
            KEY_ENTER | KEY_KPENTER => self.submit(now),
            KEY_BACKSPACE => {
                if !self.greeter.busy {
                    self.greeter.field.pop();
                    self.damage_all();
                }
            }
            KEY_ESC => {
                if !self.greeter.busy {
                    self.greeter.clear();
                    self.damage_all();
                }
            }
            KEY_TAB => {
                let ms = self.methods_now();
                let cur = self.greeter.current_method();
                let i = ms.iter().position(|m| *m == cur).unwrap_or(0);
                self.set_method(ms[(i + 1) % ms.len()]);
            }
            KEY_LEFT | KEY_RIGHT if self.greeter.mode() == Mode::Login && self.greeter.field.is_empty() => {
                let n = self.greeter.accounts.len();
                if n > 1 {
                    let i = if code == KEY_LEFT { (self.greeter.sel + n - 1) % n } else { (self.greeter.sel + 1) % n };
                    self.select_user(i);
                }
            }
            _ => {}
        }
    }

    fn methods_now(&self) -> Vec<Method> {
        self.greeter.methods()
    }

    /// Znak (jen fyzicky vstup). PIN jen cislice.
    pub(crate) fn greeter_text(&mut self, ch: char, now: u64) {
        let g = &mut self.greeter;
        if g.busy || ch.is_control() || !now.ge(&(g.since + crate::dialog::INPUT_DELAY)) {
            return;
        }
        if g.current_method() == Method::Pin && !ch.is_ascii_digit() {
            return;
        }
        if g.field.chars().count() < MAX_FIELD {
            g.field.push(ch);
            if g.msg.as_ref().is_some_and(|m| m.1) && now >= g.wait_until {
                g.msg = None;
            }
            self.damage_all();
        }
    }

    /// Klik (jen fyzicky vstup).
    pub(crate) fn greeter_click(&mut self, x: i32, y: i32, now: u64) {
        if !self.input_ready(now) {
            return;
        }
        let l = self.greeter_layout();
        if l.go.contains(x, y) {
            self.submit(now);
            return;
        }
        if let Some((i, _)) = l.users.iter().find(|(_, r)| r.contains(x, y)) {
            let i = *i;
            self.select_user(i);
            return;
        }
        if let Some((m, _)) = l.methods.iter().find(|(_, r)| r.contains(x, y)) {
            let m = *m;
            self.set_method(m);
        }
    }

    /// Kruh avataru prihlasovaci (`locked` = zamykaci) obrazovky:
    /// (stred x, stred y, prumer) v px. Cil prechodu startovni obrazovky;
    /// jde spocitat hned pri pripojeni displeje (nezavisi na uctech).
    pub fn greeter_avatar(&self, locked: bool) -> (f32, f32, f32) {
        let r = avatar_rect(self.w, self.h, locked, &self.style);
        (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0, r.w as f32)
    }

    pub fn greeter_layout(&self) -> Layout {
        let st = &self.style;
        let t = &st.theme;
        let sp = &t.space;
        let w = self.w as i32;
        let cx = w / 2;
        let col_w = 380.min(self.w.saturating_sub(2 * sp[4])) as i32;
        let locked = matches!(self.greeter.mode(), Mode::Locked(_));
        // Svisle: (hodiny), avatar, uzivatele, jmeno, pole, fraze, zprava,
        // metody. Misto pro radek uctu je pri prihlaseni vzdy (poloha
        // avataru nezavisi na poctu uctu: cil prechodu startovni obrazovky).
        let users_h = if !locked { 64 + sp[4] as i32 } else { 0 };
        let avatar = avatar_rect(self.w, self.h, locked, &self.style);
        let mut y = avatar.y;
        let clock_y = y - 140;
        y += 88 + sp[4] as i32;
        let mut users = Vec::new();
        if users_h > 0 && self.greeter.accounts.len() > 1 {
            let n = self.greeter.accounts.len().min(6) as i32;
            let chip = 76;
            let gap = sp[3] as i32;
            let total = n * chip + (n - 1) * gap;
            let mut x = cx - total / 2;
            for i in 0..n as usize {
                users.push((i, Rect::new(x, y, chip as u32, 64)));
                x += chip + gap;
            }
        }
        y += users_h;
        let name_y = y;
        y += t.text.title.line as i32 + sp[4] as i32;
        let field = Rect::new(cx - col_w / 2, y, col_w as u32, 48);
        let go = Rect::new(field.right() as i32 - 44, y + 4, 40, 40);
        y += 48 + sp[4] as i32;
        let phrase = Rect::new(cx - col_w / 2, y, col_w as u32, 32);
        y += 32 + sp[3] as i32;
        let msg_y = y;
        y += 24 + sp[3] as i32;
        let ms = self.greeter.methods();
        let mut methods = Vec::new();
        let widths: Vec<u32> = ms.iter().map(|m| st.width(m.label(), t.text.small) + 2 * sp[4]).collect();
        let total: i32 = widths.iter().map(|w| *w as i32).sum::<i32>() + (ms.len().saturating_sub(1) as i32) * sp[2] as i32;
        let mut x = cx - total / 2;
        for (m, bw) in ms.iter().zip(&widths) {
            methods.push((*m, Rect::new(x, y, *bw, t.size.control_h_compact)));
            x += *bw as i32 + sp[2] as i32;
        }
        let _ = w;
        Layout { avatar, users, name_y, field, go, phrase, msg_y, methods, clock_y }
    }

    /// Nakresli obrazovku pres celou plochu (nepruhledne).
    pub(crate) fn draw_greeter(&mut self, c: &mut Canvas<'_>, now: u64) {
        let l = self.greeter_layout();
        let t = self.style.theme.clone();
        let col = &t.color;
        let sp = &t.space;
        // Pozadi: obraz plochy ztlumeny (okna pod obrazovkou se nekresli).
        let (w, h) = (self.w, self.h);
        let img = self.style.backdrop(w, h);
        if let Some(img) = gfx::Image::new(img, w, h, w) {
            let clip = c.clip();
            c.draw_image(&img, clip, clip, 255, true);
        }
        c.blend(self.screen(), col.scrim);
        let st = &mut self.style;
        let g = &self.greeter;
        let center = |st: &mut Style, c: &mut Canvas<'_>, y: i32, s: &str, ts: TextStyle, color: u32, w: u32| {
            let face = st.face(ts);
            let tw = face.width(s, ts.size) as i32;
            face.draw(c, (w as i32 - tw) / 2, y + face.baseline_in(ts.size, ts.line), s, ts.size, color);
        };
        // Zamceno: hodiny a datum nahore.
        if matches!(g.mode(), Mode::Locked(_))
            && let Some((tm, date)) = self.clock.as_ref()
        {
            center(st, c, l.clock_y, tm, t.text.clock, col.on_accent, w);
            center(st, c, l.clock_y + t.text.clock.line as i32, date, t.text.title, col.on_accent, w);
        }
        // Avatar: kruh s pocatecnim pismenem.
        let target = g.target().cloned();
        let display = target.as_ref().map_or(String::from("Přihlášení"), |a| a.display.clone());
        c.round_fill(l.avatar, l.avatar.h / 2, col.accent);
        let initial: String = display.chars().next().map(|ch| ch.to_uppercase().collect()).unwrap_or_default();
        let ds = t.text.display;
        let face = st.face(ds);
        let iw = face.width(&initial, ds.size) as i32;
        face.draw(c, l.avatar.x + (l.avatar.w as i32 - iw) / 2, l.avatar.y + face.baseline_in(ds.size, l.avatar.h), &initial, ds.size, col.on_accent);
        // Vyber uctu.
        for (i, r) in &l.users {
            let a = &g.accounts[*i];
            if *i == g.sel {
                c.round_fill_bordered(*r, t.radius.card, col.surface, col.accent, 2);
            }
            let dot = Rect::new(r.x + (r.w as i32 - 36) / 2, r.y + 4, 36, 36);
            c.round_fill(dot, 18, col.accent_soft);
            let ch: String = a.display.chars().next().map(|x| x.to_uppercase().collect()).unwrap_or_default();
            let ss = t.text.body_strong;
            let f = st.face(ss);
            let cw = f.width(&ch, ss.size) as i32;
            f.draw(c, dot.x + (36 - cw) / 2, dot.y + f.baseline_in(ss.size, 36), &ch, ss.size, col.accent_ink);
            let cs = t.text.caption;
            let f = st.face(cs);
            let ink = if *i == g.sel { col.ink } else { col.on_accent };
            let nw = f.width(&a.display, cs.size).min(r.w - 8) as i32;
            f.draw_ellipsized(c, r.x + (r.w as i32 - nw) / 2, r.y + 44 + f.baseline_in(cs.size, cs.line), &a.display, cs.size, ink, r.w - 8);
        }
        center(st, c, l.name_y, &display, t.text.title, col.on_accent, w);
        // Pole: pilulka s tecky a tlacitkem.
        let fr = l.field;
        c.round_fill_bordered(fr, fr.h / 2, col.surface, col.border_strong, 1);
        c.round_stroke(Rect::new(fr.x - 2, fr.y - 2, fr.w + 4, fr.h + 4), fr.h / 2 + 2, t.size.focus_ring, col.focus);
        let method = g.current_method();
        let bs = t.text.body_lg;
        let face = st.face(bs);
        let tx = fr.x + sp[5] as i32;
        if g.field.is_empty() {
            face.draw(c, tx, fr.y + face.baseline_in(bs.size, fr.h), method.placeholder(), bs.size, col.ink_faint);
        } else {
            let dots: String = g.field.chars().map(|_| '\u{2022}').collect();
            face.draw(c, tx, fr.y + face.baseline_in(bs.size, fr.h), &dots, bs.size, col.ink);
        }
        c.round_fill(l.go, l.go.h / 2, if g.busy { col.border_strong } else { col.accent });
        let is = t.size.icon_m;
        st.icons.draw(c, if method == Method::Key { "usb" } else { "right" }, l.go.x + (l.go.w - is) as i32 / 2, l.go.y + (l.go.h - is) as i32 / 2, is, t.icon_stroke, col.on_accent);
        // Bezpecnostni fraze (ukaze se az po vyberu uctu).
        if let (Some((uid, p)), Some(a)) = (g.phrase.as_ref(), target.as_ref())
            && *uid == a.uid
            && !p.is_empty()
        {
            let text = format!("Systém · tvá fráze: {p}");
            let ss = t.text.small;
            let pw = st.width(&text, ss) + 2 * sp[4] + t.size.icon_s + sp[2];
            let r = Rect::new((w as i32 - pw as i32) / 2, l.phrase.y, pw, l.phrase.h);
            c.round_fill_bordered(r, r.h / 2, col.trust_bg, col.trust_border, 1);
            let isz = t.size.icon_s;
            st.icons.draw(c, "shieldcheck", r.x + sp[4] as i32, r.y + (r.h - isz) as i32 / 2, isz, t.icon_stroke, col.trust_ink);
            let f = st.face(ss);
            f.draw(c, r.x + (sp[4] + isz + sp[2]) as i32, r.y + f.baseline_in(ss.size, r.h), &text, ss.size, col.trust_ink);
        }
        // Zprava: chyba, prubeh, odpocet zpomaleni.
        let mut text = g.msg.clone();
        if g.wait_until > now {
            let s = (g.wait_until - now).div_ceil(1_000_000_000);
            let base = text.map(|m| m.0).unwrap_or_default();
            text = Some((format!("{base} Další pokus za {s} s (ochrana proti hádání)."), true));
        }
        if let Some((m, err)) = text {
            // Pilulka pod textem: citelne na kazdem pozadi plochy.
            let ss = t.text.small;
            let m = m.trim();
            let pw = st.width(m, ss) + 2 * sp[4];
            let r = Rect::new((w as i32 - pw as i32) / 2, l.msg_y, pw, 24);
            c.round_fill(r, 12, if err { col.danger_soft } else { col.surface });
            center(st, c, l.msg_y + (24 - ss.line as i32) / 2, m, ss, if err { col.danger } else { col.ink }, w);
        }
        // Metody.
        for (m, r) in &l.methods {
            let on = *m == method;
            c.round_fill_bordered(*r, r.h / 2, if on { col.surface } else { col.surface_2 }, if on { col.accent } else { col.border }, 1);
            let ss = t.text.small;
            let f = st.face(ss);
            let tw = f.width(m.label(), ss.size) as i32;
            f.draw(c, r.x + (r.w as i32 - tw) / 2, r.y + f.baseline_in(ss.size, r.h), m.label(), ss.size, col.ink);
        }
    }
}

/// Avatar obrazovky: svisly blok (avatar, radek uctu, jmeno, pole,
/// fraze, zprava, metody) vystredeny, zamcena obrazovka o kus niz (hodiny).
fn avatar_rect(w: u32, h: u32, locked: bool, st: &Style) -> Rect {
    let t = &st.theme;
    let sp = &t.space;
    let users_h = if !locked { 64 + sp[4] as i32 } else { 0 };
    let block = 88 + sp[4] as i32 + users_h + t.text.title.line as i32 + sp[4] as i32 + 48 + sp[4] as i32 + 32 + sp[3] as i32 + 24 + sp[3] as i32 + t.size.control_h as i32;
    let y = (h as i32 - block) / 2 + if locked { 60 } else { 0 };
    Rect::new(w as i32 / 2 - 44, y, 88, 88)
}
