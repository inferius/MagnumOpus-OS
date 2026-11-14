//! Symboly C `mosdl_*` (include/mosdl.h) nad oknem z `libs/ui`.
//!
//! Kanal okna a listek k obnove jsou prvni dva netagovane handly startovni
//! zpravy (init `gui.rs`); bere si je `osrt::start::take_handles`. Druha
//! kopie `libos` v aplikaci (vedle kopie ve std) potrebuje handle
//! vlastniho procesu pro mapovani bufferu: `set_self_process`.
//!
//! Stav je za zamkem (SDL vola video z hlavniho vlakna); udalost probuzeni
//! je mimo zamek, aby `mosdl_wake` nemusel cekat na vlakno, ktere prave
//! ceka na udalosti.

use std::collections::VecDeque;
use std::ffi::{CStr, c_char};
use std::os::magnumopus::runtime as rt;
use std::sync::{Mutex, OnceLock};

use guiproto::window::Mode;
use libos::{Channel, Error, Event, Handle, Result, Signals};
use ui::client::{Event as WinEvent, Link, Window};

use crate::event::{MosdlEvent, copy_frame, from_input, kind};

mod clip;
mod pointer;

pub(crate) struct State {
    /// Kanal okna a listek, dokud okno neni otevrene.
    link: Option<Link>,
    win: Option<Window>,
    /// Udalosti prijate behem cekani na schranku (`clip.rs`).
    pending: VecDeque<MosdlEvent>,
    /// Text posledniho `mosdl_clipboard_fetch`.
    clip_text: Vec<u8>,
    /// Posledni kurzor (po obnove kompozitoru se posle znovu).
    cursor: Option<guiproto::cursor::Cursor>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static WAKE: OnceLock<Option<Event>> = OnceLock::new();

/// Udaje o vystupu (`mosdl_display` v hlavicce).
#[repr(C)]
pub struct MosdlDisplay {
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

/// Stav procesu (handly ze startovni zpravy vezme jednou).
pub(crate) fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let st = g.get_or_insert_with(|| {
        libos::rt::set_self_process(rt::start::self_process());
        let mut it = rt::start::take_handles().into_iter().map(Handle::from_raw);
        let link = it.next().map(|w| Link { window: Channel::from_handle(w), lease: it.next().map(Channel::from_handle) });
        // Dalsi handly SDL nepouziva: zavrou se.
        drop(it);
        State { link, win: None, pending: VecDeque::new(), clip_text: Vec::new(), cursor: None }
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

#[unsafe(no_mangle)]
pub extern "C" fn mosdl_available() -> i32 {
    with_state(|s| (s.link.is_some() || s.win.is_some()) as i32)
}

/// # Safety
///
/// `out` je platny ukazatel na `mosdl_display`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mosdl_display_info(out: *mut MosdlDisplay) -> i32 {
    if out.is_null() {
        return code(Error::InvalidArgs);
    }
    let d = with_state(|s| {
        let win = s.win.as_ref()?;
        let mut d = MosdlDisplay { screen_w: win.screen.0, screen_h: win.screen.1, work_w: win.screen.0, work_h: win.screen.1, scale_pct: 100, refresh_mhz: 0 };
        if let Some(c) = &win.context {
            d = MosdlDisplay { screen_w: c.screen.w, screen_h: c.screen.h, work_w: c.work_area.w, work_h: c.work_area.h, scale_pct: c.scale, refresh_mhz: c.refresh_mhz };
        }
        Some(d)
    });
    match d {
        // SAFETY: `out` overen vyse, platny podle smlouvy.
        Some(d) => unsafe {
            out.write(d);
            0
        },
        None => code(Error::BadState),
    }
}

/// # Safety
///
/// `title` je retezec C nebo NULL, `out_w`/`out_h` platne nebo NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mosdl_window_open(w: u32, h: u32, title: *const c_char, out_w: *mut u32, out_h: *mut u32) -> i32 {
    let title = c_str(title);
    let r: Result<(u32, u32)> = with_state(|s| {
        if s.win.is_some() {
            return Err(Error::NotSupported);
        }
        let link = s.link.take().ok_or(Error::NotFound)?;
        let mut win = Window::open(link, Mode::Standard, w.max(1), h.max(1), &title)?;
        if w > 0 && h > 0 && (win.w, win.h) != (w, h) {
            win.request_size(w, h, 0, 0)?;
        }
        let size = (win.w, win.h);
        s.win = Some(win);
        Ok(size)
    });
    match r {
        Ok((ww, wh)) => {
            // SAFETY: platne nebo NULL podle smlouvy.
            unsafe {
                if !out_w.is_null() {
                    out_w.write(ww);
                }
                if !out_h.is_null() {
                    out_h.write(wh);
                }
            }
            0
        }
        Err(e) => code(e),
    }
}

/// # Safety
///
/// `title` je retezec C nebo NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mosdl_window_set_title(title: *const c_char) -> i32 {
    let t = c_str(title);
    with_state(|s| match s.win.as_mut() {
        Some(w) => w.set_title(&t).and_then(|_| w.commit(false)).map_or_else(code, |_| 0),
        None => code(Error::BadState),
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn mosdl_window_set_size(w: u32, h: u32, min_w: u32, min_h: u32) -> i32 {
    with_state(|s| match s.win.as_mut() {
        Some(win) => win.request_size_min(w, h, min_w, min_h, 0, 0).map_or_else(code, |_| 0),
        None => code(Error::BadState),
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn mosdl_window_close() {
    // Zavreni kanalu = konec okna v kompozitoru (buffery se uvolni s nim).
    with_state(|s| s.win = None);
}

/// # Safety
///
/// `pixels` ukazuje na `pitch * (h - 1) + 4 * w` citelnych bajtu.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mosdl_window_present(pixels: *const u8, pitch: u32, w: u32, h: u32) -> i32 {
    if pixels.is_null() || w == 0 || h == 0 {
        return code(Error::InvalidArgs);
    }
    let Some(len) = (pitch as usize).checked_mul(h as usize - 1).and_then(|v| v.checked_add(w as usize * 4)) else {
        return code(Error::InvalidArgs);
    };
    // SAFETY: delka podle smlouvy funkce (volajici drzi snimek po dobu volani).
    let src = unsafe { core::slice::from_raw_parts(pixels, len) };
    with_state(|s| {
        let Some(win) = s.win.as_mut() else { return code(Error::BadState) };
        let same = (win.w, win.h) == (w, h);
        let mut ok = true;
        let r = win.draw(|cv| {
            let (dw, dh) = (cv.stride, cv.h);
            ok = copy_frame(cv.pixels_mut(), dw, dh, src, pitch, w, h);
        });
        // Snimek ve velikosti okna potvrdi cekajici zmenu velikosti.
        let r = r.and_then(|_| if same { win.commit_redrawn(false) } else { win.commit(false) });
        match r {
            Ok(()) if ok => 0,
            Ok(()) => code(Error::InvalidArgs),
            // Kompozitor spadl a okno je zpet: udalost EXPOSE ve fronte.
            Err(Error::ServiceRestarted) => 0,
            Err(e) => code(e),
        }
    })
}

/// Udalost okna -> udalost pro C (`None` = SDL ji nepotrebuje). Schranku
/// obslouzi okno samo (zadost o nase data); dorucena data vrati `Pasted`.
enum Item {
    Ev(MosdlEvent),
    Pasted(ui::client::Pasted),
}

fn translate(st: &mut State, e: WinEvent) -> Result<Option<Item>> {
    let win = st.win.as_mut().ok_or(Error::BadState)?;
    Ok(match e {
        WinEvent::Input(r) => Some(Item::Ev(from_input(&r))),
        WinEvent::Resized { w, h } => Some(Item::Ev(MosdlEvent::simple(kind::RESIZE, w as i32, h as i32))),
        WinEvent::Close => Some(Item::Ev(MosdlEvent::simple(kind::CLOSE, 0, 0))),
        WinEvent::Restored { .. } => {
            // Novy kompozitor kurzor nezna: poslat znovu.
            pointer::restore(st);
            Some(Item::Ev(MosdlEvent::simple(kind::EXPOSE, 0, 0)))
        }
        WinEvent::Context => Some(Item::Ev(MosdlEvent::simple(kind::DISPLAY, 0, 0))),
        WinEvent::PointerLock(on) => Some(Item::Ev(MosdlEvent::simple(kind::POINTER_LOCK, on as i32, 0))),
        WinEvent::State(s) => Some(Item::Ev(MosdlEvent::simple(kind::STATE, s as i32, 0))),
        WinEvent::Clipboard(m) => win.clipboard_event_any(m)?.map(Item::Pasted),
        _ => None,
    })
}

/// Dalsi polozka do `deadline` (`None` = vyprsel cas nebo probuzeni).
fn next_item(st: &mut State, deadline: u64) -> Result<Option<Item>> {
    loop {
        let win = st.win.as_mut().ok_or(Error::BadState)?;
        if win.has_queued() {
            if let Some(e) = win.next_one(Some(0))?
                && let Some(i) = translate(st, e)?
            {
                return Ok(Some(i));
            }
            continue;
        }
        let wake = wake_event();
        let (which, _) = win.with_channel(|ch| {
            let mut items = vec![(ch.handle(), Signals::READABLE | Signals::PEER_CLOSED)];
            if let Some(ev) = wake {
                items.push((ev.handle(), Signals::SIGNALED));
            }
            libos::wait_many(&items, Some(deadline))
        })??;
        if which == 1 {
            if let Some(ev) = wake {
                let _ = ev.reset();
            }
            return Ok(None);
        }
        match win.next_one(Some(0)) {
            Ok(Some(e)) => {
                if let Some(i) = translate(st, e)? {
                    return Ok(Some(i));
                }
            }
            Ok(None) | Err(Error::TimedOut | Error::ShouldWait) => {}
            Err(e) => return Err(e),
        }
        if libos::time::now() >= deadline {
            return Ok(None);
        }
    }
}

fn next_event(st: &mut State, deadline: u64) -> Result<Option<MosdlEvent>> {
    if let Some(e) = st.pending.pop_front() {
        return Ok(Some(e));
    }
    loop {
        match next_item(st, deadline)? {
            Some(Item::Ev(e)) => return Ok(Some(e)),
            // Data schranky mimo `mosdl_clipboard_fetch` (opozdena): zahodit.
            Some(Item::Pasted(_)) => {}
            None => return Ok(None),
        }
    }
}

/// # Safety
///
/// `ev` je platny ukazatel na `mosdl_event`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mosdl_next_event(ev: *mut MosdlEvent, timeout_ns: i64) -> i32 {
    if ev.is_null() {
        return code(Error::InvalidArgs);
    }
    let deadline = match timeout_ns {
        t if t < 0 => u64::MAX,
        t => libos::time::now().saturating_add(t as u64),
    };
    let r = with_state(|s| if s.win.is_some() { next_event(s, deadline) } else { Ok(None) });
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

#[unsafe(no_mangle)]
pub extern "C" fn mosdl_wake() -> i32 {
    match wake_event() {
        Some(e) => e.signal().map_or_else(code, |_| 0),
        None => code(Error::NoResources),
    }
}
