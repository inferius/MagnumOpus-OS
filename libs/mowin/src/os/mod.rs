//! Symboly C `mowin_*` (include/mowin.h): vic oken a popupu nad jednim
//! spojenim s kompozitorem.
//!
//! Spojeni: relace okna zdedena roli `Gui` (`osrt::gui`), jinak kanal okna
//! ze startovni zpravy aplikace (`osrt::start::take_handles`). Druha kopie
//! `libos` (vedle kopie ve std) potrebuje handle vlastniho procesu pro
//! mapovani bufferu: `set_self_process`.
//!
//! Stav je za zamkem: Wine vola ovladac z vice vlaken (kresleni z vlaken
//! aplikace, udalosti z vlakna ovladace). `mowin_next_event` ceka **mimo**
//! zamek na duplikatu kanalu, udalosti probuzeni a udalosti `QUEUED`
//! (jine vlakno pri cekani na buffer precetlo zpravy a zaradilo udalosti).
//!
//! Obnova po padu kompozitoru (`restore.rs`): vse odeslane se zaznamenava
//! (`guiproto::replay`); pri ztrate spojeni nova relace pres listek role
//! Gui a znovu posledni potvrzeny stav (okna, vlastnici, buffery, vrstvy),
//! ovladac dostane `MOWIN_EV_EXPOSE`.

use std::collections::{BTreeMap, VecDeque};
use std::ffi::{CStr, c_char};
use std::os::magnumopus::runtime as rt;
use std::sync::{Mutex, OnceLock};

use guiproto::Rect;
use guiproto::window::{ClientMsg, LayerKind, Mode, ServerMsg, commit};
use libos::{Channel, Error, Event, Handle, Result, Signals};

use crate::event::{MoEvent, from_input, kind};

pub mod audio;
mod clip;
mod pointer;
mod restore;
mod surface;

use surface::{Buf, Surface};

/// Jak dlouho cekat na `WELCOME` a na uvolneni bufferu kompozitorem.
const REPLY_NS: u64 = 5_000_000_000;

/// Zpravy, ktere kompozitor odklada do `COMMIT` (vrstvy, titulek), se
/// posilaji az s prvnim snimkem: okno, ktere se nikdy neukaze (skryta okna
/// Wine), nesmi nechat v kompozitoru rozpracovanou transakci.
struct Win {
    w: u32,
    h: u32,
    /// `CONFIGURE` cekajici na potvrzeni (serial, w, h).
    ack: Option<(u32, u32, u32)>,
    content: Surface,
    shown: bool,
    /// Titulek pred prvnim snimkem (posle se s nim).
    title: Option<String>,
}

struct Popup {
    window: u32,
    rect: Rect,
    surface: Surface,
    /// Vrstva uz je v kompozitoru (`LAYER_CREATE` s prvnim snimkem).
    created: bool,
}

pub(crate) struct State {
    ch: Option<Channel>,
    hello: bool,
    version: u32,
    screen: (u32, u32),
    work: (u32, u32),
    scale: u32,
    refresh: u32,
    windows: BTreeMap<u32, Win>,
    popups: BTreeMap<u32, Popup>,
    next_id: u32,
    next_layer: u32,
    next_buffer: u32,
    txn: u32,
    queue: VecDeque<MoEvent>,
    /// Posledni cislo gesta (fyzicky stisk doruceny oknum procesu).
    gesture: u32,
    clip: clip::Clip,
    /// Zaznam odeslaneho stavu pro obnovu po padu kompozitoru.
    replay: guiproto::replay::Replay,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static WAKE: OnceLock<Option<Event>> = OnceLock::new();
/// Signal: ve fronte je udalost (zaradilo ji jine vlakno).
static QUEUED: OnceLock<Option<Event>> = OnceLock::new();

fn queued_event() -> Option<&'static Event> {
    QUEUED.get_or_init(|| Event::create().ok()).as_ref()
}

