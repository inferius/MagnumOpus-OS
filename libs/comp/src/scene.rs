//! Klienti, okna, buffery a vrstvy; zpravy klienta a transakce.
//!
//! Zpravy menici stav okna se do `COMMIT` jen hromadi (`staged`). `COMMIT`
//! je uplatni najednou, nebo je podrzi, dokud se nesejde cela skupina
//! (transakce pres vic aplikaci) - viz [`crate::frame`].

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use guiproto::a11y::Tree;
use guiproto::control::ConnectInfo;
use guiproto::window::{
    AnimProp, ButtonSide, ClientMsg, Curve, LayerKind, LayerProps, Mode, RegionKind, ServerMsg, commit, prop, wstate,
};
use guiproto::{Error, MAX_DIM, Rect};

use crate::{ClientId, Comp, Pixels};

/// Rozmery okna z tokenu (`style::Style::geo`). Kopie je v kazdem okne,
/// aby geometrie nepotrebovala pristup ke kompozitoru.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geo {
    /// Vyska titulku (`size.title_h`).
    pub title_h: u32,
    /// Sirka ramu.
    pub border: u32,
    /// Viditelne tlacitko okna (`size.title_btn`).
    pub btn: u32,
    /// Cil kliknuti tlacitka (`size.hit_min`), ctverec: jen u maximalizovaneho
    /// okna (roh obrazovky, Fittsuv zakon).
    pub btn_hit: u32,
    /// Presah kruhove zony tlacitka za jeho viditelny kruh (`space.s2`).
    pub btn_slop: u32,
    /// Mezera mezi tlacitky.
    pub btn_gap: u32,
    /// Okraj titulku (vlevo, vpravo).
    pub pad: u32,
    pub badge_h: u32,
    /// Zaobleni okna (`radius.window`).
    pub radius: u32,
    /// Neviditelny okraj pro zmenu velikosti (vne okna).
    pub edge: u32,
    /// Presah stinu: vlevo, nahore, vpravo, dole.
    pub shadow: [u32; 4],
}
/// Nejvic rozpracovanych zprav do `COMMIT` a vrstev, bufferu a oken klienta.
pub const MAX_STAGED: usize = 16384;
pub const MAX_LAYERS: usize = 8192;
pub const MAX_BUFFERS: usize = 256;
pub const MAX_WINDOWS: usize = 32;
/// Nejvetsi buffer (64 MiB) a soucet bufferu klienta (256 MiB).
pub const MAX_BUFFER_BYTES: u64 = 64 << 20;
pub const MAX_CLIENT_BUFFER_BYTES: u64 = 256 << 20;
/// Nejvic bajtu rozpracovanych zprav do `COMMIT`.
pub const MAX_STAGED_BYTES: usize = 16 << 20;
/// Nejdelsi titulek okna (znaku).
pub const MAX_TITLE: usize = 256;

/// Jsou rozmery bufferu prijatelne (pred mapovanim)?
pub fn buffer_ok(w: u32, h: u32, stride: u32) -> bool {
    w > 0 && h > 0 && w <= MAX_DIM && h <= MAX_DIM && stride >= w && stride <= MAX_DIM && stride as u64 * h as u64 * 4 <= MAX_BUFFER_BYTES
}

fn clip_title(s: String) -> String {
    if s.chars().count() <= MAX_TITLE { s } else { s.chars().take(MAX_TITLE).collect() }
}

/// Po tolika chybach klienta se dalsi chyby uz nehlasi (ochrana fronty).
pub const MAX_ERRORS: u32 = 64;

pub struct Buf {
    pub w: u32,
    pub h: u32,
    pub stride: u32,
    pub px: Box<dyn Pixels>,
}

#[derive(Debug, Clone)]
pub struct Anim {
    pub prop: AnimProp,
    pub from: i32,
    pub to: i32,
    pub start: u64,
    pub ms: u32,
    pub curve: Curve,
}

#[derive(Debug, Clone)]
pub struct Layer {
    /// Mistni id okna klienta.
    pub window: u32,
    pub parent: u32,
    pub kind: LayerKind,
    pub props: LayerProps,
    pub anim: Vec<Anim>,
}

