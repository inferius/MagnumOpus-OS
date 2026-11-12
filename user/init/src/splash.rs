//! Startovni obrazovka v initu (navrh `design/start.html`).
//!
//! Loader vykreslil prvni snimek (cerna a znak), jadro do framebufferu
//! nekreslilo (`bootstrap::flags::SPLASH`). Init obraz prevezme
//! (`FB_CONTROL CLAIM`) a ve vlastnim vlakne:
//!
//! - rozsviti teple pozadi (900 ms) a ukaze nazev,
//! - nechava dychat zari (perioda 2,4 s podle casu od startu systemu, takze
//!   kompozitor navaze bez skoku),
//! - plni prstenec po krocich startu ([`step`]): jadro, zarizeni, soubory,
//!   sit, plocha,
//! - kdyz se krok zdrzi pres 2 s, ukaze pod nazvem "ceka na: ...".
//!
//! Pred spustenim displeje kompozitoru (nebo zachranne konzole) se obraz
//! zastavi a framebuffer vrati bez smazani ([`finish`]); prstenec, na kterem
//! skoncil, dostane kompozitor (`display ... splash=<promile>`) a dokonci
//! prechod do plochy. Kdyz start selze, [`fail`] ukaze obrazovku chyby
//! s kodem misto zaseknuteho obrazu.
//!
//! Kresli se do kopie obrazovky v RAM a do framebufferu se kopiruji jen
//! zmenene obdelniky (rozsviceni: cela obrazovka ~1 s, pak jen znak:
//! ~0,3 MB na snimek). Vlakno nic nenacita a na start nikdo neceka.

use alloc::string::String;
use alloc::vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use core::time::Duration;

use bootsplash::{Rect, STEPS, Splash, State, timing};
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Mapping, Startup, Vmo, println, time};

/// Hotove kroky startu (0..=STEPS.len()).
static STEPS_DONE: AtomicU32 = AtomicU32::new(0);
/// Kdy se pocet kroku naposledy zmenil (ns od startu).
static STEP_AT: AtomicU64 = AtomicU64::new(0);
/// Zastavit obraz (predani displeji).
static STOP: AtomicBool = AtomicBool::new(false);
/// Prstenec v okamziku predani (promile) pro kompozitor; `u32::MAX` = obraz
/// nebezel nebo uz byl predan.
static HANDOFF: AtomicU32 = AtomicU32::new(u32::MAX);
/// Zprava chyby startu (nastavi [`fail`]).
static FAILED: spin::Mutex<Option<String>> = spin::Mutex::new(None);
static THREAD: spin::Mutex<Option<(libos::thread::JoinHandle, Vmo)>> = spin::Mutex::new(None);

/// Doba snimku: rozsviceni 30 fps (cela obrazovka), dychani 25 fps.
const FRAME_FADE: Duration = Duration::from_millis(33);
const FRAME_IDLE: Duration = Duration::from_millis(40);

/// Prevezme startovni obraz, kdyz ho loader vykreslil (bezny beh, ne
/// testy). Krok "jadro" je tim hotovy.
pub fn start(startup: &Startup) {
    use libos::abi::bootstrap::flags;
    if startup.flags & flags::SPLASH == 0 || startup.flags & (flags::TEST_RUN | flags::RESCUE_LOGS) != 0 {
        return;
    }
    let Some(fb) = startup.framebuffer.as_ref() else { return };
    match open(fb) {
        Ok((screen, vmo)) => {
            STEPS_DONE.store(1, Ordering::Relaxed);
            STEP_AT.store(time::now(), Ordering::Relaxed);
            HANDOFF.store(0, Ordering::Relaxed);
            match libos::thread::spawn("start-obraz", move || run(screen)) {
                Ok(h) => *THREAD.lock() = Some((h, vmo)),
                Err(e) => {
                    println!("init: vlakno startovniho obrazu nelze spustit: {e}");
                    let _ = libos::sys::fb_claim(vmo.handle().raw(), false);
                    HANDOFF.store(u32::MAX, Ordering::Relaxed);
                }
            }
        }
        Err(e) => println!("init: startovni obraz nelze prevzit: {e}"),
    }
}

/// Hotovo `n` kroku z [`STEPS`] (prstenec se plni jen dopredu).
pub fn step(n: usize) {
    let n = n.min(STEPS.len()) as u32;
    if STEPS_DONE.fetch_max(n, Ordering::Relaxed) < n {
        STEP_AT.store(time::now(), Ordering::Relaxed);
    }
}

/// Zastavi obraz a vrati framebuffer (bez smazani: posledni snimek zustane
/// do prvniho snimku displeje). Vraci prstenec pro kompozitor (0..=1000),
/// `None` = obraz nebezel. Dalsi volani nic nedela.
pub fn finish() -> Option<u32> {
    let (h, vmo) = THREAD.lock().take()?;
    STOP.store(true, Ordering::Relaxed);
    let _ = h.join(None);
    // Vlastnikem je init (CLAIM ve vlakne pres stejny objekt).
    if let Err(e) = libos::sys::fb_claim(vmo.handle().raw(), false) {
        println!("init: vraceni framebufferu po startovnim obrazu: {e}");
    }
    let p = HANDOFF.swap(u32::MAX, Ordering::Relaxed);
    (p != u32::MAX).then_some(p)
}

