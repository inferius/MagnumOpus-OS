//! Prechod ze startovni obrazovky do plochy (`design/start.html`).
//!
//! Init preda kompozitoru prstenec, na kterem startovni obraz skoncil
//! (`display <pasmo> splash=<promile>`). Prvni snimek po pripojeni
//! displeje je tentyz obraz (`libs/bootsplash`), takze se nic nebliká,
//! a pak:
//!
//! 1. prstenec dobehne do plna (500 ms; proces `splash` ho obvykle preda
//!    uz plny) a 200 ms se drzi,
//! 2. znak se zmensi na velikost avataru a vjede do nej (600 ms, nazev
//!    a zare zmizi za 200 ms),
//! 3. znak i startovni pozadi se rozplynou (300 ms) do plochy, kterou
//!    kompozitor slozil pod nimi.
//!
//! Plocha se behem prechodu sklada dal jako obvykle (klienti dostavaji
//! snimky, start aplikaci se nezdrzi), jen do vlastniho bufferu v RAM
//! misto do snimku displeje. Displej dostava obraz: v krocich 1 a 2 jen
//! oblast znaku, v kroku 3 prolnuti bufferu plochy s obrazem. Na konci se
//! buffer zkopiruje do snimku a kompozitor pokracuje normalne. Prechod
//! trva ~1,6 s.
//!
//! Cil prechodu: avatar prihlasovaci obrazovky ([`Service::set_splash_target`],
//! sdilene misto s vlaknem uctu); bez cile se znak zmensi na velikost
//! avataru (88 px) na miste a rozplyne se.

use alloc::vec::Vec;

use bootsplash::{Rect, Splash, State, timing};

use libos::println;

use crate::Service;

/// Dobeh prstence a podrzeni pred presunem (ms).
const RING_FILL: u64 = 500;
const HOLD: u64 = 200;
/// Zmizeni nazvu a zare na zacatku presunu.
const WORD_OUT: u64 = 200;
/// Prumer avataru prihlasovaci obrazovky v logickych px (`.av`).
const AVATAR: f32 = 88.0;
/// Takt prechodu (30 fps: kompozitor mezitim obsluhuje klienty).
const FRAME_NS: u64 = 33_000_000;

/// Faze prechodu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Prstenec a presun: kresli se jen obraz (bez plochy).
    Splash,
    /// Rozplynuti do plochy.
    Fade,
    Done,
}

/// Bezici prechod.
pub struct Handoff {
    splash: Splash,
    /// Prstenec od initu (0..1).
    p0: f32,
    /// Zacatek (prvni snimek), ns.
    t0: u64,
    /// Cil: stred a prumer avataru v px.
    target: Option<(f32, f32, f32)>,
    /// Plocha pod obrazem: kompozitor sem sklada po celou dobu prechodu.
    under: Vec<u32>,
    /// Posledni snimek prechodu (ns).
    last: u64,
    /// Oblast znaku v predchozim snimku (prekreslit).
    last_mark: Rect,
    /// Omezeny pohyb: jen kratke prolnuti.
    reduced: bool,
}

impl Handoff {
    pub fn new(w: u32, h: u32, permille: u32) -> Self {
        Self {
            splash: Splash::new(w, h),
            p0: (permille.min(1000) as f32) / 1000.0,
            t0: 0,
            target: None,
            under: alloc::vec![0; (w * h) as usize],
            last: 0,
            last_mark: Rect::default(),
            reduced: false,
        }
    }

    fn ms(&self, now: u64) -> u64 {
        now.saturating_sub(self.t0) / 1_000_000
    }

    /// Zacatek presunu: prstenec dobehne (kdyz uz neni plny) a drzi se.
    fn move_at(&self) -> u64 {
        if self.reduced {
            0
        } else if self.p0 >= 1.0 {
            HOLD
        } else {
            RING_FILL + HOLD
        }
    }

    fn fade_at(&self) -> u64 {
        if self.reduced { 0 } else { self.move_at() + timing::HANDOFF_MOVE }
    }

    fn fade_len(&self) -> u64 {
        if self.reduced { timing::HANDOFF_REDUCED } else { timing::HANDOFF_FADE }
    }

    fn phase(&self, now: u64) -> Phase {
        let t = self.ms(now);
        if t < self.fade_at() {
            Phase::Splash
        } else if t < self.fade_at() + self.fade_len() {
            Phase::Fade
        } else {
            Phase::Done
        }
    }

    /// Stav obrazu v case `now`.
    fn state(&self, now: u64) -> State {
        let l = self.splash.layout;
        let t = self.ms(now);
        let k = |from: u64, len: u64| bootsplash::ease(t.saturating_sub(from) as f32 / len.max(1) as f32);
        let ring = self.p0 + (1.0 - self.p0) * k(0, RING_FILL);
        let (glow, glow_scale) = bootsplash::breathe(now / 1_000_000);
        let out = 1.0 - k(self.move_at(), WORD_OUT);
        // Cil: avatar (stred a prumer); bez nej avatar na miste znaku.
        let (tx, ty, td) = self.target.unwrap_or((l.cx, l.cy, AVATAR * l.s));
        let scale_to = (td / 2.0) / (bootsplash::size::RING_R * l.s);
        let m = if self.reduced { 0.0 } else { k(self.move_at(), timing::HANDOFF_MOVE) };
        let f = k(self.fade_at(), self.fade_len());
        State {
            wall: 1.0,
            backdrop: 1.0 - f,
            word: out,
            progress: ring,
            glow: glow * out,
            glow_scale,
            mark_scale: 1.0 + (scale_to - 1.0) * m,
            mark_dx: (tx - l.cx) * m,
            mark_dy: (ty - l.cy) * m,
            mark_opacity: 1.0 - f,
        }
    }