pub struct Client {
    pub info: ConnectInfo,
    pub hello: bool,
    pub staged: Vec<ClientMsg>,
    /// Priblizna velikost `staged` v bajtech (z delky zprav).
    pub staged_bytes: usize,
    pub buffers: BTreeMap<u32, Buf>,
    pub layers: BTreeMap<u32, Layer>,
    /// Mistni id okna -> globalni.
    pub windows: BTreeMap<u32, u32>,
    pub frame_wanted: bool,
    /// Transakce cekajici na zobrazeni (`PRESENTED`) a uz odeslane k
    /// zobrazeni.
    pub txn_waiting: Vec<u32>,
    pub txn_in_flight: Vec<u32>,
    pub errors: u32,
    pub ping: Option<(u32, u64)>,
    pub ping_serial: u32,
    pub last_ping: u64,
    pub unresponsive: bool,
}

pub struct Window {
    pub client: ClientId,
    pub local: u32,
    pub mode: Mode,
    pub title: String,
    /// Obdelnik obsahu na obrazovce.
    pub rect: Rect,
    pub min: (u32, u32),
    pub side: ButtonSide,
    pub button_color: u32,
    pub a11y: Tree,
    pub regions: Vec<(Rect, RegionKind)>,
    /// Aspon jeden commit (okno je videt).
    pub mapped: bool,
    pub minimized: bool,
    /// Obdelnik pred maximalizaci.
    pub restore: Option<Rect>,
    /// Menu okna (rozbalene, jen overene polozky; `palette.rs`).
    pub menu: Vec<guiproto::menu::Flat>,
    /// Okno se otevrelo na pozadi (bez akce uzivatele): zvyraznena ikona
    /// v liste, dokud nedostane fokus.
    pub attention: bool,
    /// Systemova cela obrazovka: stav pred ni (`fullscreen.rs`).
    pub fs_saved: Option<crate::fullscreen::Saved>,
    /// Zmena velikosti tahem za levy / horni okraj: pravy / dolni okraj
    /// (obrazovka), ktery zustava na miste, az aplikace potvrdi velikost.
    pub anchor: (Option<i32>, Option<i32>),
    /// Velikost pozadovana behem nepotvrzeneho CONFIGURE (tazeni okraje):
    /// posle se az po potvrzeni nebo terminu. Aplikace tak kresli jen
    /// posledni velikost a nikdy vic nez jednu za snimek.
    pub pending_size: Option<(u32, u32)>,
    /// Cekajici `CONFIGURE`: serial, rozmery, termin.
    pub configure: Option<(u32, u32, u32, u64)>,
    pub serial: u32,
    /// Do kdy ukazovat odznak u bezramoveho okna.
    pub badge_until: u64,
    pub geo: Geo,
}

impl Window {
    /// Obdelnik vcetne ramu a titulku (co okno zabira na obrazovce).
    pub fn outer(&self) -> Rect {
        let (t, b) = (self.geo.title_h, self.geo.border);
        match self.mode {
            Mode::Standard => Rect::new(self.rect.x - b as i32, self.rect.y - (t + b) as i32, self.rect.w + 2 * b, self.rect.h + t + 2 * b),
            Mode::CustomTitle => Rect::new(self.rect.x - b as i32, self.rect.y - b as i32, self.rect.w + 2 * b, self.rect.h + 2 * b),
            Mode::Frameless | Mode::Fullscreen => self.rect,
        }
    }

    /// Co okno zabira vcetne stinu (poskozeni pri zmene).
    pub fn visual(&self) -> Rect {
        let o = self.outer();
        if !self.mode.has_frame() {
            return o;
        }
        let [l, t, r, b] = self.geo.shadow;
        Rect::new(o.x - l as i32, o.y - t as i32, o.w + l + r, o.h + t + b)
    }

    /// Oblast pro zmenu velikosti tazenim (okno a neviditelny okraj kolem).
    /// Nova velikost obsahu (po potvrzeni CONFIGURE); s `anchor` zustane
    /// pravy / dolni okraj na miste.
    pub(crate) fn set_size(&mut self, w: u32, h: u32) {
        self.rect.w = w;
        self.rect.h = h;
        if let Some(r) = self.anchor.0 {
            self.rect.x = r - w as i32;
        }
        if let Some(b) = self.anchor.1 {
            self.rect.y = b - h as i32;
        }
    }

    pub fn grab(&self) -> Rect {
        let o = self.outer();
        if !self.mode.has_frame() {
            return o;
        }
        let e = self.geo.edge;
        Rect::new(o.x - e as i32, o.y - e as i32, o.w + 2 * e, o.h + 2 * e)
    }

