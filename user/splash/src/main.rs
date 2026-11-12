//! splash: animace startovni obrazovky (navrh `design/start.html`).
//!
//! Loader vykreslil prvni snimek (cerna a znak), jadro do framebufferu
//! nekreslilo (`bootstrap::flags::SPLASH`). Init spusti tento proces
//! z initfs a preda mu objekt pameti framebufferu a ridici kanal (kind
//! USER, v tomto poradi). Proces obraz prevezme (`FB_CONTROL CLAIM`) a:
//!
//! - rozsviti teple pozadi (900 ms) a ukaze nazev,
//! - nechava dychat zari (perioda 2,4 s podle casu od startu systemu, takze
//!   kompozitor navaze bez skoku),
//! - plni prstenec po krocich startu, kdyz se krok zdrzi pres 2 s, ukaze
//!   pod nazvem "ceka na: ...".
//!
//! Samostatny proces (ne vlakno initu): chyba v kresleni shodi jen jeho,
//! init ho ukonci a start pokracuje.
//!
//! Ridici kanal (text, init -> splash):
//!
//! | Zprava | Vyznam | Odpoved |
//! |---|---|---|
//! | `krok <n>` | hotovo `n` z `bootsplash::STEPS` | zadna |
//! | `konec` | dokreslit plny prstenec a skoncit | `hotovo <promile>` |
//! | `chyba <text>` | obrazovka chyby startu s duvodem, pak konec | `ok` |
//!
//! Konec procesu framebuffer vrati bez smazani (jadro v rezimu SPLASH
//! obraz nemaze), dalsi vlastnik (displej kompozitoru) ho prevezme tak, jak
//! je. Zavreny kanal = konec bez dalsiho kresleni.
//!
//! Kresli se do kopie obrazovky v RAM a do framebufferu se kopiruji jen
//! zmenene obdelniky (rozsviceni: cela obrazovka ~1 s, pak jen znak).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec;

use bootsplash::{Rect, STEPS, Splash, State, timing};
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Channel, Error, Mapping, Signals, Startup, Vmo, println, time};

libos::entry!(main);

/// Doba snimku: rozsviceni 30 fps (cela obrazovka), dychani 25 fps.
const FRAME_FADE_NS: u64 = 33_000_000;
const FRAME_IDLE_NS: u64 = 40_000_000;
/// Dobeh prstence do plna pri `konec`.
const FINISH_MS: u64 = 200;

/// Framebuffer namapovany pro zapis.
struct Screen {
    map: Mapping,
    vmo: Vmo,
    w: u32,
    h: u32,
    stride: u32,
    rgb: bool,
}

impl Screen {
    fn open(vmo: Vmo) -> libos::Result<Self> {
        let i = libos::sys::fb_info(vmo.handle().raw())?;
        if i.width == 0 || i.height == 0 || i.stride < i.width || !(1..=2).contains(&i.format) {
            return Err(Error::NotSupported);
        }
        let bytes = i.stride as u64 * i.height as u64 * 4;
        let map = Mapping::new(&vmo, 0, bytes, MAP_READ | MAP_WRITE)?;
        libos::sys::fb_claim(vmo.handle().raw(), true)?;
        Ok(Self { map, vmo, w: i.width, h: i.height, stride: i.stride, rgb: i.format == 1 })
    }

    /// Zkopiruje obdelnik z kopie obrazovky do framebufferu.
    fn copy(&self, shadow: &[u32], r: Rect) {
        let r = r.clip(self.w, self.h);
        // SAFETY: mapovani pokryva stride * h pixelu (viz `open`) a zije
        // stejne dlouho jako `self`; framebuffer drzi jen tento proces.
        let fb = unsafe { core::slice::from_raw_parts_mut(self.map.addr() as *mut u32, (self.stride * self.h) as usize) };
        for y in r.y..r.bottom() {
            let src = &shadow[(y as u32 * self.w) as usize..][r.x as usize..r.right() as usize];
            let dst = &mut fb[(y as u32 * self.stride) as usize..][r.x as usize..r.right() as usize];
            if self.rgb {
                for (d, &s) in dst.iter_mut().zip(src) {
                    *d = (s & 0xFF00_FF00) | (s >> 16 & 0xFF) | (s & 0xFF) << 16;
                }
            } else {
                dst.copy_from_slice(src);
            }
        }
    }
}

