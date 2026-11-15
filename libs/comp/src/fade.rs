//! Plynuly prechod rezimu barev na CPU: kompozitor jednou slozi novy stav
//! (aplikace dostanou hned konecny `Theme`) a kazdy snimek smicha stary a
//! novy snimek po pixelech v **linearnim svetle** (gamma LUT): stred
//! prechodu neni tmavy ani "spinavy" jako pri michani hodnot sRGB.
//!
//! - Stary snimek = obsah displeje v okamziku prepnuti (i rozprolinany,
//!   kdyz prijde dalsi zmena behem prechodu).
//! - Novy snimek se sklada normalne (jen poskozena mista) do vlastniho
//!   bufferu: aplikace, ktera se behem prechodu prekresli (klik, psani),
//!   se do noveho snimku promitne hned. Vstup se nezdrzuje.
//! - Cena snimku = jeden pruchod obrazovkou (michani) + slozeni
//!   poskozenych mist; rozmazani listy se spocita jednou pro novy stav.
//! - GPU backend (`tick_gpu`) zatim prepina hned (prolinani shaderem ve
//!   `vkcomp` je ve fronte).

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use gfx::Canvas;
use guiproto::Rect;
use theme::{Curve, Mode};

use crate::Comp;

/// Tabulky sRGB <-> linearni svetlo. Linearni hodnota ma 12 bitu: tabulka
/// zpet (4 KiB) se vejde do L1 a index nepotrebuje kontrolu mezi; nejtmavsi
/// odstiny se behem prechodu slijou do par kroku, coz v pohybu neni videt
/// (na konci prechodu je presny novy snimek).
pub(crate) struct Lut {
    to_lin: [u16; 256],
    to_srgb: [u8; LIN],
}

/// Pocet urovni linearniho svetla (12 bitu).
const LIN: usize = 4096;

impl Lut {
    pub(crate) fn new() -> Self {
        let mut to_lin = [0u16; 256];
        for (i, v) in to_lin.iter_mut().enumerate() {
            let c = i as f32 / 255.0;
            let l = if c <= 0.04045 { c / 12.92 } else { libm::powf((c + 0.055) / 1.055, 2.4) };
            *v = (l * (LIN - 1) as f32 + 0.5) as u16;
        }
        // Zpet bez mocnin: kazda linearni hodnota dostane nejblizsi kod sRGB
        // (hranice = stred mezi sousednimi kody v linearnim svetle).
        let mut to_srgb = [0u8; LIN];
        let mut code = 0usize;
        for (v, out) in to_srgb.iter_mut().enumerate() {
            while code < 255 && 2 * v as u32 >= to_lin[code] as u32 + to_lin[code + 1] as u32 {
                code += 1;
            }
            *out = code as u8;
        }
        Self { to_lin, to_srgb }
    }

    /// Michani dvou nepruhlednych pixelu: `t` 0..=256. Kanaly zvlast:
    /// varianta se vsemi tremi kanaly v jednom u64 (dve nasobeni misto
    /// sesti) byla v QEMU TCG pomalejsi (20-22 ms misto 15-19 ms pri
    /// 1280x800).
    #[inline]
    fn mix(&self, a: u32, b: u32, t: u32) -> u32 {
        if a == b {
            return a;
        }
        let s = 256 - t;
        let ch = |sh: u32| {
            let x = self.to_lin[(a >> sh & 0xFF) as usize] as u32;
            let y = self.to_lin[(b >> sh & 0xFF) as usize] as u32;
            (self.to_srgb[((x * s + y * t) >> 8) as usize & (LIN - 1)] as u32) << sh
        };
        0xFF00_0000 | ch(16) | ch(8) | ch(0)
    }
}

/// Bezici prechod.
pub(crate) struct Fade {
    old: Vec<u32>,
    new: Vec<u32>,
    w: u32,
    h: u32,
    start: u64,
    ms: u32,
    curve: Curve,
}

impl Comp {
    /// Snimek CPU s prechodem rezimu (`Comp::tick`).
    pub(crate) fn tick_fading(&mut self, now: u64, canvas: &mut Canvas<'_>) -> Option<Vec<Rect>> {
        if self.frame.in_flight {
            return self.tick_with(now, |_, _| {});
        }
        // Zacatek: stary snimek z displeje, prepnout rezim (aplikace hned
        // konecny `Theme`, cela obrazovka poskozena).
        if let Some((mode, ms)) = self.appearance.fade_pending.take() {
            self.begin_fade(mode, ms, now, canvas);
        }
        let Some(mut f) = self.appearance.fade.take() else {
            return self.tick_with(now, |c, rects| c.compose(canvas, rects, now));
        };
        {
            let mut nc = Canvas::new(&mut f.new, f.w, f.h, f.w).expect("buffer prechodu ma rozmery obrazovky");
            self.tick_with(now, |c, rects| c.compose(&mut nc, rects, now));
        }
        // Obrazovka se mezitim zmenila (jiny rezim displeje): novy stav
        // se slozi cely do displeje.
        if (canvas.w, canvas.h) != (f.w, f.h) || (self.w, self.h) != (f.w, f.h) {
            self.damage_all();
            return None;
        }
        let x = (now.saturating_sub(f.start) as f32 / 1_000_000.0 / f.ms as f32).clamp(0.0, 1.0);
        let t = if x >= 1.0 { 256 } else { (f.curve.at(x) * 256.0 + 0.5) as u32 };
        let t0 = self.prof.start();
        let lut = self.fade_lut.get_or_insert_with(|| Box::new(Lut::new()));
        let stride = canvas.stride() as usize;
        let px = canvas.pixels_mut();
        for y in 0..f.h as usize {
            let row = &mut px[y * stride..y * stride + f.w as usize];
            let (a, b) = (&f.old[y * f.w as usize..][..f.w as usize], &f.new[y * f.w as usize..][..f.w as usize]);
            if t >= 256 {
                row.copy_from_slice(b);
            } else {
                for ((o, &p), &q) in row.iter_mut().zip(a).zip(b) {
                    *o = lut.mix(p, q, t);
                }
            }
        }
        self.prof.add(crate::prof::Part::Fade, t0);
        if t < 256 {
            self.appearance.fade = Some(f);
        }
        // Snimek k zobrazeni (i kdyz se v novem stavu nic neslozilo).
        if !self.frame.in_flight {
            self.frame.in_flight = true;
            self.frame.frames += 1;
        }
        Some(vec![self.screen()])
    }

    fn begin_fade(&mut self, mode: Mode, ms: u32, now: u64, canvas: &Canvas<'_>) {
        let (w, h) = (self.w, self.h);
        if (canvas.w, canvas.h) != (w, h) {
            self.set_mode(mode);
            return;
        }
        let stride = canvas.stride() as usize;
        let src = canvas.pixels();
        let mut old = Vec::with_capacity((w * h) as usize);
        for y in 0..h as usize {
            old.extend_from_slice(&src[y * stride..y * stride + w as usize]);
        }
        self.set_mode(mode);
        let curve = self.style.theme.motion.ease_in_out;
        self.appearance.fade = Some(Fade { new: old.clone(), old, w, h, start: now, ms, curve });
    }
}

#[cfg(test)]
pub(crate) fn lut_mix(a: u32, b: u32, t: u32) -> u32 {
    Lut::new().mix(a, b, t)
}