    /// Zaobleni rohu okna (bez ramu 0).
    pub fn radius(&self) -> u32 {
        if self.mode.has_frame() { self.geo.radius } else { 0 }
    }

    /// Pruh titulku na obrazovce (u vlastniho titulku je to horni cast
    /// obsahu, kterou kresli aplikace).
    pub fn title_bar(&self) -> Option<Rect> {
        match self.mode {
            Mode::Standard => Some(Rect::new(self.rect.x, self.rect.y - self.geo.title_h as i32, self.rect.w, self.geo.title_h)),
            Mode::CustomTitle => Some(Rect::new(self.rect.x, self.rect.y, self.rect.w, self.geo.title_h)),
            _ => None,
        }
    }

    /// Systemova tlacitka okna (zavrit, maximalizovat, minimalizovat),
    /// viditelna velikost. Zavrit je nejbliz okraji.
    pub fn buttons(&self) -> [Rect; 3] {
        let Some(t) = self.title_bar() else {
            return [Rect::default(); 3];
        };
        let g = &self.geo;
        let b = g.btn;
        let y = t.y + (g.title_h.saturating_sub(b) / 2) as i32;
        let mut r = [Rect::default(); 3];
        for (i, slot) in r.iter_mut().enumerate() {
            let off = g.pad as i32 + i as i32 * (b + g.btn_gap) as i32;
            let x = match self.side {
                ButtonSide::Right => (t.right() as i32) - off - b as i32,
                ButtonSide::Left => t.x + off,
            };
            *slot = Rect::new(x, y, b, b);
        }
        r
    }

    /// Cile kliknuti tlacitek (`hit_min` x `hit_min` kolem stredu, orez na
    /// titulek). Prekryvaji se; plati nejblizsi stred.
    /// Tlacitko titulku pod bodem. Bezne okno: kruh tlacitka a `btn_slop`
    /// (najeti odpovida tomu, co clovek vidi). Maximalizovane: ctverec
    /// `btn_hit` az k hornimu okraji a zavrit az k pravemu (roh obrazovky).
    pub fn button_at(&self, x: i32, y: i32) -> Option<usize> {
        let o = self.outer();
        let hits = self.button_hits();
        let near = |i: usize| {
            let b = self.buttons()[i];
            let (cx, cy) = (b.x + b.w as i32 / 2, b.y + b.h as i32 / 2);
            let (dx, dy) = ((x - cx) as i64, (y - cy) as i64);
            dx * dx + dy * dy
        };
        let inside = |i: usize| {
            if self.restore.is_some() {
                let mut r = hits[i];
                // Az k hornimu okraji ramu (a zavrit k pravemu).
                r = r.union(&Rect::new(r.x, o.y, r.w, 1));
                if i == 0 && self.side == ButtonSide::Right {
                    r = r.union(&Rect::new(o.right() as i32 - 1, r.y, 1, r.h));
                }
                r.contains(x, y)
            } else {
                let rad = (self.geo.btn / 2 + self.geo.btn_slop) as i64;
                hits[i].w > 0 && near(i) <= rad * rad
            }
        };
        (0..3).filter(|&i| inside(i)).min_by_key(|&i| near(i))
    }

    pub fn button_hits(&self) -> [Rect; 3] {
        let Some(t) = self.title_bar() else {
            return [Rect::default(); 3];
        };
        let h = self.geo.btn_hit;
        self.buttons().map(|b| {
            let (cx, cy) = (b.x + b.w as i32 / 2, b.y + b.h as i32 / 2);
            Rect::new(cx - h as i32 / 2, cy - h as i32 / 2, h, h).intersect(&t)
        })
    }

    /// Odznak duvery: pevne misto vlevo v titulku (u leve strany tlacitek
    /// vpravo). U bezramoveho okna u horniho okraje.
    pub fn badge(&self, width: u32) -> Rect {
        let g = &self.geo;
        // Nikdy pres systemova tlacitka.
        let buttons = g.pad + 3 * g.btn + 2 * g.btn_gap + g.pad;
        let width = width.min(self.rect.w.saturating_sub(buttons + g.pad)).max(g.badge_h);
        let dy = (g.title_h.saturating_sub(g.badge_h) / 2) as i32;
        match self.title_bar() {
            Some(t) => {
                let x = match self.side {
                    ButtonSide::Right => t.x + g.pad as i32,
                    ButtonSide::Left => t.right() as i32 - g.pad as i32 - width as i32,
                };
                Rect::new(x, t.y + dy, width, g.badge_h)
            }
            None => Rect::new(self.rect.x + g.pad as i32, self.rect.y + g.pad as i32, width, g.badge_h),
        }
    }

