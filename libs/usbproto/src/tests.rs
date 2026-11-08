//! Testy (na hostiteli: `cargo test -p usbproto`).

use inputproto::code::*;
use inputproto::device::RawEvent;

use crate::Speed;
use crate::desc::{Configuration, DeviceDescriptor, Setup, Transfer, class};
use crate::hid::{BootKeyboard, BootMouse, usage_to_key};
use crate::hub::{self, HubDescriptor, PortStatus, port};
use crate::msg::{Info, Reply, Request, op};

/// Deskriptor zarizeni QEMU usb-kbd (zkraceno na podstatne).
const DEVICE: [u8; 18] = [
    18, 1, 0x00, 0x02, 0, 0, 0, 8, 0x27, 0x06, 0x01, 0x00, 0x00, 0x00, 1, 2, 3, 1,
];

/// Konfigurace klavesnice: rozhrani HID boot (3/1/1), deskriptor HID,
/// endpoint 0x81 interrupt 8 B, interval 10 ms.
const CONFIG: [u8; 34] = [
    9, 2, 34, 0, 1, 1, 0, 0xA0, 50, // konfigurace
    9, 4, 0, 0, 1, 3, 1, 1, 0, // rozhrani 0
    9, 0x21, 0x11, 0x01, 0, 1, 0x22, 63, 0, // HID, report 63 B
    7, 5, 0x81, 3, 8, 0, 10, // endpoint
];

#[test]
fn device_descriptor() {
    let d = DeviceDescriptor::parse(&DEVICE).unwrap();
    assert_eq!((d.vendor, d.product), (0x0627, 0x0001));
    assert_eq!(d.usb_version, 0x0200);
    assert_eq!(d.max_packet0, 8);
    assert_eq!(d.configurations, 1);
    assert_eq!(DeviceDescriptor::parse(&DEVICE[..10]), None);
}

#[test]
fn configuration_parse() {
    assert_eq!(Configuration::total_length(&CONFIG[..9]), Some(34));
    let c = Configuration::parse(&CONFIG).unwrap();
    assert_eq!(c.value, 1);
    assert_eq!(c.interfaces.len(), 1);
    let i = &c.interfaces[0];
    assert_eq!((i.class, i.subclass, i.protocol), (class::HID, 1, 1));
    assert_eq!(i.hid_report_len, 63);
    assert_eq!(i.endpoints.len(), 1);
    let e = i.endpoints[0];
    assert!(e.is_in());
    assert_eq!(e.number(), 1);
    assert_eq!(e.transfer, Transfer::Interrupt);
    assert_eq!((e.max_packet, e.interval), (8, 10));
}

#[test]
fn configuration_alternates_and_rejects_bad_length() {
    let mut c = CONFIG.to_vec();
    // Druhe nastaveni rozhrani 0 (alternate 1) s vlastnim endpointem.
    c.extend_from_slice(&[9, 4, 0, 1, 1, 3, 0, 0, 0, 7, 5, 0x82, 3, 4, 0, 10]);
    let total = c.len() as u16;
    c[2..4].copy_from_slice(&total.to_le_bytes());
    let parsed = Configuration::parse(&c).unwrap();
    assert_eq!(parsed.interfaces.len(), 1);
    assert_eq!(parsed.interfaces[0].endpoints.len(), 1);
    assert_eq!(parsed.interfaces[0].endpoints[0].address, 0x81);
    // Alternativni nastaveni je zvlast a jde ho pouzit misto zakladniho.
    assert_eq!(parsed.alternates.len(), 1);
    assert_eq!(parsed.alternate(0, 1).map(|i| i.endpoints[0].address), Some(0x82));
    assert_eq!(parsed.alternate(0, 2), None);
    let alt = parsed.with_alternate(0, 1).unwrap();
    assert_eq!(alt.interfaces.len(), 1);
    assert_eq!((alt.interfaces[0].alternate, alt.interfaces[0].endpoints[0].address), (1, 0x82));
    assert_eq!(parsed.with_alternate(1, 1), None);
    // Deskriptor s delkou 0 (zacykleni) a pres konec.
    let mut bad = CONFIG.to_vec();
    bad[9] = 0;
    assert_eq!(Configuration::parse(&bad), None);
    let mut bad = CONFIG.to_vec();
    bad[27] = 40;
    assert_eq!(Configuration::parse(&bad), None);
}

