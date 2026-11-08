//! HDMI: infoframy (soucty, pole), TMDS clock, scrambling.

use super::data::AOC_2243W;
use crate::cta::{self, Cta};
use crate::edid::Edid;
use crate::hdmi::{self, Colorspace, Range, kind, scdc, tmds_clock_khz, tmds_config};

#[test]
fn avi_1080p60() {
    let t = cta::vic(16).unwrap();
    let f = hdmi::avi(&t, Colorspace::Rgb, Range::Default);
    let mut out = [0u8; 31];
    let n = f.encode(&mut out);
    assert_eq!(n, 17);
    // 82 02 0D, PB1 = A0, PB2 = 16:9 + jako obraz, PB4 = VIC 16.
    assert_eq!(&out[..8], &[0x82, 0x02, 0x0D, 0x27, 0x10, 0x28, 0x00, 0x10]);
    assert_eq!(out[..n].iter().fold(0u8, |a, &b| a.wrapping_add(b)), 0);
}

#[test]
fn avi_vic_lookup_tolerates_ntsc_clock() {
    // 720p59.94 (74.176 MHz) ma stejny VIC jako 720p60.
    let mut t = cta::vic(4).unwrap();
    t.clock_khz = 74_176;
    assert_eq!(cta::vic_of(&t), 4);
    // Rezim mimo CTA (DMT 1280x1024) VIC nema.
    let t = crate::dmt::find(1280, 1024, 60, None).unwrap().timing;
    assert_eq!(cta::vic_of(&t), 0);
    let f = hdmi::avi(&t, Colorspace::Rgb, Range::Full);
    assert_eq!(f.payload[3], 0);
    assert_eq!(f.payload[2], 2 << 2);
    // 4:3 obraz.
    let t = crate::dmt::find(1024, 768, 60, None).unwrap().timing;
    assert_eq!(hdmi::avi(&t, Colorspace::Ycbcr444, Range::Limited).payload[..2], [0x50, 0x18]);
}

#[test]
fn vendor_infoframe_for_4k30() {
    let t = cta::vic(95).unwrap();
    let f = hdmi::vendor_hdmi(&t).unwrap();
    assert_eq!(f.kind, kind::VENDOR);
    assert_eq!(&f.payload[..5], &[0x03, 0x0C, 0x00, 0x20, 0x01]);
    // VIC v AVI je pak 0.
    assert_eq!(hdmi::avi(&t, Colorspace::Rgb, Range::Default).payload[3], 0);
    assert!(hdmi::vendor_hdmi(&cta::vic(16).unwrap()).is_none());
}

#[test]
fn spd_and_audio() {
    let f = hdmi::spd("MagnumOS", "intel-display");
    assert_eq!(f.len, 25);
    assert_eq!(&f.payload[..8], b"MagnumOS");
    assert_eq!(&f.payload[8..21], b"intel-display");
    assert_eq!(f.payload[24], 0x09);
    let a = hdmi::audio(2);
    assert_eq!((a.kind, a.len, a.payload[0]), (kind::AUDIO, 10, 1));
    let mut out = [0u8; 31];
    let n = a.encode(&mut out);
    assert_eq!(out[..n].iter().fold(0u8, |x, &b| x.wrapping_add(b)), 0);
}

#[test]
fn tmds_clocks() {
    let t = cta::vic(16).unwrap();
    assert_eq!(tmds_clock_khz(&t, 8, Colorspace::Rgb), 148_500);
    assert_eq!(tmds_clock_khz(&t, 10, Colorspace::Rgb), 185_625);
    assert_eq!(tmds_clock_khz(&t, 12, Colorspace::Ycbcr422), 148_500);
    let k = cta::vic(97).unwrap();
    assert_eq!(tmds_clock_khz(&k, 8, Colorspace::Ycbcr420), 297_000);
}

#[test]
fn scrambling_decision() {
    let aoc = Edid::parse(&AOC_2243W).unwrap();
    let c = aoc.cta();
    let t = cta::vic(16).unwrap();
    // HDMI 1.4 monitor bez limitu: 165 MHz, 1080p60 8 bit projde, 10 bit ne.
    let cfg = tmds_config(&t, 8, Colorspace::Rgb, c, 600_000).unwrap();
    assert!(!cfg.scrambling && !cfg.ratio_1_40);
    assert!(tmds_config(&t, 10, Colorspace::Rgb, c, 600_000).is_none());
    // HDMI 2.0 monitor (HF-VSDB 600 MHz, SCDC): 4K60 RGB jen se scramblingem.
    let mut forum = c.unwrap().clone();
    forum.hdmi_forum = Some(cta::HdmiForum { version: 1, max_tmds_char_khz: 600_000, scdc: true, lte_340_scramble: false });
    let k = cta::vic(97).unwrap();
    let cfg = tmds_config(&k, 8, Colorspace::Rgb, Some(&forum), 600_000).unwrap();
    assert!(cfg.scrambling && cfg.ratio_1_40);
    assert_eq!(scdc::tmds_config(&cfg), 0x03);
    // Zdroj do 300 MHz (napr. starsi port): 4K60 RGB ne, 4:2:0 ano bez scramblingu.
    assert!(tmds_config(&k, 8, Colorspace::Rgb, Some(&forum), 300_000).is_none());
    let cfg = tmds_config(&k, 8, Colorspace::Ycbcr420, Some(&forum), 300_000).unwrap();
    assert!(!cfg.scrambling);
    // Bez SCDC nad 340 MHz nelze.
    let mut no_scdc: Cta = forum.clone();
    no_scdc.hdmi_forum.as_mut().unwrap().scdc = false;
    assert!(tmds_config(&k, 8, Colorspace::Rgb, Some(&no_scdc), 600_000).is_none());
}
