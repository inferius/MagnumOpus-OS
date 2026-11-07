//! console: textova a zachranna konzole (beh bez GUI, pad GUI, nouzovy
//! rezim). Kresli interaktivni terminal (`term::ui`) do framebufferu
//! z firmware a bere klavesy od `inputd`.
//!
//! Start: bootstrap kanal = ridici kanal (od initu, pozdeji supervisor);
//! handly `kind::USER` = [kanal k termd (okno)].
//!
//! Displej ani fokus si konzole nebere sama - preda ji je ten, kdo je
//! vlastni (supervisor), stejne jako kompozitoru. Ridici prikazy (text,
//! odpoved `ok` / `ok <data>` / `err <KOD>` jako u inputd):
//!
//! | Prikaz | Handly | Co |
//! |---|---|---|
//! | `display` | framebuffer (VMO) | prevzit displej (jadro prestane kreslit log), kreslit |
//! | `focus` | kanal udalosti od inputd | brat klavesy |
//! | `release` | - | prestat kreslit, vratit displej jadru; odpoved nese VMO framebufferu, kanal fokusu se zavre |
//! | `status` | - | `ok drawing=0/1 focus=0/1 session=N connected=0/1` |
//! | `dump` | - | `ok` + text obrazovky po radcich (testy, diagnostika) |

#![no_std]
#![no_main]

extern crate alloc;

mod keys;

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use libos::Mapping;
use libos::{Channel, Error, Handle, Signals, Startup, Vmo, println, time};
use term::proto::ServerMsg;
use term::screen::{PixelFormat, Screen, Surface, cell_size, font_for, paint};
use term::theme::Theme;
use term::ui::App;

libos::entry!(main);

/// Prekreslit nejvys jednou za tolik (animace pruhu a tociciho znaku).
const TICK: Duration = Duration::from_millis(150);

/// Displej: framebuffer, mapovani a obraz v pameti (do framebufferu se
/// kopiruje cely snimek najednou - zapis po pixelech do pameti zarizeni je
/// pomaly).
struct Display {
    vmo: Vmo,
    map: Mapping,
    width: usize,
    height: usize,
    stride: usize,
    format: PixelFormat,
    back: Vec<u32>,
}

impl Display {
    fn open(vmo: Vmo) -> Result<Display, Error> {
        let info = libos::sys::fb_info(vmo.handle().raw())?;
        let format = match info.format {
            1 => PixelFormat::Rgb,
            2 => PixelFormat::Bgr,
            _ => return Err(Error::NotSupported),
        };
        let bytes = info.stride as u64 * info.height as u64 * 4;
        let map = Mapping::new(&vmo, 0, bytes.next_multiple_of(libos::vm::page_size()), abi_rw())?;
        libos::sys::fb_claim(vmo.handle().raw(), true)?;
        let (w, h, s) = (info.width as usize, info.height as usize, info.stride as usize);
        Ok(Display { vmo, map, width: w, height: h, stride: s, format, back: alloc::vec![0; s * h] })
    }

    fn grid(&self) -> (usize, usize) {
        let (cw, ch) = cell_size(font_for(self.height));
        ((self.width.saturating_sub(16)) / cw, (self.height.saturating_sub(16)) / ch)
    }

    fn show(&mut self, screen: &Screen) {
        let mut surf =
            Surface { buf: &mut self.back, width: self.width, height: self.height, stride: self.stride, format: self.format };
        paint(screen, &mut surf);
        // SAFETY: `back` je souvisle pole u32; bajty jen ctu.
        let bytes = unsafe { core::slice::from_raw_parts(self.back.as_ptr() as *const u8, self.back.len() * 4) };
        self.map.write_bytes(0, bytes);
    }

    /// Vratit displej jadru; vrati VMO (pro dalsiho vlastnika).
    fn release(self) -> Vmo {
        let _ = libos::sys::fb_claim(self.vmo.handle().raw(), false);
        self.vmo
    }
}

fn abi_rw() -> u32 {
    libos::abi::vm::MAP_READ | libos::abi::vm::MAP_WRITE
}

struct Console {
    app: App,
    screen: Screen,
    display: Option<Display>,
    focus: Option<Channel>,
    termd: Channel,
    dirty: bool,
}

impl Console {
    fn send_out(&mut self) {
        for m in self.app.take_out() {
            if self.termd.write(&m.encode(), Vec::new()).is_err() {
                self.app.connected = false;
            }
        }
    }

    fn redraw(&mut self) {
        self.screen.write(&self.app.frame());
        if let Some(d) = self.display.as_mut() {
            d.show(&self.screen);
        }
        self.dirty = false;
    }

    fn dump(&self) -> String {
        let mut s = String::new();
        for y in 0..self.screen.rows {
            s.push_str(&self.screen.line(y));
            s.push('\n');
        }
        s
    }

