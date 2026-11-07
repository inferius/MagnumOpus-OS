//! Testy parseru a dekoderu reportu HID (na hostiteli: `cargo test -p
//! usbproto`). Deskriptory jsou ze zarizeni QEMU (usb-kbd, usb-mouse,
//! usb-tablet) a vlastni pro multimedialni klavesy.

use inputproto::code::*;
use inputproto::device::{AxisInfo, RawEvent, caps};

use crate::hid_report::Decoder;
use crate::report::{self, flags, usage};

/// Klavesnice v boot formatu (HID 1.11, dodatek E.6; QEMU usb-kbd).
const KEYBOARD: &[u8] = &[
    0x05, 0x01, 0x09, 0x06, 0xA1, 0x01, 0x05, 0x07, 0x19, 0xE0, 0x29, 0xE7, 0x15, 0x00, 0x25,
    0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0x95, 0x01, 0x75, 0x08, 0x81, 0x01, 0x95, 0x05,
    0x75, 0x01, 0x05, 0x08, 0x19, 0x01, 0x29, 0x05, 0x91, 0x02, 0x95, 0x01, 0x75, 0x03, 0x91,
    0x01, 0x95, 0x06, 0x75, 0x08, 0x15, 0x00, 0x25, 0x65, 0x05, 0x07, 0x19, 0x00, 0x29, 0x65,
    0x81, 0x00, 0xC0,
];

/// Mys: 5 tlacitek, X, Y, kolecko relativne (i8).
const MOUSE: &[u8] = &[
    0x05, 0x01, 0x09, 0x02, 0xA1, 0x01, 0x09, 0x01, 0xA1, 0x00, 0x05, 0x09, 0x19, 0x01, 0x29,
    0x05, 0x15, 0x00, 0x25, 0x01, 0x95, 0x05, 0x75, 0x01, 0x81, 0x02, 0x95, 0x01, 0x75, 0x03,
    0x81, 0x01, 0x05, 0x01, 0x09, 0x30, 0x09, 0x31, 0x09, 0x38, 0x15, 0x81, 0x25, 0x7F, 0x75,
    0x08, 0x95, 0x03, 0x81, 0x06, 0xC0, 0xC0,
];

/// Tablet QEMU: 3 tlacitka, X a Y absolutne 0..0x7FFF (u16), kolecko.
const TABLET: &[u8] = &[
    0x05, 0x01, 0x09, 0x02, 0xA1, 0x01, 0x09, 0x01, 0xA1, 0x00, 0x05, 0x09, 0x19, 0x01, 0x29,
    0x03, 0x15, 0x00, 0x25, 0x01, 0x95, 0x03, 0x75, 0x01, 0x81, 0x02, 0x95, 0x01, 0x75, 0x05,
    0x81, 0x01, 0x05, 0x01, 0x09, 0x30, 0x09, 0x31, 0x15, 0x00, 0x26, 0xFF, 0x7F, 0x35, 0x00,
    0x46, 0xFF, 0x7F, 0x75, 0x10, 0x95, 0x02, 0x81, 0x02, 0x05, 0x01, 0x09, 0x38, 0x15, 0x81,
    0x25, 0x7F, 0x35, 0x00, 0x45, 0x00, 0x75, 0x08, 0x95, 0x01, 0x81, 0x06, 0xC0, 0xC0,
];

/// Multimedialni klavesy s id reportu 2: hlasitost +, -, ztlumeni (bity),
/// pak vypln; report 1 = nic (jiny report).
const CONSUMER: &[u8] = &[
    0x05, 0x0C, 0x09, 0x01, 0xA1, 0x01, 0x85, 0x02, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95,
    0x03, 0x09, 0xE9, 0x09, 0xEA, 0x09, 0xE2, 0x81, 0x02, 0x95, 0x05, 0x81, 0x01, 0xC0,
];

fn keys(ev: &[RawEvent]) -> Vec<(u16, bool)> {
    ev.iter()
        .filter(|e| e.kind == EV_KEY)
        .map(|e| (e.code, e.value != 0))
        .collect()
}

