//! EDID, CTA-861, DisplayID a seznam rezimu nad realnymi dumpy
//! (`data.rs`) a nad bloky sestavenymi v testu.

use alloc::vec::Vec;

use super::data::{AOC_2243W, LGD_LP133WH2};
use super::fix_checksum;
use crate::Error;
use crate::displayid::{DisplayId, encode_timing};
use crate::edid::{self, Edid, Extension, Interface, StdTiming, checksum_ok};
use crate::modes::{Limits, modes, pick};
use crate::timing::{Source, flags};

#[test]
fn dumps_have_valid_checksums() {
    for b in AOC_2243W.chunks(128).chain(LGD_LP133WH2.chunks(128)) {
        assert!(checksum_ok(b));
    }
}

#[test]
fn aoc_base_block() {
    let e = Edid::parse(&AOC_2243W).unwrap();
    assert_eq!(e.vendor_str(), "AOC");
    assert_eq!(e.product, 0x2243);
    assert_eq!(e.serial, 0x0410);
    assert_eq!((e.week, e.year), (5, 2013));
    assert_eq!(e.version, (1, 3));
    assert!(e.input.digital);
    assert_eq!(e.input.interface, Interface::Undefined);
    assert_eq!(e.size_cm, (48, 27));
    assert_eq!(e.gamma, Some(220));
    assert_eq!(e.name.as_deref(), Some("2243W"));
    assert_eq!(e.serial_text.as_deref(), Some("ASKD29A001040"));
    assert_eq!(e.display_name(), "2243W");
    let r = e.range.unwrap();
    assert_eq!((r.vmin_hz, r.vmax_hz, r.hmin_khz, r.hmax_khz), (55, 75, 30, 83));
    assert_eq!(r.max_clock_khz, 170_000);
    // Preferovane 1920x1080@60 (DTD) s velikosti obrazu.
    let p = e.preferred().unwrap();
    assert_eq!((p.hactive, p.vactive, p.clock_khz), (1920, 1080, 148_500));
    assert_eq!((p.hfront, p.hsync, p.hback), (88, 44, 148));
    assert_eq!((p.vfront, p.vsync, p.vback), (4, 5, 36));
    assert_eq!(p.flags & (flags::HSYNC_POS | flags::VSYNC_POS), flags::HSYNC_POS | flags::VSYNC_POS);
    assert!(p.preferred());
    assert_eq!(e.size_mm(), (477, 268));
    // Standardni casovani: 81 80 = 1280x1024@60 (5:4), 95 00 = 1440x900
    // (16:10), d1 c0 = 1920x1080@60 (16:9).
    assert!(e.standard.contains(&StdTiming { w: 1280, h: 1024, refresh_hz: 60 }));
    assert!(e.standard.contains(&StdTiming { w: 1920, h: 1080, refresh_hz: 60 }));
    assert!(e.standard.contains(&StdTiming { w: 1440, h: 900, refresh_hz: 60 }));
    // Ustalena: bf ef 00.
    assert!(e.established.contains(&(640, 480, 60, false)));
    assert!(e.established.contains(&(1024, 768, 75, false)));
    assert!(!e.established.contains(&(1152, 870, 75, false)));
}

#[test]
fn aoc_cta_extension() {
    let e = Edid::parse(&AOC_2243W).unwrap();
    assert!(e.is_hdmi());
    let c = e.cta().unwrap();
    assert_eq!(c.revision, 3);
    assert!(c.underscan && c.basic_audio && c.ycbcr444 && c.ycbcr422);
    assert_eq!(c.native_dtds, 1);
    let vics: Vec<u8> = c.svds.iter().map(|s| s.vic).collect();
    assert_eq!(vics, [5, 20, 16, 31, 4, 19, 3, 18, 2, 17, 1]);
    let a = c.audio[0];
    assert_eq!((a.format, a.channels, a.rates, a.detail), (1, 2, 0x07, 0x01));
    assert_eq!(c.speakers, 1);
    let h = c.hdmi.unwrap();
    assert_eq!(h.phys_addr, 0x1000);
    assert!(!h.dc30 && h.max_tmds_khz == 0);
    assert!(c.hdmi_forum.is_none());
    // Ctyri DTD v rozsireni: 720x480, 1280x720, 720x480, 720x576.
    let d: Vec<(u16, u16)> = c.detailed.iter().map(|d| (d.timing.hactive, d.timing.vactive)).collect();
    assert_eq!(d, [(720, 480), (1280, 720), (720, 480), (720, 576)]);
}