#[test]
fn setup_roundtrip() {
    let s = Setup::get_descriptor(1, 0, 18);
    assert_eq!(s.to_bytes(), [0x80, 6, 0, 1, 0, 0, 18, 0]);
    assert_eq!(Setup::from_bytes(&s.to_bytes()), Some(s));
    assert!(s.is_in());
    assert!(!Setup::set_configuration(1).is_in());
    assert_eq!(hub::set_port_feature(3, hub::feature::PORT_RESET).to_bytes(), [0x23, 3, 4, 0, 3, 0, 0, 0]);
    assert_eq!(hub::get_port_status(2).to_bytes(), [0xA3, 0, 0, 0, 2, 0, 4, 0]);
    // SET_INTERFACE rozhrani 2, alternativa 1: standardni, prijemce rozhrani.
    assert_eq!(Setup::set_interface(2, 1).to_bytes(), [0x01, 11, 1, 0, 2, 0, 0, 0]);
}

#[test]
fn hub_route_string() {
    // Hub na root portu (uroven 0): port v nejnizsich 4 bitech.
    assert_eq!(hub::route(0, 0, 3), Some(0x3));
    // Dalsi uroven: port o 4 bity vys, rodic zustava.
    assert_eq!(hub::route(0x3, 1, 15), Some(0xF3));
    assert_eq!(hub::route(0x4321, 4, 5), Some(0x54321));
    // Port 0, port nad 15 a sesta uroven nejde adresovat.
    assert_eq!(hub::route(0, 0, 0), None);
    assert_eq!(hub::route(0, 0, 16), None);
    assert_eq!(hub::route(0x4321, 2, 255), None);
    assert_eq!(hub::route(0x54321, 5, 1), None);
}

#[test]
fn messages_roundtrip() {
    let raw = Request::control(7, Setup::get_descriptor(2, 0, 9), &[]);
    let r = Request::decode(&raw).unwrap();
    assert_eq!((r.op, r.id), (op::CONTROL, 7));
    assert_eq!(Setup::from_bytes(r.data), Some(Setup::get_descriptor(2, 0, 9)));
    let reply = Reply {
        id: 7,
        status: -5,
        completion: crate::msg::completion::STALL,
        actual: 3,
        data: &[1, 2, 3],
    };
    assert_eq!(Reply::decode(&reply.encode()), Some(reply.clone()));
    assert_eq!(Request::decode(&[0; 15]), None);
    assert_eq!(Reply::decode(&[0; 3]), None);
}

#[test]
fn info_roundtrip() {
    let info = Info {
        speed: Speed::Full,
        interface: 0,
        path: vec![2, 4],
        device: DeviceDescriptor::parse(&DEVICE).unwrap(),
        config: CONFIG.to_vec(),
    };
    let decoded = Info::decode(&info.encode(&DEVICE)).unwrap();
    assert_eq!(decoded, info);
    assert_eq!(Info::decode(&[9, 0, 0]), None);
}

#[test]
fn hub_descriptor_and_status() {
    let d = HubDescriptor {
        ports: 8,
        characteristics: 0x0009,
        power_on_2ms: 10,
    };
    let b = d.to_bytes();
    assert_eq!(b[0] as usize, b.len());
    assert_eq!(b.len(), 7 + 2 * 2);
    assert_eq!(HubDescriptor::parse(&b), Some(d));
    let s = PortStatus {
        status: port::CONNECTION | port::ENABLE | port::POWER | port::HIGH_SPEED,
        change: port::C_CONNECTION,
    };
    assert_eq!(PortStatus::from_bytes(&s.to_bytes()), Some(s));
    assert_eq!(s.speed(), Speed::High);
    assert_eq!(
        PortStatus {
            status: port::LOW_SPEED,
            change: 0
        }
        .speed(),
        Speed::Low
    );
    assert_eq!(PortStatus::default().speed(), Speed::Full);
}

fn keys(events: &[RawEvent]) -> Vec<(u16, bool)> {
    events
        .iter()
        .filter(|e| e.kind == EV_KEY)
        .map(|e| (e.code, e.value != 0))
        .collect()
}

