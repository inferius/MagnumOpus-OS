//! Testy formatu zprav (na hostiteli: `cargo test -p inputproto`).

use crate::code::*;
use crate::control::Status;
use crate::device::{self, DeviceMessage, RawEvent};
use crate::event::{self, Action, Button, Event, Record, flags};

#[test]
fn raw_event_roundtrip() {
    let e = RawEvent::new(EV_REL, REL_Y, -7);
    assert_eq!(RawEvent::from_bytes(&e.to_bytes()), Some(e));
    assert_eq!(RawEvent::from_bytes(&[1, 2, 3]), None);
}

#[test]
fn hello_roundtrip_and_name_limit() {
    let msg = device::encode_hello(device::caps::KEYBOARD, "QEMU Virtio Keyboard");
    assert_eq!(
        device::decode(&msg),
        Some(DeviceMessage::Hello {
            version: device::VERSION,
            caps: device::caps::KEYBOARD,
            name: "QEMU Virtio Keyboard",
        })
    );
    // Dlouhe jmeno s vicebajtovymi znaky se zkrati na hranici znaku.
    let long = "č".repeat(100);
    let msg = device::encode_hello(0, &long);
    match device::decode(&msg) {
        Some(DeviceMessage::Hello { name, .. }) => {
            assert!(name.len() <= device::MAX_NAME);
            assert!(name.chars().all(|c| c == 'č'));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn events_roundtrip() {
    let evs = [
        RawEvent::key(KEY_A, true),
        RawEvent::syn(),
        RawEvent::rel(REL_X, 5),
        RawEvent::syn(),
    ];
    let msg = device::encode_events(&evs);
    assert_eq!(
        device::decode(&msg),
        Some(DeviceMessage::Events(evs.to_vec()))
    );
}

#[test]
fn device_decode_rejects_garbage() {
    assert_eq!(device::decode(b""), None);
    assert_eq!(device::decode(b"XXXX"), None);
    // Delka udalosti neni nasobek 8.
    assert_eq!(device::decode(b"IEVT\x01\x00\x1e"), None);
    // Jmeno neni UTF-8.
    assert_eq!(device::decode(b"IDEV\x01\x00\x00\x00\xff\xfe"), None);
    // Prilis mnoho udalosti.
    let mut big = b"IEVT".to_vec();
    big.resize(4 + (device::MAX_EVENTS + 1) * 8, 0);
    assert_eq!(device::decode(&big), None);
}

#[test]
fn records_roundtrip_all_kinds() {
    let events = [
        Event::Key {
            code: KEY_Y,
            scancode: 0x15,
            action: Action::Press,
            ch: Some('z'),
        },
        Event::Key {
            code: KEY_ENTER,
            scancode: KEY_ENTER as u32,
            action: Action::Repeat,
            ch: None,
        },
        Event::Text('ě'),
        Event::Text('😀'),
        Event::Motion { dx: -3, dy: 12 },
        Event::Button {
            button: Button::Left,
            action: Action::Press,
        },
        Event::Button {
            button: Button::Other(0x116),
            action: Action::Release,
        },
        Event::Scroll { dx: 0, dy: -1 },
        Event::SecureAttention,
        Event::Focus(true),
        Event::Focus(false),
        Event::Injection { active: 2 },
        Event::Layout(1),
        Event::Dropped { messages: 7 },
    ];
    let records: Vec<Record> = events
        .iter()
        .enumerate()
        .map(|(i, &e)| Record::new(e, i as u16, flags::SYNTHETIC, 1_000 + i as u64))
        .collect();
    let msg = event::encode(&records);
    assert_eq!(msg.len(), records.len() * Record::SIZE);
    assert_eq!(event::decode(&msg), Some(records));
}

#[test]
fn record_unknown_kind_is_kept() {
    let mut b = Record::new(Event::Focus(true), 0, 0, 0).to_bytes();
    b[0] = 0xEE;
    b[1] = 0x00;
    let r = Record::from_bytes(&b).unwrap();
    assert_eq!(r.event, Event::Unknown { kind: 0xEE });
}

#[test]
fn record_rejects_invalid_values() {
    let key = Record::new(
        Event::Key {
            code: KEY_A,
            scancode: 0,
            action: Action::Press,
            ch: Some('a'),
        },
        0,
        0,
        0,
    );
    // Neplatna akce.
    let mut b = key.to_bytes();
    b[24] = 9;
    assert_eq!(Record::from_bytes(&b), None);
    // Neplatny znak (surrogate).
    let mut b = key.to_bytes();
    b[28..32].copy_from_slice(&0xD800u32.to_le_bytes());
    assert_eq!(Record::from_bytes(&b), None);
    // Tlacitko nema opakovani.
    let mut b = Record::new(
        Event::Button {
            button: Button::Left,
            action: Action::Press,
        },
        0,
        0,
        0,
    )
    .to_bytes();
    b[24] = 2;
    assert_eq!(Record::from_bytes(&b), None);
    // Zprava, ktera neni nasobek zaznamu.
    assert_eq!(event::decode(&[0u8; 31]), None);
}

#[test]
fn status_roundtrip() {
    let mut s = Status {
        devices: 3,
        keyboards: 1,
        pointers: 1,
        injecting: 1,
        dropped: 4,
        secure: true,
        ..Status::default()
    };
    s.layout[..2].copy_from_slice(b"cz");
    let mut text = String::from("ok ");
    s.format(&mut text).unwrap();
    assert_eq!(Status::parse(&text), Some(s));
    assert_eq!(Status::parse(&text).unwrap().layout_name(), "cz");
    assert_eq!(Status::parse("ok devices=x"), None);
}

#[test]
fn button_codes() {
    for b in [
        Button::Left,
        Button::Right,
        Button::Middle,
        Button::Back,
        Button::Forward,
    ] {
        assert_eq!(Button::from_code(b.code()), b);
        assert!(is_mouse_button(b.code()));
    }
    assert!(!is_mouse_button(KEY_A));
    // Tlacitka joysticku, tabletu a gamepadu nejsou klavesy.
    for c in [0x100, BTN_LEFT, 0x120, 0x130, 0x14a, 0x15f, 0x2c0, 0x2e7] {
        assert!(is_button(c), "{c:#x}");
    }
    for c in [KEY_A, KEY_DELETE, 0x160, 0x2bf, 0x2e8, KEY_MAX] {
        assert!(!is_button(c), "{c:#x}");
    }
}