    pub fn state(&self, focused: bool) -> u32 {
        let mut s = 0;
        if focused {
            s |= wstate::FOCUSED;
        }
        if self.restore.is_some() {
            s |= wstate::MAXIMIZED;
        }
        if self.mode == Mode::Fullscreen {
            s |= wstate::FULLSCREEN;
        }
        s
    }
}

impl Comp {
    /// Novy klient (identitu dava init).
    pub fn add_client(&mut self, info: ConnectInfo, now: u64) -> ClientId {
        let id = self.next_client;
        self.next_client += 1;
        self.clients.insert(
            id,
            Client {
                info,
                hello: false,
                staged: Vec::new(),
                staged_bytes: 0,
                buffers: BTreeMap::new(),
                layers: BTreeMap::new(),
                windows: BTreeMap::new(),
                frame_wanted: false,
                txn_waiting: Vec::new(),
                txn_in_flight: Vec::new(),
                errors: 0,
                ping: None,
                ping_serial: 0,
                last_ping: now,
                unresponsive: false,
            },
        );
        id
    }

    /// Klient se odpojil (zavrel kanal nebo skoncil).
    pub fn remove_client(&mut self, id: ClientId) {
        let Some(c) = self.clients.remove(&id) else { return };
        for gid in c.windows.values() {
            self.drop_window(*gid);
        }
        // Potvrzeni akci teto aplikace uz nema smysl.
        let calls = self.services.calls_of(id);
        if self.dialogs.remove_where(|o| matches!(o, crate::dialog::Owner::Action(k) if calls.contains(&k))) {
            self.damage_all();
        }
        self.services.client_gone(id, &mut self.out);
        self.clip_client_gone(id);
    }

    pub(crate) fn drop_window(&mut self, gid: u32) {
        if let Some(w) = self.windows.remove(&gid) {
            // Cele misto okna vcetne stinu (aktivni okno ma vetsi stin).
            let r = w.visual();
            self.damage(r);
        }
        self.z.retain(|&g| g != gid);
        if self.focus == Some(gid) {
            self.focus = None;
            if let Some(&top) = self.z.iter().rev().find(|g| self.windows.get(g).is_some_and(|w| w.mapped && !w.minimized)) {
                self.set_focus(Some(top));
            }
        }
        self.input.window_gone(gid);
        self.damage_panel();
    }

    pub(crate) fn error(&mut self, c: ClientId, op: u32, code: Error) {
        if let Some(cl) = self.clients.get_mut(&c) {
            cl.errors += 1;
            if cl.errors <= MAX_ERRORS {
                self.send(c, ServerMsg::Error { op, code });
            }
        }
    }

    /// Zprava od klienta. `buffer` = pamet z handlu zpravy `BUFFER_CREATE`.
    pub fn client_msg(&mut self, c: ClientId, m: ClientMsg, buffer: Option<Box<dyn Pixels>>, now: u64) {
        self.client_msg_sized(c, m, buffer, 64, now)
    }