#[test]
fn aoc_mode_list() {
    let e = Edid::parse(&AOC_2243W).unwrap();
    let m = modes(&e, &Limits::new(600_000, 4096, 4096));
    // Prvni je preferovany 1920x1080@60 z detailniho casovani.
    assert!(m[0].timing.preferred());
    assert_eq!((m[0].timing.hactive, m[0].timing.vactive, m[0].timing.refresh_hz()), (1920, 1080, 60));
    assert_eq!(m[0].source, Source::Detailed);
    assert_eq!(m.iter().filter(|x| x.timing.preferred()).count(), 1);
    // Prokladane (VIC 5, 20) bez povoleni chybi.
    assert!(m.iter().all(|x| !x.timing.interlaced()));
    // 1080p50 z VIC 31, 1280x1024@60 z DMT, bez duplicit.
    assert!(m.iter().any(|x| x.timing.hactive == 1920 && x.timing.refresh_hz() == 50 && x.source == Source::Vic));
    assert!(m.iter().any(|x| x.timing.hactive == 1280 && x.timing.vactive == 1024 && x.source == Source::Dmt));
    for (i, a) in m.iter().enumerate() {
        assert!(m[i + 1..].iter().all(|b| !a.timing.same_mode(&b.timing)), "{:?}", a);
    }
    // Serazeno: po preferovanem od nejvetsi plochy.
    let areas: Vec<u32> = m[1..].iter().map(|x| x.timing.hactive as u32 * x.timing.vactive as u32).collect();
    assert!(areas.windows(2).all(|w| w[0] >= w[1]));
    // Limit monitoru 170 MHz plati, limit zdroje take.
    assert!(m.iter().all(|x| x.timing.clock_khz <= 170_000));
    let small = modes(&e, &Limits::new(100_000, 4096, 4096));
    assert!(small.iter().all(|x| x.timing.clock_khz <= 100_000));
    assert!(!small.iter().any(|x| x.timing.hactive == 1920));
    // Vyber: 1280x720 s obnovou nejblize 50 Hz.
    let p = pick(&m, 1280, 720, 50_000).unwrap();
    assert_eq!(p.timing.refresh_hz(), 50);
    assert!(pick(&m, 1280, 720, 0).unwrap().timing.refresh_hz() == 60);
    assert!(pick(&m, 999, 999, 0).is_none());
}

#[test]
fn laptop_panel() {
    let e = Edid::parse(&LGD_LP133WH2).unwrap();
    assert_eq!(e.vendor_str(), "LGD");
    assert_eq!(e.product, 0x0217);
    assert_eq!(e.year, 2009);
    assert!(e.extensions.is_empty());
    assert!(e.name.is_none());
    assert_eq!(e.texts, ["LGDisplay", "LP133WH2-TLA2"]);
    let p = e.preferred().unwrap();
    assert_eq!((p.hactive, p.vactive, p.clock_khz), (1366, 768, 69_300));
    assert_eq!(p.htotal(), 1470);
    assert_eq!(p.vtotal(), 786);
    assert_eq!(p.refresh_hz(), 60);
    assert_eq!(e.size_mm(), (293, 165));
    assert_eq!(e.display_name(), "LGD 0217");
    // Panel ma jediny rezim (zadna standardni ani ustalena casovani).
    let m = modes(&e, &Limits::new(600_000, 4096, 4096));
    assert_eq!(m.len(), 1);
}

#[test]
fn rejects_corrupted() {
    assert_eq!(Edid::parse(&AOC_2243W[..100]), Err(Error::Truncated));
    let mut bad = AOC_2243W;
    bad[0] = 1;
    assert_eq!(Edid::parse(&bad), Err(Error::BadHeader));
    let mut bad = AOC_2243W;
    bad[60] ^= 0x40;
    assert_eq!(Edid::parse(&bad), Err(Error::BadChecksum));
    // Vadny soucet rozsireni: zakladni blok plati, rozsireni oznacene.
    let mut bad = AOC_2243W;
    bad[200] ^= 1;
    let e = Edid::parse(&bad).unwrap();
    assert_eq!(e.extensions, [Extension::Broken(0x02)]);
    assert!(e.cta().is_none());
    // Rozsireni ohlasene, ale chybi: jen zakladni blok.
    let e = Edid::parse(&AOC_2243W[..128]).unwrap();
    assert!(e.extensions.is_empty());
}