#[repr(C)]
pub struct MowinDisplay {
    pub screen_w: u32,
    pub screen_h: u32,
    pub work_w: u32,
    pub work_h: u32,
    pub scale_pct: u32,
    pub refresh_mhz: u32,
}

pub(crate) fn code(e: Error) -> i32 {
    e.code()
}

fn wake_event() -> Option<&'static Event> {
    WAKE.get_or_init(|| Event::create().ok()).as_ref()
}

pub(crate) fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let st = g.get_or_insert_with(|| {
        libos::rt::set_self_process(rt::start::self_process());
        // Relace zdedena roli Gui ma prednost; jinak okno aplikace.
        let ch = rt::gui::take_session_raw().map(|h| Channel::from_handle(Handle::from_raw(h))).or_else(|| {
            let mut it = rt::start::take_handles().into_iter().map(Handle::from_raw);
            it.next().map(Channel::from_handle)
        });
        State {
            ch,
            hello: false,
            version: 0,
            screen: (0, 0),
            work: (0, 0),
            scale: 100,
            refresh: 0,
            windows: BTreeMap::new(),
            popups: BTreeMap::new(),
            next_id: 1,
            next_layer: 1,
            next_buffer: 1,
            txn: 0,
            queue: VecDeque::new(),
            gesture: 0,
            clip: clip::Clip::default(),
            replay: guiproto::replay::Replay::new(),
        }
    });
    f(st)
}

