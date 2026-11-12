//! Prvni snimek startovni obrazovky (`libs/bootsplash`): cerna a znak
//! s prazdnym prstencem. Rozsviceni pozadi, nazev a prubeh doplni init,
//! jadro mezitim do framebufferu nekresli (`boot_flags::SPLASH`).
//!
//! Kresli se primo do framebufferu GOP (firmware ho mapuje identicky),
//! z framebufferu se nikdy necte. Rezim (rozliseni) se nemeni: obraz se
//! jen prizpusobi meritkem.

use boot_info::{Framebuffer, PixelFormat};
use bootsplash::{Splash, State};

/// Vykresli prvni snimek; `false` = framebuffer neni.
pub fn show(fb: &Framebuffer) -> bool {
    if !fb.is_present() || fb.stride < fb.width {
        return false;
    }
    let t0 = crate::arch::cycle_counter();
    let mut s = Splash::new(fb.width, fb.height);
    s.set_rgb(fb.format == PixelFormat::Rgb);
    let len = fb.stride as usize * fb.height as usize;
    if (len * 4) as u64 > fb.len {
        return false;
    }
    // SAFETY: framebuffer GOP je pri bezicich boot services identicky
    // namapovany, `len` pixelu lezi uvnitr `fb.len` (overeno vyse) a nikdo
    // jiny do nej ted nepise.
    let px = unsafe { core::slice::from_raw_parts_mut(fb.phys_addr as *mut u32, len) };
    s.paint_all(&State::FIRST, px, fb.stride as usize);
    // Od ted text jen na seriovou linku (konzole firmwaru by psala pres obraz).
    crate::logger::serial_only();
    log::info!(
        "startovni obraz {}x{} (meritko {:.2}), {} Mcyklu",
        fb.width,
        fb.height,
        s.layout.s,
        (crate::arch::cycle_counter() - t0) / 1_000_000
    );
    true
}