    /// Prvni snimek (pred `ATTACH`): obraz s prstencem od initu.
    pub fn first_frame(&mut self, px: &mut [u32], now: u64) {
        self.t0 = now;
        let st = self.state(now);
        let w = self.splash.layout.w as usize;
        self.splash.paint_all(&st, px, w);
        self.last_mark = self.splash.layout.mark_rect(1.0, 0.0, 0.0);
    }
}

/// Obdelnik pres celou obrazovku.
fn full(l: &bootsplash::Layout) -> Rect {
    Rect { x: 0, y: 0, w: l.w, h: l.h }
}

fn to_gui(r: Rect) -> guiproto::Rect {
    guiproto::Rect { x: r.x, y: r.y, w: r.w, h: r.h }
}

impl Service {
    /// `display ... splash=<promile>`: po pripojeni displeje navazat na
    /// startovni obraz.
    pub fn begin_splash(&mut self, permille: u32) {
        self.splash_pending = Some(permille);
    }

    /// Cil prechodu: avatar prihlasovaci obrazovky (stred x, y a prumer
    /// v px obrazovky). Volat pred koncem presunu (~1,3 s od prvniho
    /// snimku); `None` = bez avataru (plocha).
    #[allow(dead_code)]
    pub fn set_splash_target(&mut self, target: Option<(f32, f32, f32)>) {
        if let Some(h) = self.splash.as_mut() {
            h.target = target;
        }
    }

    /// Pri pripojeni displeje (`install`): prvni snimek je startovni obraz.
    /// Pozadi (svetly motiv) je stejne jako plocha kompozitoru `comp`: vezme
    /// se od nej (pocita se jednou, kompozitor ho stejne potrebuje).
    pub fn splash_first_frame(&mut self, comp: &mut comp::Comp, px: &mut [u32], w: u32, h: u32) -> bool {
        let Some(p) = self.splash_pending.take() else { return false };
        let t = libos::time::now();
        let mut ho = Handoff::new(w, h, p);
        if comp.background() == theme::Theme::builtin(theme::Mode::Light).color.bg {
            ho.splash.set_backdrop(comp.backdrop().to_vec());
        }
        ho.first_frame(px, libos::time::now());
        println!("compd: navazuji na startovni obraz (prstenec {p} promile), prvni snimek {} us", (libos::time::now() - t) / 1000);
        self.splash = Some(ho);
        true
    }

    /// Kdy probudit kvuli prechodu.
    pub fn splash_wakeup(&self, now: u64) -> Option<u64> {
        self.splash.as_ref().map(|h| (h.last + FRAME_NS).max(now + 1_000_000))
    }

    /// Takt prechodu: plochu slozit do bufferu, displeji poslat obraz.
    /// `true` = vyrizeno (bezne skladani se preskoci).
    pub fn splash_tick(&mut self, now: u64) -> bool {
        let Service { splash, comp, screen, gpu, .. } = self;
        let (Some(ho), Some(c), Some(sc)) = (splash.as_mut(), comp.as_mut(), screen.as_mut()) else { return false };
        let l = ho.splash.layout;
        if (sc.w, sc.h) != (l.w, l.h) {
            // Zmena rezimu behem prechodu: obraz uz nesedi, rovnou plocha.
            *splash = None;
            c.redraw_all();
            return false;
        }
        // 1. Plocha do bufferu (snimek se nikam neposila: hned "zobrazeno").
        if c.next_wakeup(now).is_none_or(|t| t <= now) {
            let composed = match gpu.as_mut() {
                Some(g) => g.tick(c, now, &mut ho.under).unwrap_or_else(|_| {
                    c.redraw_all();
                    None
                }),
                None => gfx::Canvas::new(&mut ho.under, l.w, l.h, l.w).and_then(|mut cv| c.tick(now, &mut cv)),
            };
            if composed.is_some() {
                c.presented(now);
            }
        }
        // 2. Obraz na displej (vlastni takt).
        if now < ho.last + FRAME_NS {
            return true;
        }
        ho.last = now;
        let rects: Vec<guiproto::Rect> = match ho.phase(now) {
            Phase::Done => {
                println!("compd: prechod ze startovni obrazovky hotov ({} ms)", ho.ms(now));
                sc.pixels().copy_from_slice(&ho.under);
                *splash = None;
                c.redraw_all();
                return false;
            }
            Phase::Splash => {
                let st = ho.state(now);
                let mark = l.mark_rect(st.mark_scale, st.mark_dx, st.mark_dy);
                let mut dirty = mark.union(ho.last_mark);
                // Nazev mizi: jeho radek do posledniho snimku presunu.
                if st.word < 1.0 && ho.ms(now) <= ho.move_at() + WORD_OUT + 32 {
                    dirty = dirty.union(Rect { x: 0, y: l.word_top as i32, w: l.w, h: (bootsplash::size::WORD_H * l.s) as u32 + 1 });
                }
                ho.last_mark = mark;
                ho.splash.paint(&st, sc.pixels(), l.w as usize, dirty);
                alloc::vec![to_gui(dirty)]
            }
            Phase::Fade => {
                let st = ho.state(now);
                let px = sc.pixels();
                px.copy_from_slice(&ho.under);
                ho.splash.paint_all(&st, px, l.w as usize);
                alloc::vec![to_gui(full(&l))]
            }
        };
        sc.seq += 1;
        let req = guiproto::display::Request::Present { seq: sc.seq, output: sc.output, rects };
        let _ = sc.display.write(&req.encode(), Vec::new());
        true
    }
}
