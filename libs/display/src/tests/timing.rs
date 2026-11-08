//! CVT a DMT: hodnoty porovnane s kalkulatorem VESA CVT 1.2 (stejne
//! vypisuje `cvt` z X.Org) a s tabulkou DMT.

use crate::cvt::{Blanking, cvt};
use crate::dmt;
use crate::timing::{Timing, flags};

/// (clock kHz, hactive, hsync start, hsync end, htotal, vactive, vsync
/// start, vsync end, vtotal) - format modeline.
fn modeline(t: &Timing) -> [u32; 9] {
    let h = t.hactive as u32;
    let v = t.vactive as u32;
    [
        t.clock_khz,
        h,
        h + t.hfront as u32,
        h + t.hfront as u32 + t.hsync as u32,
        t.htotal(),
        v,
        v + t.vfront as u32,
        v + t.vfront as u32 + t.vsync as u32,
        t.vtotal(),
    ]
}

#[test]
fn cvt_standard_matches_vesa() {
    // cvt 1920 1080 60
    let t = cvt(1920, 1080, 60_000, Blanking::Standard).unwrap();
    assert_eq!(modeline(&t), [173_000, 1920, 2048, 2248, 2576, 1080, 1083, 1088, 1120]);
    assert_eq!(t.flags & (flags::HSYNC_POS | flags::VSYNC_POS), flags::VSYNC_POS);
    // cvt 1024 768 60
    let t = cvt(1024, 768, 60_000, Blanking::Standard).unwrap();
    assert_eq!(modeline(&t), [63_500, 1024, 1072, 1176, 1328, 768, 771, 775, 798]);
    // cvt 1280 800 60 (16:10 -> vsync 6)
    let t = cvt(1280, 800, 60_000, Blanking::Standard).unwrap();
    assert_eq!(modeline(&t), [83_500, 1280, 1352, 1480, 1680, 800, 803, 809, 831]);
}

#[test]
fn cvt_reduced_v1_matches_vesa() {
    // cvt -r 1920 1080 60
    let t = cvt(1920, 1080, 60_000, Blanking::ReducedV1).unwrap();
    assert_eq!(modeline(&t), [138_500, 1920, 1968, 2000, 2080, 1080, 1083, 1088, 1111]);
    assert_eq!(t.flags & (flags::HSYNC_POS | flags::VSYNC_POS), flags::HSYNC_POS);
    // cvt -r 2560 1440 60
    let t = cvt(2560, 1440, 60_000, Blanking::ReducedV1).unwrap();
    assert_eq!(modeline(&t), [241_500, 2560, 2608, 2640, 2720, 1440, 1443, 1448, 1481]);
}

#[test]
fn cvt_reduced_v2_exact_rate() {
    // 3840x2160@60 RB v2: htotal 3920, vtotal 2222, 522.614 MHz.
    let t = cvt(3840, 2160, 60_000, Blanking::ReducedV2).unwrap();
    assert_eq!(t.htotal(), 3920);
    assert_eq!(t.vtotal(), 2222);
    assert_eq!(t.clock_khz, 522_614);
    assert_eq!((t.hfront, t.hsync, t.hback), (8, 32, 40));
    assert_eq!((t.vsync, t.vback), (8, 6));
    assert_eq!(t.refresh_hz(), 60);
}

#[test]
fn cvt_rejects_nonsense() {
    assert!(cvt(0, 768, 60_000, Blanking::Standard).is_none());
    assert!(cvt(1024, 768, 0, Blanking::Standard).is_none());
    // Obnova tak vysoka, ze na radky nezbude cas.
    assert!(cvt(1024, 768, 2_000_000, Blanking::Standard).is_none());
}

#[test]
fn dmt_table_is_consistent() {
    for e in dmt::TABLE {
        let t = &e.timing;
        assert!(t.is_valid(), "DMT {:#x}", e.id);
        // Kazdy zaznam ma obnovu blizko celeho cisla v Hz (do 1 %).
        let r = t.refresh_mhz();
        let hz = t.refresh_hz() * 1000;
        assert!(r.abs_diff(hz) * 100 <= hz, "DMT {:#x}: {r} mHz", e.id);
    }
    // Id jsou jedinecna.
    for (i, a) in dmt::TABLE.iter().enumerate() {
        assert!(dmt::TABLE[i + 1..].iter().all(|b| b.id != a.id));
    }
}

#[test]
fn dmt_lookup() {
    let e = dmt::find(1920, 1080, 60, None).unwrap();
    assert_eq!(e.id, 0x52);
    assert_eq!(e.timing.clock_khz, 148_500);
    // 1280x800: bez preference bezne zatemneni, s preferenci RB.
    assert_eq!(dmt::find(1280, 800, 60, None).unwrap().id, 0x1C);
    assert_eq!(dmt::find(1280, 800, 60, Some(true)).unwrap().id, 0x1B);
    assert!(dmt::find(1234, 567, 60, None).is_none());
    assert_eq!(dmt::by_id(0x10).unwrap().timing.hactive, 1024);
}

#[test]
fn refresh_of_ntsc_like_rate() {
    // 1920x1080 s pixel clock 148.352 MHz = 59.94 Hz.
    let mut t = dmt::find(1920, 1080, 60, None).unwrap().timing;
    t.clock_khz = 148_352;
    assert_eq!(t.refresh_mhz(), 59_940);
    assert_eq!(t.refresh_hz(), 60);
    assert!(t.same_mode(&dmt::find(1920, 1080, 60, None).unwrap().timing));
}