#[test]
fn read_retries_bad_block() {
    // Prvni cteni zakladniho bloku je poskozene (sum na DDC), druhe dobre.
    let mut calls = 0;
    let out = edid::read(|i, b| {
        calls += 1;
        let start = i as usize * 128;
        b.copy_from_slice(&AOC_2243W[start..start + 128]);
        if calls == 1 {
            b[5] = 0;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(out, AOC_2243W);
    assert_eq!(calls, 3);
    // Trvale vadny zakladni blok = chyba.
    let r = edid::read(|_, b| {
        b.fill(0);
        Ok(())
    });
    assert_eq!(r, Err(Error::BadChecksum));
    // Chyba prenosu se preda dal.
    assert_eq!(edid::read(|_, _| Err(Error::Nack)), Err(Error::Nack));
}

/// Rozsireni DisplayID 1.3 s jednim casovanim typu I a jmenem produktu.
fn displayid_block() -> [u8; 128] {
    let mut b = [0u8; 128];
    b[0] = 0x70;
    let mut sec: Vec<u8> = Vec::new();
    // Identifikace produktu (0x00): id "TST", kod, serie, tyden, rok, jmeno.
    let name = b"Tile 5K";
    let mut pid = Vec::from(*b"TST");
    pid.extend_from_slice(&[0x34, 0x12, 1, 0, 0, 0, 10, 20, name.len() as u8]);
    pid.extend_from_slice(name);
    sec.extend_from_slice(&[0x00, 0x00, pid.len() as u8]);
    sec.extend_from_slice(&pid);
    // Casovani typu I (0x03): 5120x2880@60 RB v2.
    let t = crate::cvt::cvt(5120, 2880, 60_000, crate::cvt::Blanking::ReducedV2).unwrap();
    let mut pref = t;
    pref.flags |= flags::PREFERRED;
    pref.clock_khz = pref.clock_khz / 10 * 10;
    sec.extend_from_slice(&[0x03, 0x00, 20]);
    sec.extend_from_slice(&encode_timing(&pref, false));
    // Hlavicka sekce: verze 1.3, delka, typ produktu, 0 rozsireni.
    let mut s = alloc::vec![0x13, sec.len() as u8, 0x03, 0x00];
    s.extend_from_slice(&sec);
    let sum = s.iter().fold(0u8, |a, &x| a.wrapping_add(x));
    s.push(0u8.wrapping_sub(sum));
    b[1..1 + s.len()].copy_from_slice(&s);
    fix_checksum(&mut b);
    b
}

#[test]
fn displayid_type1_in_edid() {
    let mut data = Vec::from(&LGD_LP133WH2[..]);
    data[126] = 1;
    fix_checksum(&mut data[..128]);
    data.extend_from_slice(&displayid_block());
    let e = Edid::parse(&data).unwrap();
    let d: Vec<&DisplayId> = e.displayid().collect();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].version, 0x13);
    assert_eq!(d[0].name.as_deref(), Some("Tile 5K"));
    let t = d[0].timings[0];
    assert_eq!((t.hactive, t.vactive, t.refresh_hz()), (5120, 2880, 60));
    assert!(t.preferred());
    assert_eq!(t.flags & (flags::HSYNC_POS | flags::VSYNC_POS), flags::HSYNC_POS);
    // Rezim z DisplayID je v seznamu (jen kdyz ho zdroj unese).
    let m = modes(&e, &Limits::new(1_000_000, 8192, 8192));
    assert!(m.iter().any(|x| x.timing.hactive == 5120));
    let m = modes(&e, &Limits::new(600_000, 4096, 4096));
    assert!(!m.iter().any(|x| x.timing.hactive == 5120));
}

#[test]
fn displayid_type7_roundtrip_and_checks() {
    let t = crate::cvt::cvt(3840, 2160, 120_000, crate::cvt::Blanking::ReducedV2).unwrap();
    let b = encode_timing(&t, true);
    let back = crate::displayid::timing(&b, true).unwrap();
    // Priznak redukovaneho zatemneni DisplayID nenese.
    let t = crate::timing::Timing { flags: flags::HSYNC_POS, ..t };
    assert_eq!(back, t);
    // Sekce 2.0 s blokem 0x22.
    let mut s = alloc::vec![0x20, 23, 0x00, 0x00, 0x22, 0x00, 20];
    s.extend_from_slice(&b);
    let sum = s.iter().fold(0u8, |a, &x| a.wrapping_add(x));
    s.push(0u8.wrapping_sub(sum));
    let d = DisplayId::parse(&s).unwrap();
    assert_eq!(d.timings, [t]);
    // Spatny soucet sekce.
    let mut bad = s.clone();
    bad[10] ^= 1;
    assert_eq!(DisplayId::parse(&bad), Err(Error::BadChecksum));
    // Delka bloku pres konec sekce.
    let mut bad = s.clone();
    bad[6] = 40;
    let sum = bad[..bad.len() - 1].iter().fold(0u8, |a, &x| a.wrapping_add(x));
    *bad.last_mut().unwrap() = 0u8.wrapping_sub(sum);
    assert_eq!(DisplayId::parse(&bad), Err(Error::Truncated));
}

#[test]
fn hdmi_forum_vsdb() {
    // Sestavene rozsireni: video blok s VIC 97 (2160p60) a HF-VSDB 600 MHz + SCDC.
    let mut b = [0u8; 128];
    b[0] = 0x02;
    b[1] = 0x03;
    let blocks = [
        0x42, 97, 16, // video: VIC 97, 16
        0x67, 0xD8, 0x5D, 0xC4, 0x01, 120, 0x88, 0x00, // HF-VSDB: verze 1, 600 MHz, SCDC + LTE_340
        0x65, 0x03, 0x0C, 0x00, 0x10, 0x00, // HDMI VSDB 1.0.0.0
    ];
    b[4..4 + blocks.len()].copy_from_slice(&blocks);
    b[2] = (4 + blocks.len()) as u8;
    fix_checksum(&mut b);
    let c = crate::cta::Cta::parse(&b).unwrap();
    let f = c.hdmi_forum.unwrap();
    assert_eq!((f.version, f.max_tmds_char_khz, f.scdc, f.lte_340_scramble), (1, 600_000, true, true));
    let t: Vec<u32> = c.vic_timings().iter().map(|t| t.clock_khz).collect();
    assert_eq!(t, [594_000, 148_500]);
}