fn ms(ns: u64) -> u64 {
    ns / 1_000_000
}

/// Co prislo od initu.
enum Cmd {
    Step(u32),
    Finish,
    Fail(String),
    Closed,
}

fn read_cmd(ch: &Channel) -> Option<Cmd> {
    match ch.try_read() {
        Ok(m) => {
            let text = String::from_utf8_lossy(&m.data);
            if let Some(n) = text.strip_prefix("krok ") {
                Some(Cmd::Step(n.trim().parse().unwrap_or(0)))
            } else if text.trim() == "konec" {
                Some(Cmd::Finish)
            } else if let Some(r) = text.strip_prefix("chyba ") {
                Some(Cmd::Fail(String::from(r)))
            } else {
                println!("splash: neznamy prikaz {text:?}");
                None
            }
        }
        Err(Error::ShouldWait) => None,
        Err(_) => Some(Cmd::Closed),
    }
}

fn main(startup: Startup) -> i64 {
    // Na obraz start neceka: proces ustoupi praci initu a sluzeb.
    let _ = libos::sched::set_current(libos::sched::Qos::Background);
    let mut handles = startup.handles.into_iter();
    let (Some(fb), Some(ctl)) = (handles.next(), handles.next()) else {
        println!("splash: chybi handly ve startovni zprave");
        return 1;
    };
    let ctl = Channel::from_handle(ctl);
    let screen = match Screen::open(Vmo::from_handle(fb)) {
        Ok(s) => s,
        Err(e) => {
            println!("splash: framebuffer nelze prevzit: {e}");
            return 2;
        }
    };
    run(&screen, &ctl);
    // Konec procesu vrati framebuffer; obraz zustava.
    drop(screen.vmo);
    0
}

fn run(screen: &Screen, ctl: &Channel) {
    let t0 = time::now();
    let mut s = Splash::new(screen.w, screen.h);
    let l = s.layout;
    let mut shadow = vec![0u32; (screen.w * screen.h) as usize];
    let theme = theme::Theme::builtin(theme::Mode::Light);
    let mut fonts = gfx::text::Family::new(font::inter(), &[]);
    let full = Rect { x: 0, y: 0, w: screen.w, h: screen.h };
    let mark = l.mark_rect(1.0, 0.0, 0.0);
    let stall_r = l.stall_rect();
    // Krok "jadro" je hotovy uz tim, ze bezi init.
    let (mut done, mut step_at) = (1u32, t0);
    // Prstenec: animace z `from` na cil od `at` (delka `len` ms).
    let (mut ring_from, mut ring_to, mut ring_at, mut ring_len) = (0.0f32, 0.0f32, t0, timing::RING);
    let mut cur = 0.0f32;
    let mut stall_shown: Option<usize> = None;
    let (mut frames, mut busy) = (0u64, 0u64);
    let mut finishing: Option<u64> = None;
    let fade_end = (timing::WORD_DELAY + timing::WORD_FADE).max(timing::WALL_FADE);
    loop {
        // Prikazy initu.
        while let Some(cmd) = read_cmd(ctl) {
            match cmd {
                Cmd::Step(n) => {
                    let n = n.min(STEPS.len() as u32);
                    if n > done {
                        done = n;
                        step_at = time::now();
                    }
                }
                Cmd::Finish => {
                    if finishing.is_none() {
                        done = STEPS.len() as u32;
                        finishing = Some(time::now());
                    }
                }
                Cmd::Fail(reason) => {
                    show_failure(screen, &mut s, &mut shadow, &mut fonts, &theme, &reason);
                    let _ = ctl.write(b"ok", alloc::vec::Vec::new());
                    return;
                }
                Cmd::Closed => return,
            }
        }
        let now = time::now();
        let t = ms(now - t0);
        let target = done as f32 / STEPS.len() as f32;
        if target != ring_to {
            ring_from = cur;
            ring_to = target;
            ring_at = now;
            ring_len = if finishing.is_some() { FINISH_MS } else { timing::RING };
        }
        let k = (ms(now - ring_at) as f32 / ring_len as f32).clamp(0.0, 1.0);
        cur = ring_from + (ring_to - ring_from) * bootsplash::ease(k);
        let (glow, glow_scale) = bootsplash::breathe(ms(now));
        let st = State {
            wall: bootsplash::ease(t as f32 / timing::WALL_FADE as f32),
            word: bootsplash::ease(t.saturating_sub(timing::WORD_DELAY) as f32 / timing::WORD_FADE as f32),
            progress: cur,
            glow: glow * (t as f32 / 600.0).min(1.0),
            glow_scale,
            ..State::FIRST
        };
        // Zdrzeny krok: "ceka na: <dalsi krok>".
        let waiting = ((done as usize) < STEPS.len() && ms(now - step_at) >= timing::STALL_AFTER).then_some(done as usize);
        let fading = t <= fade_end + 50 || frames == 0;
        let started = time::now();
        let mut dirty = if fading { full } else { mark };
        s.paint(&st, &mut shadow, screen.w as usize, dirty);
        if waiting != stall_shown || (fading && waiting.is_some()) {
            if !fading {
                s.paint(&st, &mut shadow, screen.w as usize, stall_r);
            }
            if let Some(i) = waiting {
                draw_stall(&mut shadow, screen, &l, &mut fonts, &theme, STEPS[i]);
            }
            stall_shown = waiting;
            dirty = dirty.union(stall_r);
        }
        screen.copy(&shadow, dirty);
        frames += 1;
        busy += time::now() - started;
        // Konec: prstenec doběhl do plna (posledni snimek je na obrazovce).
        if let Some(f) = finishing
            && cur >= 1.0
        {
            println!(
                "splash: obraz predan po {} ms (konec trval {} ms), {frames} snimku, prumer {} us na snimek",
                ms(now - t0),
                ms(now - f),
                busy / frames.max(1) / 1000
            );
            let _ = ctl.write(b"hotovo 1000", alloc::vec::Vec::new());
            return;
        }
        let period = if fading || finishing.is_some() { FRAME_FADE_NS } else { FRAME_IDLE_NS };
        let _ = libos::wait_one(ctl.handle(), Signals::READABLE | Signals::PEER_CLOSED, Some(started + period));
    }
}