#[test]
fn parse_keyboard_fields() {
    let d = report::parse(KEYBOARD).unwrap();
    assert!(!d.uses_ids);
    // Modifikatory (promenna 8x1) a klavesy (pole 6x8); vystup LED a vypln
    // nejsou vstupni pole.
    assert_eq!(d.fields.len(), 2);
    let m = &d.fields[0];
    assert!(m.is_variable());
    assert_eq!((m.bit_offset, m.size, m.count), (0, 1, 8));
    assert_eq!(m.usage_of(0), Some(usage(7, 0xE0)));
    let k = &d.fields[1];
    assert!(!k.is_variable());
    assert_eq!((k.bit_offset, k.size, k.count), (16, 8, 6));
    assert_eq!(k.usage_for_index(4), Some(usage(7, 4)));
    assert_eq!(d.max_input_len(), 8);
}

#[test]
fn keyboard_reports() {
    let mut dec = Decoder::new(report::parse(KEYBOARD).unwrap());
    assert_eq!(dec.caps(), caps::KEYBOARD);
    let mut out = Vec::new();
    dec.report(&[0x02, 0, 0x04, 0, 0, 0, 0, 0], &mut out);
    assert_eq!(keys(&out), vec![(KEY_LEFTSHIFT, true), (KEY_A, true)]);
    assert!(out.contains(&RawEvent::new(EV_MSC, MSC_SCAN, 0x70004)));
    assert_eq!(out.last(), Some(&RawEvent::syn()));
    out.clear();
    dec.report(&[0x02, 0, 0x05, 0, 0, 0, 0, 0], &mut out);
    assert_eq!(keys(&out), vec![(KEY_A, false), (KEY_B, true)]);
    out.clear();
    // Rollover: nic.
    dec.report(&[0x02, 0, 1, 1, 1, 1, 1, 1], &mut out);
    assert!(keys(&out).is_empty());
    dec.release_all(&mut out);
    assert_eq!(keys(&out), vec![(KEY_LEFTSHIFT, false), (KEY_B, false)]);
}

#[test]
fn mouse_reports() {
    let mut dec = Decoder::new(report::parse(MOUSE).unwrap());
    assert_eq!(dec.caps(), caps::POINTER);
    let mut out = Vec::new();
    // Prave tlacitko, dx -2, dy 3, kolecko -1.
    dec.report(&[0x02, 0xFE, 0x03, 0xFF], &mut out);
    assert_eq!(
        out,
        vec![
            RawEvent::new(EV_MSC, MSC_SCAN, 0x90002),
            RawEvent::key(BTN_RIGHT, true),
            RawEvent::rel(REL_X, -2),
            RawEvent::rel(REL_Y, 3),
            RawEvent::rel(REL_WHEEL, -1),
            RawEvent::syn(),
        ]
    );
    out.clear();
    // Bez zmeny tlacitek a bez pohybu: zadny snimek.
    dec.report(&[0x02, 0, 0, 0], &mut out);
    assert!(out.is_empty());
}

#[test]
fn tablet_absolute_axes() {
    let mut dec = Decoder::new(report::parse(TABLET).unwrap());
    assert_eq!(dec.caps(), caps::ABSOLUTE);
    assert_eq!(
        dec.axes(),
        vec![
            AxisInfo {
                code: ABS_X,
                min: 0,
                max: 0x7FFF
            },
            AxisInfo {
                code: ABS_Y,
                min: 0,
                max: 0x7FFF
            },
        ]
    );
    let mut out = Vec::new();
    // Leve tlacitko, x = 0x4000, y = 0x2000, kolecko 0.
    dec.report(&[0x01, 0x00, 0x40, 0x00, 0x20, 0x00], &mut out);
    assert!(out.contains(&RawEvent::new(EV_ABS, ABS_X, 0x4000)));
    assert!(out.contains(&RawEvent::new(EV_ABS, ABS_Y, 0x2000)));
    assert_eq!(keys(&out), vec![(BTN_LEFT, true)]);
    out.clear();
    // Stejna poloha: absolutni osa se neopakuje.
    dec.report(&[0x01, 0x00, 0x40, 0x00, 0x20, 0x00], &mut out);
    assert!(out.is_empty());
}