    /// Jako [`Comp::client_msg`] s delkou zakodovane zpravy (rozpocet pameti).
    pub fn client_msg_sized(&mut self, c: ClientId, m: ClientMsg, buffer: Option<Box<dyn Pixels>>, size: usize, now: u64) {
        self.input_now = now;
        let Some(cl) = self.clients.get_mut(&c) else { return };
        if !cl.hello && !matches!(m, ClientMsg::Hello { .. }) {
            return self.error(c, op_of(&m), Error::InvalidArgs);
        }
        if m.is_staged() {
            // Dekodovana zprava zabere zhruba dvojnasobek zakodovane.
            let cost = size.saturating_mul(2) + 64;
            if cl.staged.len() >= MAX_STAGED || cl.staged_bytes + cost > MAX_STAGED_BYTES {
                cl.staged.clear();
                cl.staged_bytes = 0;
                return self.error(c, op_of(&m), Error::NoResources);
            }
            cl.staged_bytes += cost;
            cl.staged.push(m);
            return;
        }
        match m {
            ClientMsg::Hello { version } => {
                let app = cl.info.name.clone();
                cl.hello = true;
                if version != guiproto::VERSION {
                    return self.error(c, guiproto::window::op::HELLO, Error::NotSupported);
                }
                self.send(
                    c,
                    ServerMsg::Welcome {
                        version: guiproto::VERSION,
                        w: self.w,
                        h: self.h,
                        scale: 100,
                        app,
                    },
                );
                let mode = crate::style::mode_code(self.style.mode());
                self.send(c, ServerMsg::Theme { mode });
            }
            ClientMsg::WindowCreate { window, mode, w, h, title } => self.window_create(c, window, mode, w, h, clip_title(title)),
            ClientMsg::WindowDestroy { window } => {
                if let Some(gid) = cl.windows.remove(&window) {
                    cl.layers.retain(|_, l| l.window != window);
                    self.drop_window(gid);
                }
            }
            ClientMsg::BufferCreate { buffer: id, w, h, stride } => {
                let used: u64 = cl.buffers.values().map(|b| b.stride as u64 * b.h as u64 * 4).sum();
                let ok = buffer_ok(w, h, stride) && used + stride as u64 * h as u64 * 4 <= MAX_CLIENT_BUFFER_BYTES;
                let fits = buffer.as_ref().and_then(|b| b.pixels()).is_some_and(|p| p.len() as u64 >= stride as u64 * h as u64);
                if !ok || !fits || cl.buffers.contains_key(&id) || cl.buffers.len() >= MAX_BUFFERS {
                    return self.error(c, guiproto::window::op::BUFFER_CREATE, Error::InvalidArgs);
                }
                cl.buffers.insert(id, Buf { w, h, stride, px: buffer.unwrap() });
            }
            ClientMsg::BufferDestroy { buffer } => {
                // Vrstvy, ktere ho ukazuji, ukazou prazdno.
                if cl.buffers.remove(&buffer).is_some() {
                    let wins: Vec<u32> = cl.layers.values().filter(|l| l.props.buffer == buffer).map(|l| l.window).collect();
                    for w in wins {
                        self.damage_local_window(c, w);
                    }
                }
            }
            ClientMsg::Commit { txn, flags, window, serial, token } => {
                let ops = core::mem::take(&mut cl.staged);
                cl.staged_bytes = 0;
                if flags & commit::GROUP != 0 && self.frame_hold(c, ops.clone(), txn, flags, window, serial, token, now) {
                    return;
                }
                self.apply(c, ops, txn, flags, window, serial, now);
            }
            ClientMsg::SyncGroup { count } => {
                let token = self.new_group(c, count, now);
                self.send(c, ServerMsg::SyncToken { token });
            }
            ClientMsg::Pong { serial } => {
                // I odpoved na drivejsi ping (nereagujici aplikace jich
                // dostane vic, starsi odpoved muze prijit pozde).
                if cl.ping.is_some() && cl.ping_serial.wrapping_sub(serial) < 64 {
                    cl.ping = None;
                    if cl.unresponsive {
                        cl.unresponsive = false;
                        let wins: Vec<u32> = cl.windows.values().copied().collect();
                        for g in wins {
                            self.damage_window(g);
                        }
                    }
                }
            }
            ClientMsg::ActionResult { call, ok, value, message } => {
                self.services.action_result(c, call, ok, value, message, &mut self.out);
            }
            ClientMsg::MenuUpdate { window, items } => self.menu_update(c, window, &items),
            ClientMsg::Clipboard { data } => self.clipboard_msg(c, &data, now),
            ClientMsg::WindowSize { window, w, h, min_w, min_h, max_frac_w, max_frac_h } => self.window_size(c, window, w, h, (min_w, min_h), (max_frac_w, max_frac_h), now),
            _ => {}
        }
    }

