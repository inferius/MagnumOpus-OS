//! Testy dekodovani (na hostiteli: `cargo test -p i8042`).

use inputproto::code::*;
use inputproto::device::RawEvent;

use crate::mouse::{self, Kind, Packet};
use crate::scancode::{Decoder, Key};

fn keys(bytes: &[u8]) -> Vec<Key> {
    let mut d = Decoder::new();
    bytes.iter().filter_map(|&b| d.feed(b)).collect()
}

fn k(code: u16, scancode: u32, pressed: bool) -> Key {
    Key {
        code,
        scancode,
        pressed,
    }
}

#[test]
fn basic_keys_are_evdev() {
    // a (0x1E), uvolneni (0x9E), Esc, F12.
    assert_eq!(
        keys(&[0x1E, 0x9E, 0x01, 0x58, 0xD8]),
        vec![
            k(KEY_A, 0x1E, true),
            k(KEY_A, 0x1E, false),
            k(KEY_ESC, 1, true),
            k(KEY_F12, 0x58, true),
            k(KEY_F12, 0x58, false),
        ]
    );
    assert_eq!(keys(&[0x2A, 0xAA]), vec![k(KEY_LEFTSHIFT, 0x2A, true), k(KEY_LEFTSHIFT, 0x2A, false)]);
}

#[test]
fn extended_keys() {
    assert_eq!(
        keys(&[0xE0, 0x1D, 0xE0, 0x38, 0xE0, 0x53, 0xE0, 0xD3, 0xE0, 0x48]),
        vec![
            k(KEY_RIGHTCTRL, 0xE01D, true),
            k(KEY_RIGHTALT, 0xE038, true),
            k(KEY_DELETE, 0xE053, true),
            k(KEY_DELETE, 0xE053, false),
            k(KEY_UP, 0xE048, true),
        ]
    );
}

#[test]
fn fake_shifts_and_print_screen() {
    // Print Screen: E0 2A E0 37, uvolneni E0 B7 E0 AA.
    assert_eq!(
        keys(&[0xE0, 0x2A, 0xE0, 0x37, 0xE0, 0xB7, 0xE0, 0xAA]),
        vec![k(KEY_SYSRQ, 0xE037, true), k(KEY_SYSRQ, 0xE037, false)]
    );
}

#[test]
fn pause_key() {
    assert_eq!(
        keys(&[0xE1, 0x1D, 0x45, 0xE1, 0x9D, 0xC5, 0x1E]),
        vec![
            k(KEY_PAUSE, 0xE11D45, true),
            k(KEY_PAUSE, 0xE11D45, false),
            k(KEY_A, 0x1E, true),
        ]
    );
}

#[test]
fn control_bytes_and_unknown_codes_ignored() {
    assert_eq!(keys(&[0xFA, 0xFE, 0x00, 0xFF, 0xEE, 0x70, 0xE0, 0x10]), vec![]);
    // Po neznamem rozsirenem kodu dekoder pokracuje normalne.
    assert_eq!(keys(&[0xE0, 0x10, 0x1E]), vec![k(KEY_A, 0x1E, true)]);
}

#[test]
fn mouse_standard_packet() {
    let mut d = mouse::Decoder::new(Kind::Standard);
    // Leve tlacitko, X = +5, Y = +3 (nahoru) -> dy = -3.
    assert_eq!(d.feed(0x09), None);
    assert_eq!(d.feed(5), None);
    assert_eq!(
        d.feed(3),
        Some(Packet {
            dx: 5,
            dy: -3,
            wheel: 0,
            buttons: mouse::LEFT
        })
    );
    // Zaporny X (znamenko v bitu 4): 0xFE = -2; Y dolu: znamenko bit 5, 0xFC = -4 -> dy = 4.
    for b in [0x38, 0xFE] {
        assert_eq!(d.feed(b), None);
    }
    assert_eq!(
        d.feed(0xFC),
        Some(Packet {
            dx: -2,
            dy: 4,
            wheel: 0,
            buttons: 0
        })
    );
}

#[test]
fn mouse_sync_and_overflow() {
    let mut d = mouse::Decoder::new(Kind::Standard);
    // Bajt bez bitu 3 na zacatku paketu se zahodi.
    assert_eq!(d.feed(0x05), None);
    assert_eq!(d.feed(0x08), None);
    assert_eq!(d.feed(1), None);
    assert_eq!(d.feed(1).map(|p| (p.dx, p.dy)), Some((1, -1)));
    // Preteceni X: pohyb zahozen, tlacitka plati.
    for b in [0x4A, 0xFF] {
        assert_eq!(d.feed(b), None);
    }
    assert_eq!(
        d.feed(0x10),
        Some(Packet {
            dx: 0,
            dy: 0,
            wheel: 0,
            buttons: mouse::RIGHT
        })
    );
}

#[test]
fn mouse_wheel_and_five_buttons() {
    let mut d = mouse::Decoder::new(Kind::Wheel);
    for b in [0x08, 0, 0] {
        assert_eq!(d.feed(b), None);
    }
    // Kolecko 0x01 (k uzivateli) -> REL_WHEEL -1.
    assert_eq!(d.feed(0x01).map(|p| p.wheel), Some(-1));

    let mut d = mouse::Decoder::new(Kind::FiveButton);
    for b in [0x08, 0, 0] {
        d.feed(b);
    }
    // Dolni 4 bity 0xF = -1 (od uzivatele) -> +1; bit 4 zpet, bit 5 vpred.
    let p = d.feed(0x3F).unwrap();
    assert_eq!(p.wheel, 1);
    assert_eq!(p.buttons, mouse::BACK | mouse::FORWARD);
}

#[test]
fn mouse_events_only_on_change() {
    let mut d = mouse::Decoder::new(Kind::Wheel);
    let mut out = Vec::new();
    d.events(
        Packet {
            dx: 3,
            dy: -2,
            wheel: 1,
            buttons: mouse::LEFT,
        },
        &mut out,
    );
    assert_eq!(
        out,
        vec![
            RawEvent::key(BTN_LEFT, true),
            RawEvent::rel(REL_X, 3),
            RawEvent::rel(REL_Y, -2),
            RawEvent::rel(REL_WHEEL, 1),
            RawEvent::syn(),
        ]
    );
    out.clear();
    // Stejna tlacitka, zadny pohyb: nic.
    d.events(
        Packet {
            buttons: mouse::LEFT,
            ..Packet::default()
        },
        &mut out,
    );
    assert!(out.is_empty());
    d.events(Packet::default(), &mut out);
    assert_eq!(out, vec![RawEvent::key(BTN_LEFT, false), RawEvent::syn()]);
}

#[test]
fn kinds() {
    assert_eq!(Kind::from_id(0), Kind::Standard);
    assert_eq!(Kind::from_id(3), Kind::Wheel);
    assert_eq!(Kind::from_id(4), Kind::FiveButton);
    assert_eq!(Kind::from_id(3).packet_len(), 4);
    assert_eq!(crate::controller::leds(true, false, true), 0b101);
}