    /// Ridici prikaz; vrati odpoved a handly k ni.
    fn control(&mut self, text: &str, mut handles: Vec<Handle>) -> (String, Vec<Handle>) {
        let cmd = text.split_whitespace().next().unwrap_or("");
        match cmd {
            "display" => {
                let Some(h) = handles.pop() else { return ("err INVALID_ARGS".into(), Vec::new()) };
                if self.display.is_some() {
                    return ("err ALREADY_EXISTS".into(), Vec::new());
                }
                match Display::open(Vmo::from_handle(h)) {
                    Ok(d) => {
                        let (cols, rows) = d.grid();
                        self.screen = Screen::new(cols, rows);
                        self.app.resize(cols, rows);
                        self.display = Some(d);
                        self.dirty = true;
                        println!("console: displej prevzat ({cols}x{rows} znaku)");
                        ("ok".into(), Vec::new())
                    }
                    Err(e) => (alloc::format!("err {}", e.name()), Vec::new()),
                }
            }
            "focus" => match handles.pop() {
                Some(h) => {
                    self.focus = Some(Channel::from_handle(h));
                    ("ok".into(), Vec::new())
                }
                None => ("err INVALID_ARGS".into(), Vec::new()),
            },
            "release" => {
                self.focus = None;
                match self.display.take() {
                    Some(d) => {
                        println!("console: displej vracen");
                        ("ok".into(), alloc::vec![d.release().into_handle()])
                    }
                    None => ("ok".into(), Vec::new()),
                }
            }
            "status" => (
                alloc::format!(
                    "ok drawing={} focus={} session={} connected={}",
                    self.display.is_some() as u8,
                    self.focus.is_some() as u8,
                    self.app.session,
                    self.app.connected as u8
                ),
                Vec::new(),
            ),
            "dump" => (alloc::format!("ok\n{}", self.dump()), Vec::new()),
            _ => ("err INVALID_ARGS".into(), Vec::new()),
        }
    }
}

fn now() -> Option<i64> {
    time::wall().map(|t| t.secs.saturating_mul(1_000_000_000).saturating_add(t.nanos as i64))
}

fn main(mut startup: Startup) -> i64 {
    let control = startup.bootstrap;
    let Some(h) = startup.handles.drain(..).next() else {
        println!("console: chybi kanal k termd");
        return 1;
    };
    let termd = Channel::from_handle(h);
    // Bez displeje (pred `display`) se kresli jen do obrazovky v pameti.
    let (cols, rows) = (100, 30);
    let mut c = Console {
        app: App::new(Theme::dark(), cols, rows),
        screen: Screen::new(cols, rows),
        display: None,
        focus: None,
        termd,
        dirty: true,
    };
    c.app.start(0);
    c.send_out();
    println!("console: bezim (ceka na displej a fokus)");
    let mut next_tick = time::deadline_after(TICK);
    loop {
        let mask = Signals::READABLE | Signals::PEER_CLOSED;
        let mut items: Vec<(&Handle, Signals)> = alloc::vec![(control.handle(), mask), (c.termd.handle(), mask)];
        if let Some(f) = &c.focus {
            items.push((f.handle(), mask));
        }
        let _ = libos::wait_many(&items, Some(next_tick));
        // Ridici kanal.
        loop {
            match control.try_read() {
                Ok(m) => {
                    let text = String::from_utf8_lossy(&m.data).into_owned();
                    let (reply, hs) = c.control(&text, m.handles);
                    let _ = control.write(reply.as_bytes(), hs);
                }
                Err(Error::ShouldWait) => break,
                Err(_) => {
                    // Kdo konzoli ridil, skoncil: displej vratit jadru, konec.
                    if let Some(d) = c.display.take() {
                        d.release();
                    }
                    println!("console: ridici kanal zavren, koncim");
                    return 0;
                }
            }
        }
        // Zpravy od termd.
        loop {
            match c.termd.try_read() {
                Ok(m) => {
                    if let Ok(sm) = ServerMsg::decode(&m.data) {
                        c.app.on_msg(sm);
                        c.dirty = true;
                    }
                }
                Err(Error::ShouldWait) => break,
                Err(_) => {
                    if c.app.connected {
                        c.app.connected = false;
                        c.dirty = true;
                    }
                    break;
                }
            }
        }
        // Klavesy.
        let mut lost = false;
        if let Some(f) = &c.focus {
            loop {
                match f.try_read() {
                    Ok(m) => {
                        for r in inputproto::event::decode(&m.data).unwrap_or_default() {
                            if let Some(k) = keys::key(&r) {
                                c.app.key(k);
                                c.dirty = true;
                            }
                        }
                    }
                    Err(Error::ShouldWait) => break,
                    Err(_) => {
                        lost = true;
                        break;
                    }
                }
            }
        }
        if lost {
            // inputd dal fokus nekomu jinemu.
            c.focus = None;
        }
        c.send_out();
        if time::now() >= next_tick {
            next_tick = time::deadline_after(TICK);
            c.app.tick(now());
            if c.app.busy() {
                c.dirty = true;
            }
        }
        if c.dirty {
            c.redraw();
        }
    }
}
