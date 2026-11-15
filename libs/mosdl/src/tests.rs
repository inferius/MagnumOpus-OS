use guiproto::window::{InputEvent, InputRecord};

use crate::event::{MosdlEvent, kind};

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

// Preklad udalosti a kopie snimku testuje spolecny modul (`libs/mowin`).