#[test]
fn consumer_keys_with_report_id() {
    let d = report::parse(CONSUMER).unwrap();
    assert!(d.uses_ids);
    assert_eq!(d.max_input_len(), 2);
    let mut dec = Decoder::new(d);
    // Jen multimedialni klavesy: neni to klavesnice.
    assert_eq!(dec.caps(), caps::MEDIA);
    let mut out = Vec::new();
    dec.report(&[0x02, 0b001], &mut out);
    assert_eq!(keys(&out), vec![(KEY_VOLUMEUP, true)]);
    assert!(out.contains(&RawEvent::new(EV_MSC, MSC_SCAN, 0x0C00E9)));
    out.clear();
    dec.report(&[0x02, 0b100], &mut out);
    assert_eq!(keys(&out), vec![(KEY_VOLUMEUP, false), (KEY_MUTE, true)]);
    out.clear();
    // Report s jinym id se ignoruje.
    dec.report(&[0x01, 0xFF], &mut out);
    assert!(out.is_empty());
}

#[test]
fn parser_rejects_garbage() {
    // Prilis velke pole (report count x size nad limit).
    let huge = [0x75, 0x20, 0x96, 0xFF, 0xFF, 0x09, 0x30, 0x81, 0x02];
    assert_eq!(report::parse(&huge), None);
    // Utrzena data polozky.
    assert_eq!(report::parse(&[0x05]), None);
    // Pop bez Push.
    assert_eq!(report::parse(&[0xB4]), None);
    // Report ID 0 je neplatne.
    assert_eq!(report::parse(&[0x85, 0x00]), None);
    // Hluboke vnoreni kolekci.
    assert_eq!(report::parse(&[0xA1, 0x01].repeat(40)), None);
    // Prazdny deskriptor je platny (zadna pole).
    assert_eq!(report::parse(&[]).map(|d| d.fields.len()), Some(0));
}

#[test]
fn usage_page_applies_at_main_item() {
    // Usage (0x30) pred Usage Page (Generic Desktop): plati strana
    // v okamziku polozky Input -> X.
    let d = [0x05, 0x09, 0x09, 0x30, 0x05, 0x01, 0x15, 0x81, 0x25, 0x7F, 0x75, 0x08, 0x95, 0x01, 0x81, 0x06];
    let desc = report::parse(&d).unwrap();
    assert_eq!(desc.fields[0].usage_of(0), Some(usage(1, 0x30)));
    // Usage s 4 B dat ma vlastni stranu.
    let d = [0x05, 0x09, 0x0B, 0x30, 0x00, 0x01, 0x00, 0x15, 0x81, 0x25, 0x7F, 0x75, 0x08, 0x95, 0x01, 0x81, 0x06];
    let desc = report::parse(&d).unwrap();
    assert_eq!(desc.fields[0].usage_of(0), Some(usage(1, 0x30)));
}

#[test]
fn signed_and_wide_values() {
    // Jedno pole X: 12 bitu se znamenkem (-2048..2047), relativni.
    let d = [
        0x05, 0x01, 0x09, 0x30, 0x16, 0x00, 0xF8, 0x26, 0xFF, 0x07, 0x75, 0x0C, 0x95, 0x01, 0x81,
        0x06,
    ];
    let desc = report::parse(&d).unwrap();
    let f = &desc.fields[0];
    assert_eq!((f.logical_min, f.logical_max), (-2048, 2047));
    assert!(f.flags & flags::RELATIVE != 0);
    // -5 ve 12 bitech = 0xFFB.
    assert_eq!(f.value(&[0xFB, 0x0F], 0), Some(-5));
    assert_eq!(f.value(&[0xFB], 0), None);
}
