//! Kurzor, zamek ukazatele a cela obrazovka na zadost (verze 5).

use alloc::vec;

use crate::cursor::{Cursor, Custom, MAX_CUSTOM, Shape};
use crate::window::*;

fn rt_client(m: ClientMsg) {
    let enc = m.encode();
    assert_eq!(ClientMsg::decode(&enc), Ok(m.clone()), "{m:?}");
    for n in 0..enc.len() {
        assert!(ClientMsg::decode(&enc[..n]).is_err());
    }
}

#[test]
fn cursor_messages_roundtrip() {
    for s in Shape::ALL {
        rt_client(ClientMsg::SetCursor { window: 1, cursor: Cursor::Shape(s) });
    }
    let c = Custom { w: 3, h: 2, hot_x: 2, hot_y: 1, pixels: vec![0xFF00_00FF; 6] };
    rt_client(ClientMsg::SetCursor { window: 7, cursor: Cursor::Custom(c) });
    let big = Custom { w: MAX_CUSTOM, h: MAX_CUSTOM, hot_x: 0, hot_y: 0, pixels: vec![0; 64 * 64] };
    let enc = ClientMsg::SetCursor { window: 1, cursor: Cursor::Custom(big) }.encode();
    assert!(enc.len() <= crate::MAX_MESSAGE, "nejvetsi kurzor se vejde do zpravy");
    rt_client(ClientMsg::WindowState { window: 1, fullscreen: true, gesture: 9 });
    rt_client(ClientMsg::PointerLock { window: 1, lock: false, gesture: 0 });
}

#[test]
fn cursor_rejects_bad_input() {
    // Neznamy tvar.
    let mut enc = ClientMsg::SetCursor { window: 1, cursor: Cursor::Shape(Shape::Hidden) }.encode();
    *enc.last_mut().unwrap() = 13;
    assert!(ClientMsg::decode(&enc).is_err());
    // Prilis velky obrazek, aktivni bod mimo, nulova velikost.
    for (w, h, hx, hy) in [(65u16, 1u16, 0u16, 0u16), (2, 2, 2, 0), (0, 1, 0, 0)] {
        let c = Custom { w, h, hot_x: hx, hot_y: hy, pixels: vec![0; w as usize * h as usize] };
        assert!(c.validate().is_err());
        let enc = ClientMsg::SetCursor { window: 1, cursor: Cursor::Custom(c) }.encode();
        assert!(ClientMsg::decode(&enc).is_err(), "{w}x{h} hot {hx},{hy}");
    }
    // Pravdivostni hodnota jen 0/1.
    let mut enc = ClientMsg::PointerLock { window: 1, lock: true, gesture: 3 }.encode();
    enc[8] = 2;
    assert!(ClientMsg::decode(&enc).is_err());
}

#[test]
fn relative_motion_and_lock_state() {
    let rec = InputRecord { event: InputEvent::PointerRelative { dx: -5, dy: 12 }, mods: 0, synthetic: false, time: 4, class: 0, serial: 0 };
    let m = ServerMsg::Input { window: 2, records: vec![rec] };
    assert_eq!(ServerMsg::decode(&m.encode()), Ok(m));
    let m = ServerMsg::PointerLockState { window: 2, locked: true };
    assert_eq!(ServerMsg::decode(&m.encode()), Ok(m.clone()));
    let enc = m.encode();
    for n in 0..enc.len() {
        assert!(ServerMsg::decode(&enc[..n]).is_err());
    }
}
