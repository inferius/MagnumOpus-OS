use guiproto::window::{InputEvent, InputRecord};

use crate::event::{MosdlEvent, copy_frame, from_input, kind};

fn rec(event: InputEvent) -> InputRecord {
    InputRecord { event, mods: inputproto::mods::SHIFT, synthetic: false, time: 77, class: 0, serial: 5 }
}

#[test]
fn layout_matches_header() {
    // include/mosdl.h: 2x u32, u64, 4x i32, 2x u32.
    assert_eq!(core::mem::size_of::<MosdlEvent>(), 40);
    assert_eq!(core::mem::offset_of!(MosdlEvent, time_ns), 8);
    assert_eq!(core::mem::offset_of!(MosdlEvent, a), 16);
    assert_eq!(core::mem::offset_of!(MosdlEvent, synthetic), 32);
}

#[test]
fn header_constants_match() {
    let h = include_str!("../include/mosdl.h");
    for (name, v) in [
        ("MOSDL_EV_KEY", kind::KEY),
        ("MOSDL_EV_TEXT", kind::TEXT),
        ("MOSDL_EV_MOTION", kind::MOTION),
        ("MOSDL_EV_LEAVE", kind::LEAVE),
        ("MOSDL_EV_BUTTON", kind::BUTTON),
        ("MOSDL_EV_WHEEL", kind::WHEEL),
        ("MOSDL_EV_FOCUS", kind::FOCUS),
        ("MOSDL_EV_RESIZE", kind::RESIZE),
        ("MOSDL_EV_CLOSE", kind::CLOSE),
        ("MOSDL_EV_EXPOSE", kind::EXPOSE),
        ("MOSDL_EV_RELMOTION", kind::RELMOTION),
        ("MOSDL_EV_POINTER_LOCK", kind::POINTER_LOCK),
        ("MOSDL_EV_DISPLAY", kind::DISPLAY),
        ("MOSDL_EV_STATE", kind::STATE),
    ] {
        assert!(h.contains(&format!("{name} = {v},")) || h.contains(&format!("{name} = {v}\n")), "{name}");
    }
    for s in guiproto::cursor::Shape::ALL {
        let name = format!("{s:?}").to_uppercase();
        let name = match name.as_str() {
            "RESIZENWSE" => String::from("RESIZE_NWSE"),
            "RESIZENESW" => String::from("RESIZE_NESW"),
            "RESIZEEW" => String::from("RESIZE_EW"),
            "RESIZENS" => String::from("RESIZE_NS"),
            "NOTALLOWED" => String::from("NOT_ALLOWED"),
            _ => name,
        };
        let v = s as u8;
        assert!(h.contains(&format!("MOSDL_CURSOR_{name} = {v},")) || h.contains(&format!("MOSDL_CURSOR_{name} = {v}\n")), "kurzor {name}");
    }
    use guiproto::window::wstate;
    assert!(h.contains(&format!("MOSDL_STATE_MAXIMIZED = 1 << {},", wstate::MAXIMIZED.trailing_zeros())));
    assert!(h.contains(&format!("MOSDL_STATE_FULLSCREEN = 1 << {}\n", wstate::FULLSCREEN.trailing_zeros())));
    use inputproto::mods;
    for (name, v) in [("SHIFT", mods::SHIFT), ("CTRL", mods::CTRL), ("ALT", mods::ALT), ("ALTGR", mods::ALTGR), ("META", mods::META), ("CAPS", mods::CAPS_LOCK), ("NUM", mods::NUM_LOCK)] {
        let bit = v.trailing_zeros();
        assert!(h.contains(&format!("MOSDL_MOD_{name} = 1 << {bit}")), "{name}");
    }
}

#[test]
fn input_translation() {
    let e = from_input(&rec(InputEvent::Key { code: 30, action: 1, ch: Some('a') }));
    assert_eq!((e.kind, e.a, e.b, e.c, e.mods, e.time_ns), (kind::KEY, 30, 1, 'a' as i32, 1, 77));
    let e = from_input(&rec(InputEvent::Button { code: 0x110, pressed: true, x: 3, y: -4 }));
    assert_eq!((e.kind, e.a, e.b, e.c, e.d), (kind::BUTTON, 0x110, 1, 3, -4));
    let e = from_input(&rec(InputEvent::Scroll { dx: 0, dy: -2 }));
    assert_eq!((e.kind, e.a, e.b), (kind::WHEEL, 0, -2));
    let e = from_input(&rec(InputEvent::PointerRelative { dx: -3, dy: 7 }));
    assert_eq!((e.kind, e.a, e.b), (kind::RELMOTION, -3, 7));
    let e = from_input(&rec(InputEvent::Text('č')));
    assert_eq!((e.kind, e.a), (kind::TEXT, 'č' as i32));
    let mut r = rec(InputEvent::Focus(false));
    r.synthetic = true;
    let e = from_input(&r);
    assert_eq!((e.kind, e.a, e.synthetic), (kind::FOCUS, 0, 1));
}

#[test]
fn frame_copy_crops_and_pads() {
    // Snimek 3x2 s krokem 16 B do okna 2x3.
    let mut src = vec![0u8; 16 * 2];
    for y in 0..2 {
        for x in 0..3 {
            let o = y * 16 + x * 4;
            src[o..o + 4].copy_from_slice(&[x as u8, y as u8, 9, 0]);
        }
    }
    let mut dst = vec![0u32; 6];
    assert!(copy_frame(&mut dst, 2, 3, &src, 16, 3, 2));
    assert_eq!(dst[0], 0xFF09_0000);
    assert_eq!(dst[1], 0xFF09_0001);
    assert_eq!(dst[3], 0xFF09_0101);
    assert_eq!(&dst[4..], &[0xFF00_0000, 0xFF00_0000]);
}

#[test]
fn frame_copy_rejects_short_source() {
    let mut dst = vec![0u32; 4];
    // krok mensi nez radek
    assert!(!copy_frame(&mut dst, 2, 2, &[0; 64], 4, 2, 2));
    // malo dat: posledni radek by presahl
    assert!(!copy_frame(&mut dst, 2, 2, &[0; 15], 8, 2, 2));
    assert!(copy_frame(&mut dst, 2, 2, &[0; 16], 8, 2, 2));
    // nepratelske rozmery bez preteceni
    assert!(!copy_frame(&mut dst, 2, 2, &[0; 16], u32::MAX, 2, u32::MAX));
}