#[test]
fn boot_keyboard_diff() {
    let mut kb = BootKeyboard::new();
    let mut out = Vec::new();
    // Shift + a.
    kb.report(&[0x02, 0, 0x04, 0, 0, 0, 0, 0], &mut out);
    assert_eq!(keys(&out), vec![(KEY_LEFTSHIFT, true), (KEY_A, true)]);
    assert!(out.contains(&RawEvent::new(EV_MSC, MSC_SCAN, 0x70004)));
    out.clear();
    // a pusteno, b stisknuto, Shift drzeny.
    kb.report(&[0x02, 0, 0x05, 0, 0, 0, 0, 0], &mut out);
    assert_eq!(keys(&out), vec![(KEY_A, false), (KEY_B, true)]);
    out.clear();
    // Rollover (usage 1): ignorovat, stav se nemeni.
    kb.report(&[0x02, 0, 1, 1, 1, 1, 1, 1], &mut out);
    assert!(out.is_empty());
    kb.release_all(&mut out);
    assert_eq!(keys(&out), vec![(KEY_LEFTSHIFT, false), (KEY_B, false)]);
    // Kazda klavesa je vlastni snimek.
    assert_eq!(out.iter().filter(|e| **e == RawEvent::syn()).count(), 2);
}

#[test]
fn usage_table() {
    assert_eq!(usage_to_key(0x04), Some(KEY_A));
    assert_eq!(usage_to_key(0x1D), Some(KEY_Z));
    assert_eq!(usage_to_key(0x27), Some(KEY_0));
    assert_eq!(usage_to_key(0x28), Some(KEY_ENTER));
    assert_eq!(usage_to_key(0x45), Some(KEY_F12));
    assert_eq!(usage_to_key(0x4C), Some(KEY_DELETE));
    assert_eq!(usage_to_key(0x52), Some(KEY_UP));
    assert_eq!(usage_to_key(0x62), Some(KEY_KP0));
    assert_eq!(usage_to_key(0x64), Some(KEY_102ND));
    assert_eq!(usage_to_key(0x65), Some(KEY_COMPOSE));
    assert_eq!(usage_to_key(0xE6), Some(KEY_RIGHTALT));
    assert_eq!(usage_to_key(0x66), None);
    assert_eq!(usage_to_key(0x00), None);
}

#[test]
fn boot_mouse() {
    let mut m = BootMouse::new();
    let mut out = Vec::new();
    m.report(&[0x01, 5, 0xFD, 1], &mut out);
    assert_eq!(
        out,
        vec![
            RawEvent::key(BTN_LEFT, true),
            RawEvent::rel(REL_X, 5),
            RawEvent::rel(REL_Y, -3),
            RawEvent::rel(REL_WHEEL, 1),
            RawEvent::syn(),
        ]
    );
    out.clear();
    m.report(&[0x01, 0, 0], &mut out);
    assert!(out.is_empty());
    m.report(&[0x00, 0, 0, 0], &mut out);
    assert_eq!(out, vec![RawEvent::key(BTN_LEFT, false), RawEvent::syn()]);
    out.clear();
    m.report(&[0x01], &mut out);
    assert!(out.is_empty());
}

#[test]
fn fido_rozpoznani() {
    // Deskriptor reportu YubiKey (CTAPHID): Usage Page (FIDO) 06 D0 F1,
    // Usage (CTAPHID) 09 01, kolekce, vstup a vystup po 64 B.
    let fido = [
        0x06, 0xD0, 0xF1, 0x09, 0x01, 0xA1, 0x01, 0x09, 0x20, 0x15, 0x00, 0x26, 0xFF, 0x00, 0x75, 0x08, 0x95, 0x40,
        0x81, 0x02, 0x09, 0x21, 0x15, 0x00, 0x26, 0xFF, 0x00, 0x75, 0x08, 0x95, 0x40, 0x91, 0x02, 0xC0,
    ];
    assert!(crate::hid::is_fido(&fido));
    // Klavesnice (Usage Page Generic Desktop 05 01) neni klic.
    let kbd = [0x05, 0x01, 0x09, 0x06, 0xA1, 0x01, 0x05, 0x07, 0x19, 0xE0, 0xC0];
    assert!(!crate::hid::is_fido(&kbd));
    // Useknuty deskriptor ani prazdny nespadne.
    assert!(!crate::hid::is_fido(&fido[..1]));
    assert!(!crate::hid::is_fido(&[]));
    assert!(!crate::hid::is_fido(&[0xFE]));
}