/// Start selhal: misto znaku obrazovka chyby (nadpis a duvod s kodem).
/// Framebuffer zustava initu, nikdo ho neprevezme.
pub fn fail(reason: &str) {
    *FAILED.lock() = Some(String::from(reason));
    if let Some((h, _vmo)) = THREAD.lock().take() {
        let _ = h.join(None);
        // Objekt framebufferu se zavre, ale obraz (a CLAIM) zustava.
    }
}

/// Framebuffer namapovany pro zapis.
struct Screen {
    map: Mapping,
    w: u32,
    h: u32,
    stride: u32,
    rgb: bool,
}

fn open(fb: &Vmo) -> libos::Result<(Screen, Vmo)> {
    let vmo = Vmo::from_handle(fb.handle().duplicate(None)?);
    let i = libos::sys::fb_info(vmo.handle().raw())?;
    if i.width == 0 || i.height == 0 || i.stride < i.width || !(1..=2).contains(&i.format) {
        return Err(libos::Error::NotSupported);
    }
    let bytes = i.stride as u64 * i.height as u64 * 4;
    let map = Mapping::new(&vmo, 0, bytes, MAP_READ | MAP_WRITE)?;
    libos::sys::fb_claim(vmo.handle().raw(), true)?;
    Ok((Screen { map, w: i.width, h: i.height, stride: i.stride, rgb: i.format == 1 }, vmo))
}

impl Screen {
    /// Zkopiruje obdelnik z kopie obrazovky do framebufferu.
    fn copy(&self, shadow: &[u32], r: Rect) {
        let r = r.clip(self.w, self.h);
        // SAFETY: mapovani pokryva stride * h pixelu (viz `open`) a zije
        // stejne dlouho jako `self`; framebuffer drzi jen toto vlakno.
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

fn run(screen: Screen) {
    // Na obraz start neceka: vlakno ustoupi praci initu a sluzeb.
    let _ = libos::sched::set_current(libos::sched::Qos::Background);
    let t0 = time::now();
    let mut s = Splash::new(screen.w, screen.h);
    let l = s.layout;
    let mut shadow = vec![0u32; (screen.w * screen.h) as usize];
    let theme = theme::Theme::builtin(theme::Mode::Light);
    let mut fonts = gfx::text::Family::new(font::inter(), &[]);
    let full = Rect { x: 0, y: 0, w: screen.w, h: screen.h };
    let mark = l.mark_rect(1.0, 0.0, 0.0);
    let stall_r = l.stall_rect();
    // Prstenec: animace z `from` na cil od `at`.
    let (mut ring_from, mut ring_to, mut ring_at) = (0.0f32, 0.0f32, t0);
    let mut cur = 0.0f32;
    let mut stall_shown: Option<usize> = None;
    let (mut frames, mut busy) = (0u64, 0u64);
    let fade_end = timing::WORD_DELAY + timing::WORD_FADE;
    loop {
        if let Some(reason) = FAILED.lock().clone() {
            show_failure(&screen, &mut s, &mut shadow, &mut fonts, &theme, &reason);
            return;
        }
        let stop = STOP.load(Ordering::Relaxed);
        let now = time::now();
        let t = ms(now - t0);
        let target = STEPS_DONE.load(Ordering::Relaxed) as f32 / STEPS.len() as f32;
        if target != ring_to {
            ring_from = cur;
            ring_to = target;
            ring_at = now;
        }
        let k = (ms(now - ring_at) as f32 / timing::RING as f32).clamp(0.0, 1.0);
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
        let done = STEPS_DONE.load(Ordering::Relaxed) as usize;
        let waiting = (done < STEPS.len() && ms(now - STEP_AT.load(Ordering::Relaxed)) >= timing::STALL_AFTER && !stop).then_some(done);
        let fading = t <= fade_end.max(timing::WALL_FADE) + 50 || frames == 0;
        let started = time::now();
        let mut dirty = if fading { full } else { mark };
        s.paint(&st, &mut shadow, screen.w as usize, dirty);
        if waiting != stall_shown || (fading && waiting.is_some()) {
            if !fading {
                s.paint(&st, &mut shadow, screen.w as usize, stall_r);
            }
            if let Some(i) = waiting {
                draw_stall(&mut shadow, &screen, &l, &mut fonts, &theme, STEPS[i]);
            }
            stall_shown = waiting;
            dirty = dirty.union(stall_r);
        }
        screen.copy(&shadow, dirty);
        frames += 1;
        busy += time::now() - started;
        if stop {
            HANDOFF.store((cur * 1000.0) as u32, Ordering::Relaxed);
            println!(
                "init: startovni obraz predan po {} ms, {frames} snimku, prumer {} us na snimek",
                ms(now - t0),
                busy / frames.max(1) / 1000
            );
            return;
        }
        time::sleep(if fading { FRAME_FADE } else { FRAME_IDLE });
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
/// a duvod s kodem uprostred. Kresli se jednou.
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
    println!("init: start selhal: {reason}");
}