    fn window_create(&mut self, c: ClientId, local: u32, mode: Mode, w: u32, h: u32, title: String) {
        let cl = self.clients.get_mut(&c).unwrap();
        if local == 0 || cl.windows.contains_key(&local) || cl.windows.len() >= MAX_WINDOWS {
            return self.error(c, guiproto::window::op::WINDOW_CREATE, Error::InvalidArgs);
        }
        let gid = self.next_window;
        self.next_window += 1;
        cl.windows.insert(local, gid);
        // Okno se umisti do pracovni plochy (obrazovka bez listy); pod listu
        // se dostane jen tazenim. Presnou velikost muze aplikace urcit
        // zpravou `WINDOW_SIZE` pred prvnim commitem (`winsize.rs`).
        let geo = self.style.geo();
        let work_h = self.work_area().h as i32;
        let top = if mode == Mode::Standard { (geo.title_h + geo.border) as i32 } else { geo.border as i32 };
        let (w, h) = self.fit_size(mode, w, h, 0, 0);
        // Kaskada od leveho horniho rohu, spodni okraj nad listou.
        let n = self.windows.len() as i32 % 8;
        let (x, y) = if mode == Mode::Fullscreen {
            (0, 0)
        } else {
            let y = (70 + n * 30).min(work_h - h as i32 - geo.border as i32).max(top);
            (60 + n * 36, y)
        };
        let win = Window {
            client: c,
            local,
            mode,
            title,
            rect: Rect::new(x, y, w, h),
            min: (64, 32),
            side: ButtonSide::Right,
            button_color: 0,
            a11y: Tree::new(),
            regions: Vec::new(),
            mapped: false,
            minimized: false,
            restore: None,
            menu: Vec::new(),
            fs_saved: None,
            attention: false,
            anchor: (None, None),
            pending_size: None,
            configure: None,
            serial: 1,
            badge_until: 0,
            geo: self.style.geo(),
        };
        self.windows.insert(gid, win);
        self.z.push(gid);
        // Kontext okna pred prvnim `CONFIGURE`.
        self.send_context(gid);
        self.send(
            c,
            ServerMsg::Configure {
                window: local,
                w,
                h,
                state: 0,
                serial: 1,
            },
        );
    }

    /// Uplatni rozpracovane zpravy transakce (atomicky: mezi tim se nesklada).
    pub(crate) fn apply(&mut self, c: ClientId, ops: Vec<ClientMsg>, txn: u32, flags: u32, window: u32, serial: u32, now: u64) {
        let mut touched: Vec<u32> = Vec::new();
        for m in ops {
            let o = op_of(&m);
            if let Err(e) = self.apply_one(c, m, now, &mut touched) {
                self.error(c, o, e);
            }
        }
        let Some(cl) = self.clients.get_mut(&c) else { return };
        if flags & commit::FRAME != 0 {
            cl.frame_wanted = true;
        }
        cl.txn_waiting.push(txn);
        if let Some(&gid) = cl.windows.get(&window) {
            touched.push(window);
            let mut became_mapped = false;
            if let Some(w) = self.windows.get_mut(&gid) {
                if !w.mapped {
                    w.mapped = true;
                    became_mapped = true;
                }
                if flags & commit::ACK_CONFIGURE != 0
                    && let Some((s, nw, nh, d)) = w.configure
                    && s == serial
                {
                    // Stara i nova plocha vcetne stinu (zmensene okno
                    // odkryje, co bylo pod nim).
                    let old = w.visual();
                    w.set_size(nw, nh);
                    w.configure = None;
                    let new = w.visual();
                    let pending = w.pending_size.take();
                    let anchor = w.anchor;
                    self.damage(old);
                    self.damage(new);
                    // Cena kroku zmeny velikosti (trasovani vstupu).
                    let ms = (now.saturating_sub(d.saturating_sub(crate::SYNC_TIMEOUT))) / 1_000_000;
                    self.trace(now, format_args!("velikost okna {gid} {nw}x{nh} potvrzena aplikaci za {ms} ms"));
                    // Mezitim tazeni dal: posledni velikost hned.
                    if let Some((pw, ph)) = pending {
                        self.configure(gid, pw, ph, now);
                        if let Some(w) = self.windows.get_mut(&gid) {
                            w.anchor = anchor;
                        }
                    }
                }
            }
            if became_mapped {
                // Fokus jen okno spustene uzivatelem (nebo kdyz zadne jine
                // fokus nema); jinak zvyrazneni ikony (`launch.rs`).
                self.new_window_focus(gid, now);
            }
        }
        touched.sort_unstable();
        touched.dedup();
        for w in touched {
            self.damage_local_window(c, w);
        }
    }

