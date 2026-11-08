//! Vypocet casovani podle VESA Coordinated Video Timings 1.2: standardni
//! zatemneni (CRT i obecne), redukovane zatemneni v1 (LCD) a v2 (vysoka
//! rozliseni, presna frekvence).
//!
//! Vzorce jsou ze specifikace (verejny tabulkovy kalkulator VESA); pocita
//! se v celych cislech (pikosekundy, ppm), aby vysledek sedel na hodnoty
//! z kalkulatoru i bez plovouci carky v `no_std`. Okraje (margins) se
//! nepouzivaji, prokladany rezim nepodporujeme (`None`).

use crate::timing::{Timing, flags};

/// Druh zatemneni.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blanking {
    Standard,
    ReducedV1,
    ReducedV2,
}

/// Velikost bunky znaku (vodorovne rozmery jsou jejim nasobkem).
const CELL_GRAN: u32 = 8;
/// Nejmensi predni porch svisle (radky).
const MIN_V_PORCH: u32 = 3;
/// Nejmensi zadni porch svisle (radky).
const MIN_V_BPORCH: u32 = 6;
/// Nejmensi doba svisleho pulzu + zadniho porchu (ps), standardni CVT.
const MIN_VSYNC_BP_PS: u64 = 550_000_000;
/// Nejmensi doba svisleho zatemneni (ps), redukovane CVT.
const RB_MIN_VBLANK_PS: u64 = 460_000_000;

/// Sirka svisleho pulzu podle pomeru stran (standardni CVT a RB v1).
fn vsync_width(h: u32, v: u32) -> u32 {
    if v * 4 == h * 3 {
        4
    } else if v * 16 == h * 9 {
        5
    } else if v * 16 == h * 10 {
        6
    } else if v * 5 == h * 4 || v * 15 == h * 9 {
        7
    } else {
        // Nestandardni pomer stran.
        10
    }
}

/// Casovani rezimu `h` x `v` s obnovou `refresh_mhz` (tisiciny Hz).
pub fn cvt(h: u32, v: u32, refresh_mhz: u32, blanking: Blanking) -> Option<Timing> {
    if h < CELL_GRAN || v < 2 || refresh_mhz < 1000 || h > 16384 || v > 16384 {
        return None;
    }
    // Vodorovne rozmery zaokrouhlene dolu na bunky; u RB v2 presne.
    let h = match blanking {
        Blanking::ReducedV2 => h,
        _ => h / CELL_GRAN * CELL_GRAN,
    };
    // Perioda pole v ps.
    let field_ps: u64 = 1_000_000_000_000_000 / refresh_mhz as u64;
    match blanking {
        Blanking::Standard => standard(h, v, field_ps),
        Blanking::ReducedV1 => reduced(h, v, refresh_mhz, field_ps, false),
        Blanking::ReducedV2 => reduced(h, v, refresh_mhz, field_ps, true),
    }
}

fn standard(h: u32, v: u32, field_ps: u64) -> Option<Timing> {
    let vsync = vsync_width(h, v);
    // Odhad periody radku.
    let h_period_ps = field_ps.checked_sub(MIN_VSYNC_BP_PS)? / (v + MIN_V_PORCH) as u64;
    if h_period_ps == 0 {
        return None;
    }
    let mut vsync_bp = (MIN_VSYNC_BP_PS / h_period_ps) as u32 + 1;
    if vsync_bp < vsync + MIN_V_BPORCH {
        vsync_bp = vsync + MIN_V_BPORCH;
    }
    let vback = vsync_bp - vsync;
    // Idealni stridani (duty cycle) zatemneni v ppm procenta:
    // C' - M' * H_PERIOD / 1000, C' = 30, M' = 300 (H v us).
    let duty = 30_000_000i64 - (3 * h_period_ps as i64) / 10;
    let duty = duty.max(20_000_000) as u64;
    let gran2 = 2 * CELL_GRAN as u64;
    let h_blank = (h as u64 * duty / ((100_000_000 - duty) * gran2) * gran2) as u32;
    let total = h + h_blank;
    // Pixel clock zaokrouhleny dolu na krok 0.25 MHz.
    let clock_khz = (total as u64 * 1_000_000_000 / h_period_ps / 250 * 250) as u32;
    // Vodorovny pulz 8 % celkove sirky, zaokrouhleny na bunky.
    let hsync = total * 8 / 100 / CELL_GRAN * CELL_GRAN;
    let hback = h_blank / 2;
    let hfront = h_blank - hback - hsync;
    Some(Timing {
        clock_khz,
        hactive: h as u16,
        hfront: hfront as u16,
        hsync: hsync as u16,
        hback: hback as u16,
        vactive: v as u16,
        vfront: MIN_V_PORCH as u16,
        vsync: vsync as u16,
        vback: vback as u16,
        flags: flags::VSYNC_POS,
    })
}

fn reduced(h: u32, v: u32, refresh_mhz: u32, field_ps: u64, v2: bool) -> Option<Timing> {
    let (h_blank, hsync, hfront, vsync, vfront_min) = if v2 { (80, 32, 8, 8, 1) } else { (160, 32, 48, vsync_width(h, v), 3) };
    let h_period_ps = field_ps.checked_sub(RB_MIN_VBLANK_PS)? / v as u64;
    if h_period_ps == 0 {
        return None;
    }
    let vbi_lines = (RB_MIN_VBLANK_PS / h_period_ps) as u32 + 1;
    let min_vbi = vfront_min + vsync + MIN_V_BPORCH;
    let vbi = vbi_lines.max(min_vbi);
    let (vfront, vback) = if v2 {
        // RB v2: zadni porch pevne 6, zbytek do predniho.
        (vbi - vsync - MIN_V_BPORCH, MIN_V_BPORCH)
    } else {
        (vfront_min, vbi - vfront_min - vsync)
    };
    let total_v = v + vbi;
    let total_h = h + h_blank;
    // f = obnova * radky * pixely; RB v1 krok 0.25 MHz, RB v2 krok 1 kHz.
    let khz = refresh_mhz as u64 * total_v as u64 * total_h as u64 / 1_000_000;
    let clock_khz = if v2 { khz } else { khz / 250 * 250 } as u32;
    Some(Timing {
        clock_khz,
        hactive: h as u16,
        hfront: hfront as u16,
        hsync: hsync as u16,
        hback: (h_blank - hfront - hsync) as u16,
        vactive: v as u16,
        vfront: vfront as u16,
        vsync: vsync as u16,
        vback: vback as u16,
        flags: flags::HSYNC_POS | flags::REDUCED,
    })
}