/// Radek "ceka na: ..." pod nazvem (tlumena barva, maly text).
fn draw_stall(shadow: &mut [u32], screen: &Screen, l: &bootsplash::Layout, fonts: &mut gfx::text::Family, theme: &theme::Theme, step: &str) {
    let Some(mut c) = gfx::Canvas::new(shadow, screen.w, screen.h, screen.w) else { return };
    let ty = theme.text.small;
    let px = ty.size * l.s;
    let text = alloc::format!("čeká na: {step}");
    let face = fonts.face(ty.weight);
    let w = face.width(&text, px) as i32;
    let line = (ty.line as f32 * l.s) as u32;
    let base = l.stall_top as i32 + face.baseline_in(px, line);
    face.draw(&mut c, l.cx as i32 - w / 2, base, &text, px, theme.color.ink_muted);
}

/// Obrazovka chyby startu (`.err` v prototypu): teple pozadi, nadpis
/// a duvod s kodem uprostred.
fn show_failure(screen: &Screen, s: &mut Splash, shadow: &mut [u32], fonts: &mut gfx::text::Family, theme: &theme::Theme, reason: &str) {
    let st = State { mark_opacity: 0.0, word: 0.0, ..State::lit(0.0) };
    s.paint_all(&st, shadow, screen.w as usize);
    let l = s.layout;
    if let Some(mut c) = gfx::Canvas::new(shadow, screen.w, screen.h, screen.w) {
        let rows: [(&str, theme::TextStyle, u32); 3] = [
            ("Systém se nespustil", theme.text.title, theme.color.ink),
            (reason, theme.text.small, theme.color.ink_muted),
            ("Podrobnosti jsou v logu na sériové lince.", theme.text.small, theme.color.ink_muted),
        ];
        let mut y = l.cy as i32 - (40.0 * l.s) as i32;
        for (text, ty, color) in rows {
            let px = ty.size * l.s;
            let line = (ty.line as f32 * l.s) as u32;
            let face = fonts.face(ty.weight);
            let w = face.width(text, px) as i32;
            let base = y + face.baseline_in(px, line);
            face.draw(&mut c, l.cx as i32 - w / 2, base, text, px, color);
            y += line as i32 + (8.0 * l.s) as i32;
        }
    }
    screen.copy(shadow, Rect { x: 0, y: 0, w: screen.w, h: screen.h });
    println!("splash: start selhal: {reason}");
}