    fn apply_one(&mut self, c: ClientId, m: ClientMsg, now: u64, touched: &mut Vec<u32>) -> Result<(), Error> {
        let cl = self.clients.get_mut(&c).ok_or(Error::NotFound)?;
        match m {
            ClientMsg::WindowSet {
                window,
                title,
                min_w,
                min_h,
                side,
                button_color,
            } => {
                let gid = *cl.windows.get(&window).ok_or(Error::NotFound)?;
                let w = self.windows.get_mut(&gid).ok_or(Error::NotFound)?;
                w.title = clip_title(title);
                w.min = (min_w.clamp(16, MAX_DIM), min_h.clamp(16, MAX_DIM));
                w.side = side;
                w.button_color = button_color;
                touched.push(window);
                self.damage_panel();
            }
            ClientMsg::LayerCreate { layer, window, parent, kind } => {
                if layer == 0 || cl.layers.contains_key(&layer) || cl.layers.len() >= MAX_LAYERS || !cl.windows.contains_key(&window) {
                    return Err(Error::InvalidArgs);
                }
                if parent != 0 && cl.layers.get(&parent).is_none_or(|p| p.window != window) {
                    return Err(Error::InvalidArgs);
                }
                cl.layers.insert(
                    layer,
                    Layer {
                        window,
                        parent,
                        kind,
                        props: LayerProps::default(),
                        anim: Vec::new(),
                    },
                );
                touched.push(window);
            }
            ClientMsg::LayerSet { layer, props } => {
                let l = cl.layers.get_mut(&layer).ok_or(Error::NotFound)?;
                let old = l.props.buffer;
                l.props.apply(&props);
                touched.push(l.window);
                // Buffer, ktery uz zadna vrstva neukazuje, je volny:
                // aplikace do nej smi zase kreslit.
                if props.mask & prop::BUFFER != 0 && old != 0 && old != props.buffer && !cl.layers.values().any(|l| l.props.buffer == old) {
                    self.out.push(crate::Out::Client(c, ServerMsg::Released { buffer: old }));
                }
            }
            ClientMsg::LayerDestroy { layer } => {
                let w = cl.layers.get(&layer).ok_or(Error::NotFound)?.window;
                // I s potomky.
                let mut gone = alloc::vec![layer];
                let mut i = 0;
                while i < gone.len() {
                    let p = gone[i];
                    gone.extend(cl.layers.iter().filter(|(_, l)| l.parent == p).map(|(&id, _)| id));
                    i += 1;
                }
                let mut freed = Vec::new();
                for id in gone {
                    if let Some(l) = cl.layers.remove(&id)
                        && l.props.buffer != 0
                    {
                        freed.push(l.props.buffer);
                    }
                }
                for b in freed {
                    if !cl.layers.values().any(|l| l.props.buffer == b) {
                        self.out.push(crate::Out::Client(c, ServerMsg::Released { buffer: b }));
                    }
                }
                touched.push(w);
            }
            ClientMsg::Damage { buffer, .. } => {
                let wins: Vec<u32> = cl.layers.values().filter(|l| l.props.buffer == buffer).map(|l| l.window).collect();
                touched.extend(wins);
            }
            ClientMsg::Animate {
                layer,
                prop,
                target,
                ms,
                curve,
            } => {
                let l = cl.layers.get_mut(&layer).ok_or(Error::NotFound)?;
                let from = anim_value(&l.props, prop);
                l.anim.retain(|a| a.prop != prop);
                l.anim.push(Anim {
                    prop,
                    from,
                    to: target,
                    start: now,
                    ms: ms.clamp(1, 10_000),
                    curve,
                });
                touched.push(l.window);
            }
            ClientMsg::A11yUpdate { window, nodes } => {
                let gid = *cl.windows.get(&window).ok_or(Error::NotFound)?;
                let w = self.windows.get_mut(&gid).ok_or(Error::NotFound)?;
                if !w.a11y.update(nodes) {
                    return Err(Error::NoResources);
                }
            }
            ClientMsg::A11yRemove { window, ids } => {
                let gid = *cl.windows.get(&window).ok_or(Error::NotFound)?;
                self.windows.get_mut(&gid).ok_or(Error::NotFound)?.a11y.remove(&ids);
            }
            ClientMsg::Regions { window, regions } => {
                let gid = *cl.windows.get(&window).ok_or(Error::NotFound)?;
                let w = self.windows.get_mut(&gid).ok_or(Error::NotFound)?;
                w.regions = regions.into_iter().take(64).collect();
            }
            _ => {}
        }
        Ok(())
    }