pub(crate) fn c_str(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: volajici z C predava retezec zakonceny nulou.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

impl State {
    fn ch(&self) -> Result<&Channel> {
        self.ch.as_ref().ok_or(Error::NotFound)
    }

    pub(crate) fn send(&mut self, m: ClientMsg) -> Result<()> {
        self.send_with(m, Vec::new())
    }

    /// Posle zpravu (a zaznamena ji pro obnovu). Kompozitor spadl: obnova
    /// (zprava je v zaznamu, rozpracovana transakce se ztrati a ovladac
    /// okno nakresli znovu po `EXPOSE`).
    fn send_with(&mut self, m: ClientMsg, handles: Vec<Handle>) -> Result<()> {
        self.replay.record(&m);
        match self.ch()?.write(&m.encode(), handles) {
            Err(Error::PeerClosed) => self.recover(),
            r => r,
        }
    }

    /// `HELLO` a `WELCOME` (verze, obrazovka) jednou za spojeni.
    fn ensure_hello(&mut self) -> Result<()> {
        if self.hello {
            return Ok(());
        }
        self.ch()?.write(&ClientMsg::Hello { version: guiproto::VERSION }.encode(), Vec::new()).map_err(|e| if e == Error::PeerClosed { Error::ServiceUnavailable } else { e })?;
        let deadline = libos::time::now().saturating_add(REPLY_NS);
        loop {
            let m = self.ch()?.read(Some(deadline)).map_err(|e| if e == Error::PeerClosed { Error::ServiceUnavailable } else { e })?;
            match ServerMsg::decode(&m.data)? {
                ServerMsg::Welcome { version, w, h, scale, .. } => {
                    self.hello = true;
                    self.version = version;
                    self.screen = (w, h);
                    self.work = (w, h);
                    self.scale = scale;
                    return Ok(());
                }
                ServerMsg::Error { code, .. } => return Err(code),
                _ => {}
            }
        }
    }

    fn new_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn new_layer(&mut self) -> u32 {
        let id = self.next_layer;
        self.next_layer += 1;
        id
    }

    /// Zpracuje zpravu kompozitoru; udalosti pro ovladac jdou do fronty.
    pub(crate) fn handle(&mut self, m: ServerMsg) -> Result<()> {
        match m {
            ServerMsg::Input { window, records } => {
                for r in &records {
                    if r.serial != 0 {
                        self.gesture = r.serial;
                    }
                    let mut e = from_input(r);
                    e.window = window;
                    self.enqueue(e);
                }
            }
            ServerMsg::Configure { window, w, h, state, serial } => {
                self.replay.configured(window, w, h);
                if let Some(win) = self.windows.get_mut(&window) {
                    win.ack = Some((serial, w, h));
                    win.w = w;
                    win.h = h;
                    let mut e = MoEvent::simple(kind::RESIZE, w as i32, h as i32);
                    e.c = state as i32;
                    e.window = window;
                    self.enqueue(e);
                }
            }
            ServerMsg::Released { buffer } => {
                for s in self.windows.values_mut().map(|w| &mut w.content).chain(self.popups.values_mut().map(|p| &mut p.surface)) {
                    for b in s.bufs.iter_mut().flatten().filter(|b| b.id == buffer) {
                        b.busy = false;
                    }
                }
            }
            ServerMsg::Close { window } => self.push(kind::CLOSE, window, [0; 4]),
            ServerMsg::Ping { serial } => self.send(ClientMsg::Pong { serial })?,
            ServerMsg::PopoverDismiss { window } => self.push(kind::POPUP_DISMISS, window, [0; 4]),
            ServerMsg::WindowPlace { window, rect } => self.push(kind::PLACE, window, [rect.x, rect.y, rect.w as i32, rect.h as i32]),
            ServerMsg::WindowContext { window, screen, work_area, scale, refresh_mhz, .. } => {
                self.screen = (screen.w, screen.h);
                self.work = (work_area.w, work_area.h);
                self.scale = scale;
                self.refresh = refresh_mhz;
                self.push(kind::DISPLAY, window, [0; 4]);
            }
            ServerMsg::PointerLockState { window, locked } => self.push(kind::POINTER_LOCK, window, [locked as i32, 0, 0, 0]),
            ServerMsg::Clipboard { data } => {
                if let Ok(m) = clipproto::ServerMsg::decode(&data) {
                    self.clip_event(m)?;
                }
            }
            ServerMsg::Error { op, code } => println!("mowin: kompozitor odmitl operaci {op}: {code}"),
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn push(&mut self, k: u32, window: u32, v: [i32; 4]) {
        let mut e = MoEvent::simple(k, v[0], v[1]);
        e.c = v[2];
        e.d = v[3];
        e.window = window;
        self.enqueue(e);
    }

    fn enqueue(&mut self, e: MoEvent) {
        self.queue.push_back(e);
        if let Some(q) = queued_event() {
            let _ = q.signal();
        }
    }

    /// Zpracuje vse, co uz v kanalu je (bez cekani).
    fn drain(&mut self) -> Result<bool> {
        let mut got = false;
        loop {
            match self.ch()?.try_read() {
                Ok(m) => {
                    got = true;
                    let m = ServerMsg::decode(&m.data)?;
                    self.handle(m)?;
                }
                Err(Error::ShouldWait) => return Ok(got),
                // Kompozitor spadl: obnova, pak cist z nove relace.
                Err(Error::PeerClosed) => {
                    self.recover()?;
                    got = true;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Se zamkem: precte a zpracuje zpravy do `deadline`, vrati po kazde
    /// davce zprav (volajici si overi svou podminku: uvolneny buffer, data
    /// schranky). `Ok(false)` = vyprsel cas.
    pub(crate) fn pump(&mut self, deadline: u64) -> Result<bool> {
        loop {
            if self.drain()? {
                return Ok(true);
            }
            let ch = self.ch()?;
            match libos::wait_one(ch.handle(), Signals::READABLE | Signals::PEER_CLOSED, Some(deadline)) {
                Ok(_) => {}
                Err(Error::TimedOut) => return Ok(false),
                Err(e) => return Err(e),
            }
        }
    }

    /// Ukaze snimek ve vrstve: okno (`popup = None`) nebo popup.
    fn present(&mut self, window: u32, popup: Option<u32>, src: &[u8], pitch: u32, w: u32, h: u32) -> Result<()> {
        // Popup okna, ktere jeste neni ukazane, by commit okna ukazal
        // prazdne: nejdriv obsah okna (ovladac snimek zopakuje).
        if popup.is_some() && !self.windows.get(&window).is_some_and(|w| w.shown) {
            return Err(Error::BadState);
        }
        // Velikost vrstvy: okno = posledni CONFIGURE, popup = jeho obdelnik.
        let (lw, lh) = match popup {
            Some(p) => {
                let r = self.popups.get(&p).ok_or(Error::NotFound)?.rect;
                (r.w.max(1), r.h.max(1))
            }
            None => {
                let win = self.windows.get(&window).ok_or(Error::NotFound)?;
                (win.w.max(1), win.h.max(1))
            }
        };
        // Zadni buffer spravne velikosti, volny (ceka na RELEASED).
        let back = self.surface(window, popup)?.back();
        let fits = self.surface(window, popup)?.bufs[back].as_ref().is_some_and(|b| b.w == lw && b.h == lh);
        if !fits {
            if let Some(old) = self.surface(window, popup)?.bufs[back].take() {
                self.send(ClientMsg::BufferDestroy { buffer: old.id })?;
            }
            let id = self.next_buffer;
            self.next_buffer += 1;
            let (b, msg, ro) = Buf::new(id, lw, lh)?;
            self.send_with(msg, vec![ro])?;
            self.surface(window, popup)?.bufs[back] = Some(b);
        }
        let deadline = libos::time::now().saturating_add(REPLY_NS);
        while self.surface(window, popup)?.bufs[back].as_ref().is_some_and(|b| b.busy) {
            if !self.pump(deadline)? && libos::time::now() >= deadline {
                // Kompozitor buffer nevratil: pouzit ho stejne (nejhur
                // roztrzeny vlastni snimek).
                break;
            }
        }
        let rect = match popup {
            Some(p) => self.popups[&p].rect,
            None => Rect::new(0, 0, lw, lh),
        };
        let s = self.surface(window, popup)?;
        let b = s.bufs[back].as_mut().ok_or(Error::BadState)?;
        if !b.fill(src, pitch, w, h) {
            return Err(Error::InvalidArgs);
        }
        b.busy = true;
        let id = b.id;
        s.front = back;
        let layer = s.layer;
        self.first_frame(window, popup, layer)?;
        self.send(ClientMsg::Damage { buffer: id, rect: Rect::new(0, 0, lw, lh) })?;
        self.send(ClientMsg::LayerSet { layer, props: Surface::props(id, rect, popup.is_some()) })?;
        self.commit(window, popup.is_none() && (w, h) == (lw, lh))
    }

    /// Pred prvnim snimkem vrstvy: `LAYER_CREATE` (a titulek okna).
    fn first_frame(&mut self, window: u32, popup: Option<u32>, layer: u32) -> Result<()> {
        match popup {
            Some(p) => {
                let pop = self.popups.get_mut(&p).ok_or(Error::NotFound)?;
                if !pop.created {
                    pop.created = true;
                    self.send(ClientMsg::LayerCreate { layer, window, parent: 0, kind: LayerKind::Buffer })?;
                }
            }
            None => {
                let win = self.windows.get_mut(&window).ok_or(Error::NotFound)?;
                if !win.shown {
                    let title = win.title.take();
                    self.send(ClientMsg::LayerCreate { layer, window, parent: 0, kind: LayerKind::Buffer })?;
                    if let Some(t) = title {
                        self.send(set_title(window, t))?;
                    }
                }
            }
        }
        Ok(())
    }

    fn surface(&mut self, window: u32, popup: Option<u32>) -> Result<&mut Surface> {
        match popup {
            Some(p) => self.popups.get_mut(&p).map(|p| &mut p.surface).ok_or(Error::NotFound),
            None => self.windows.get_mut(&window).map(|w| &mut w.content).ok_or(Error::NotFound),
        }
    }

    /// `COMMIT` okna; `redrawn` = obsah v potvrzene velikosti (potvrdi
    /// cekajici `CONFIGURE`).
    fn commit(&mut self, window: u32, redrawn: bool) -> Result<()> {
        self.txn += 1;
        let win = self.windows.get_mut(&window).ok_or(Error::NotFound)?;
        let mut flags = 0;
        let mut serial = 0;
        if redrawn && let Some((s, aw, ah)) = win.ack.take_if(|(_, aw, ah)| (*aw, *ah) == (win.w, win.h)) {
            let _ = (aw, ah);
            flags |= commit::ACK_CONFIGURE;
            serial = s;
        }
        win.shown = true;
        let txn = self.txn;
        self.send(ClientMsg::Commit { txn, flags, window, serial, token: 0 })
    }
}

fn set_title(window: u32, title: String) -> ClientMsg {
    ClientMsg::WindowSet { window, title, min_w: 0, min_h: 0, side: guiproto::window::ButtonSide::Right, button_color: 0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn mowin_available() -> i32 {
    with_state(|s| s.ch.is_some() as i32)
}

/// # Safety
///
/// `out` je platny ukazatel na `mowin_display`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mowin_display_info(out: *mut MowinDisplay) -> i32 {
    if out.is_null() {
        return code(Error::InvalidArgs);
    }
    let r = with_state(|s| {
        s.ensure_hello()?;
        Ok(MowinDisplay { screen_w: s.screen.0, screen_h: s.screen.1, work_w: s.work.0, work_h: s.work.1, scale_pct: s.scale, refresh_mhz: s.refresh })
    });
    match r {
        // SAFETY: `out` overen vyse, platny podle smlouvy.
        Ok(d) => unsafe {
            out.write(d);
            0
        },
        Err(e) => code(e),
    }
}

fn mode_of(m: u32) -> Result<Mode> {
    match m {
        0 => Ok(Mode::Standard),
        2 => Ok(Mode::Frameless),
        _ => Err(Error::InvalidArgs),
    }
}

/// # Safety
///
/// `title` je retezec C nebo NULL, `out_id` platny nebo NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mowin_window_create(mode: u32, w: u32, h: u32, title: *const c_char, owner: u32, out_id: *mut u32) -> i32 {
    let title = c_str(title);
    let r: Result<u32> = with_state(|s| {
        let mode = mode_of(mode)?;
        s.ensure_hello()?;
        if owner != 0 && !s.windows.contains_key(&owner) {
            return Err(Error::NotFound);
        }
        let id = s.new_id();
        let layer = s.new_layer();
        s.send(ClientMsg::WindowCreate { window: id, mode, w: w.max(1), h: h.max(1), title })?;
        if owner != 0 && s.version >= 7 {
            s.send(ClientMsg::WindowOwner { window: id, owner })?;
        }
        s.windows.insert(id, Win { w: w.max(1), h: h.max(1), ack: None, content: Surface::new(layer), shown: false, title: None });
        Ok(id)
    });
    match r {
        Ok(id) => {
            if !out_id.is_null() {
                // SAFETY: platny podle smlouvy.
                unsafe { out_id.write(id) };
            }
            0
        }
        Err(e) => code(e),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn mowin_window_destroy(win: u32) -> i32 {
    with_state(|s| {
        let Some(w) = s.windows.remove(&win) else { return code(Error::NotFound) };
        let pops: Vec<u32> = s.popups.iter().filter(|(_, p)| p.window == win).map(|(k, _)| *k).collect();
        let mut bufs: Vec<u32> = w.content.buffer_ids().collect();
        for p in pops {
            if let Some(p) = s.popups.remove(&p) {
                bufs.extend(p.surface.buffer_ids());
            }
        }
        let mut r = s.send(ClientMsg::WindowDestroy { window: win });
        for b in bufs {
            r = r.and(s.send(ClientMsg::BufferDestroy { buffer: b }));
        }
        r.map_or_else(code, |_| 0)
    })
}

/// # Safety
///
/// `title` je retezec C nebo NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mowin_window_set_title(win: u32, title: *const c_char) -> i32 {
    let t = c_str(title);
    with_state(|s| {
        let Some(w) = s.windows.get_mut(&win) else { return code(Error::NotFound) };
        // Titulek plati s COMMIT: u neukazaneho okna az s prvnim snimkem.
        if !w.shown {
            w.title = Some(t);
            return 0;
        }
        s.send(set_title(win, t)).and_then(|_| s.commit(win, false)).map_or_else(code, |_| 0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn mowin_window_set_size(win: u32, w: u32, h: u32, min_w: u32, min_h: u32) -> i32 {
    with_state(|s| {
        if !s.windows.contains_key(&win) {
            return code(Error::NotFound);
        }
        s.send(ClientMsg::WindowSize { window: win, w, h, min_w, min_h, max_frac_w: 0, max_frac_h: 0 }).map_or_else(code, |_| 0)
    })
}

/// # Safety
///
/// `w`, `h` platne nebo NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mowin_window_size(win: u32, w: *mut u32, h: *mut u32) -> i32 {
    with_state(|s| match s.windows.get(&win) {
        Some(x) => {
            // SAFETY: platne nebo NULL podle smlouvy.
            unsafe {
                if !w.is_null() {
                    w.write(x.w);
                }
                if !h.is_null() {
                    h.write(x.h);
                }
            }
            0
        }
        None => code(Error::NotFound),
    })
}

/// Snimek z C: delka podle kroku a vysky.
///
/// # Safety
///
/// `pixels` ukazuje na `pitch * (h - 1) + 4 * w` citelnych bajtu.
unsafe fn frame<'a>(pixels: *const u8, pitch: u32, w: u32, h: u32) -> Result<&'a [u8]> {
    if pixels.is_null() || w == 0 || h == 0 {
        return Err(Error::InvalidArgs);
    }
    let len = (pitch as usize).checked_mul(h as usize - 1).and_then(|v| v.checked_add(w as usize * 4)).ok_or(Error::InvalidArgs)?;
    // SAFETY: delka podle smlouvy funkce (volajici drzi snimek po dobu volani).
    Ok(unsafe { core::slice::from_raw_parts(pixels, len) })
}

/// # Safety
///
/// Viz [`frame`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mowin_window_present(win: u32, pixels: *const u8, pitch: u32, w: u32, h: u32) -> i32 {
    // SAFETY: smlouva funkce.
    let src = match unsafe { frame(pixels, pitch, w, h) } {
        Ok(s) => s,
        Err(e) => return code(e),
    };
    with_state(|s| s.present(win, None, src, pitch, w, h).map_or_else(code, |_| 0))
}

/// # Safety
///
/// `out_id` platny nebo NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mowin_popup_create(win: u32, x: i32, y: i32, w: u32, h: u32, out_id: *mut u32) -> i32 {
    let r: Result<u32> = with_state(|s| {
        if !s.windows.contains_key(&win) {
            return Err(Error::NotFound);
        }
        let id = s.new_id();
        let layer = s.new_layer();
        s.popups.insert(id, Popup { window: win, rect: Rect::new(x, y, w.max(1), h.max(1)), surface: Surface::new(layer), created: false });
        Ok(id)
    });
    match r {
        Ok(id) => {
            if !out_id.is_null() {
                // SAFETY: platny podle smlouvy.
                unsafe { out_id.write(id) };
            }
            0
        }
        Err(e) => code(e),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn mowin_popup_move(popup: u32, x: i32, y: i32, w: u32, h: u32) -> i32 {
    with_state(|s| {
        let Some(p) = s.popups.get_mut(&popup) else { return code(Error::NotFound) };
        let same = (p.rect.w, p.rect.h) == (w.max(1), h.max(1));
        p.rect = Rect::new(x, y, w.max(1), h.max(1));
        let (win, layer, rect) = (p.window, p.surface.layer, p.rect);
        // Stejna velikost: jen posun vrstvy (obsah zustane).
        let front = p.surface.bufs[p.surface.front].as_ref().map(|b| b.id).filter(|_| p.created);
        match (same, front) {
            (true, Some(id)) => s.send(ClientMsg::LayerSet { layer, props: Surface::props(id, rect, true) }).and_then(|_| s.commit(win, false)).map_or_else(code, |_| 0),
            _ => 0,
        }
    })
}

/// # Safety
///
/// Viz [`frame`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mowin_popup_present(popup: u32, pixels: *const u8, pitch: u32, w: u32, h: u32) -> i32 {
    // SAFETY: smlouva funkce.
    let src = match unsafe { frame(pixels, pitch, w, h) } {
        Ok(s) => s,
        Err(e) => return code(e),
    };
    with_state(|s| {
        let Some(win) = s.popups.get(&popup).map(|p| p.window) else { return code(Error::NotFound) };
        s.present(win, Some(popup), src, pitch, w, h).map_or_else(code, |_| 0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn mowin_popup_destroy(popup: u32) -> i32 {
    with_state(|s| {
        let Some(p) = s.popups.remove(&popup) else { return code(Error::NotFound) };
        let mut r = Ok(());
        if p.created {
            r = s.send(ClientMsg::LayerDestroy { layer: p.surface.layer }).and_then(|_| s.commit(p.window, false));
        }
        for b in p.surface.buffer_ids() {
            r = r.and(s.send(ClientMsg::BufferDestroy { buffer: b }));
        }
        r.map_or_else(code, |_| 0)
    })
}

/// # Safety
///
/// `ev` je platny ukazatel na `mowin_event`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mowin_next_event(ev: *mut MoEvent, timeout_ns: i64) -> i32 {
    if ev.is_null() {
        return code(Error::InvalidArgs);
    }
    let deadline = match timeout_ns {
        t if t < 0 => u64::MAX,
        t => libos::time::now().saturating_add(t as u64),
    };
    let r = next_event(deadline);
    match r {
        // SAFETY: `ev` overen vyse.
        Ok(Some(e)) => unsafe {
            ev.write(e);
            1
        },
        Ok(None) | Err(Error::TimedOut) => 0,
        Err(e) => code(e),
    }
}

/// Dalsi udalost do `deadline`; ceka mimo zamek stavu.
fn next_event(deadline: u64) -> Result<Option<MoEvent>> {
    loop {
        let wait = with_state(|s| -> Result<Option<Handle>> {
            if let Some(q) = queued_event() {
                let _ = q.reset();
            }
            if s.ch.is_none() {
                return Ok(None);
            }
            s.drain()?;
            if !s.queue.is_empty() {
                return Ok(None);
            }
            Ok(Some(s.ch()?.handle().duplicate(None)?))
        })?;
        let Some(ch) = wait else {
            return with_state(|s| Ok(s.queue.pop_front()));
        };
        let wake = wake_event();
        let mut items = vec![(&ch, Signals::READABLE | Signals::PEER_CLOSED)];
        if let Some(w) = wake {
            items.push((w.handle(), Signals::SIGNALED));
        }
        if let Some(q) = queued_event() {
            items.push((q.handle(), Signals::SIGNALED));
        }
        match libos::wait_many(&items, Some(deadline)) {
            Ok((1, _)) if wake.is_some() => {
                if let Some(w) = wake {
                    let _ = w.reset();
                }
                return Ok(None);
            }
            Ok(_) => {}
            Err(Error::TimedOut) => return Ok(None),
            Err(e) => return Err(e),
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn mowin_wake() -> i32 {
    match wake_event() {
        Some(e) => e.signal().map_or_else(code, |_| 0),
        None => code(Error::NoResources),
    }
}