    /// Poskodi cele okno klienta (mistni id).
    pub(crate) fn damage_local_window(&mut self, c: ClientId, local: u32) {
        if let Some(&gid) = self.clients.get(&c).and_then(|cl| cl.windows.get(&local)) {
            self.damage_window(gid);
        }
    }

    pub(crate) fn damage_window(&mut self, gid: u32) {
        if let Some(w) = self.windows.get(&gid) {
            // Vcetne stinu (odznak bezramoveho okna presahuje jen dovnitr).
            let r = w.visual();
            self.damage(r);
        }
    }

    /// Absolutni poloha vrstvy v obsahu okna (soucet rodicu, minus scroll).
    pub fn layer_origin(&self, c: ClientId, layer: u32) -> Option<(i32, i32)> {
        let cl = self.clients.get(&c)?;
        let mut l = cl.layers.get(&layer)?;
        let (mut x, mut y) = (l.props.rect.x, l.props.rect.y);
        let mut depth = 0;
        while l.parent != 0 {
            let p = cl.layers.get(&l.parent)?;
            x = x.saturating_add(p.props.rect.x.saturating_sub(p.props.scroll.0));
            y = y.saturating_add(p.props.rect.y.saturating_sub(p.props.scroll.1));
            l = p;
            depth += 1;
            if depth > 64 {
                return None;
            }
        }
        Some((x, y))
    }

    pub(crate) fn set_focus(&mut self, gid: Option<u32>) {
        if self.focus == gid {
            return;
        }
        if let Some(old) = self.focus.take()
            && let Some(w) = self.windows.get(&old)
        {
            let (c, l) = (w.client, w.local);
            self.send_input(c, l, guiproto::window::InputEvent::Focus(false), 0, false, 0);
            self.damage_window(old);
        }
        self.focus = gid;
        if let Some(g) = gid
            && let Some(w) = self.windows.get_mut(&g)
        {
            w.attention = false;
            let (c, l) = (w.client, w.local);
            self.send_input(c, l, guiproto::window::InputEvent::Focus(true), 0, false, 0);
            // Nahoru v poradi.
            self.z.retain(|&x| x != g);
            self.z.push(g);
            self.damage_window(g);
            self.damage_panel();
            // Nove okno s fokusem vi, co je ve schrance (menu Vlozit).
            if !self.clipboard.formats().is_empty() {
                self.clip_changed();
            }
        }
    }
}

pub(crate) fn anim_value(p: &LayerProps, prop: AnimProp) -> i32 {
    match prop {
        AnimProp::Opacity => p.opacity as i32,
        AnimProp::X => p.rect.x,
        AnimProp::Y => p.rect.y,
        AnimProp::ScrollX => p.scroll.0,
        AnimProp::ScrollY => p.scroll.1,
    }
}

pub(crate) fn op_of(m: &ClientMsg) -> u32 {
    use guiproto::window::op;
    match m {
        ClientMsg::Hello { .. } => op::HELLO,
        ClientMsg::WindowCreate { .. } => op::WINDOW_CREATE,
        ClientMsg::WindowSet { .. } => op::WINDOW_SET,
        ClientMsg::WindowDestroy { .. } => op::WINDOW_DESTROY,
        ClientMsg::BufferCreate { .. } => op::BUFFER_CREATE,
        ClientMsg::BufferDestroy { .. } => op::BUFFER_DESTROY,
        ClientMsg::LayerCreate { .. } => op::LAYER_CREATE,
        ClientMsg::LayerSet { .. } => op::LAYER_SET,
        ClientMsg::LayerDestroy { .. } => op::LAYER_DESTROY,
        ClientMsg::Damage { .. } => op::DAMAGE,
        ClientMsg::Animate { .. } => op::ANIMATE,
        ClientMsg::A11yUpdate { .. } => op::A11Y_UPDATE,
        ClientMsg::A11yRemove { .. } => op::A11Y_REMOVE,
        ClientMsg::Regions { .. } => op::REGIONS,
        ClientMsg::Commit { .. } => op::COMMIT,
        ClientMsg::SyncGroup { .. } => op::SYNC_GROUP,
        ClientMsg::Pong { .. } => op::PONG,
        ClientMsg::ActionResult { .. } => op::ACTION_RESULT,
        ClientMsg::MenuUpdate { .. } => op::MENU_UPDATE,
        ClientMsg::Clipboard { .. } => op::CLIPBOARD,
        ClientMsg::WindowSize { .. } => op::WINDOW_SIZE,
    }
}
