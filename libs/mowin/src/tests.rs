use guiproto::window::{InputEvent, InputRecord};

use crate::event::{MoEvent, copy_frame, from_input, kind};

fn rec(event: InputEvent) -> InputRecord {
    InputRecord { event, mods: inputproto::mods::SHIFT, synthetic: false, time: 77, class: 0, serial: 5 }
}

#[test]
fn layout_matches_header() {
    // include/mowin.h: 2x u32, u64, 4x i32, 2x u32 (jako mosdl_event).
    assert_eq!(core::mem::size_of::<MoEvent>(), 40);
    assert_eq!(core::mem::offset_of!(MoEvent, time_ns), 8);
    assert_eq!(core::mem::offset_of!(MoEvent, a), 16);
    assert_eq!(core::mem::offset_of!(MoEvent, synthetic), 32);
    assert_eq!(core::mem::offset_of!(MoEvent, window), 36);
}

#[test]
fn header_constants_match() {
    let h = include_str!("../include/mowin.h");
    for (name, v) in [
        ("MOWIN_EV_KEY", kind::KEY),
        ("MOWIN_EV_TEXT", kind::TEXT),
        ("MOWIN_EV_MOTION", kind::MOTION),
        ("MOWIN_EV_LEAVE", kind::LEAVE),
        ("MOWIN_EV_BUTTON", kind::BUTTON),
        ("MOWIN_EV_WHEEL", kind::WHEEL),
        ("MOWIN_EV_FOCUS", kind::FOCUS),
        ("MOWIN_EV_RESIZE", kind::RESIZE),
        ("MOWIN_EV_CLOSE", kind::CLOSE),
        ("MOWIN_EV_EXPOSE", kind::EXPOSE),
        ("MOWIN_EV_RELMOTION", kind::RELMOTION),
        ("MOWIN_EV_POINTER_LOCK", kind::POINTER_LOCK),
        ("MOWIN_EV_DISPLAY", kind::DISPLAY),
        ("MOWIN_EV_STATE", kind::STATE),
        ("MOWIN_EV_POPUP_DISMISS", kind::POPUP_DISMISS),
        ("MOWIN_EV_PLACE", kind::PLACE),
        ("MOWIN_EV_CLIPBOARD", kind::CLIPBOARD),
    ] {
        assert!(h.contains(&format!("{name} = {v},")) || h.contains(&format!("{name} = {v}
")), "{name}");
    }
    use guiproto::window::{Mode, wstate};
    assert!(h.contains(&format!("MOWIN_MODE_STANDARD = {},", Mode::Standard.to_u8())));
    assert!(h.contains(&format!("MOWIN_MODE_FRAMELESS = {}
", Mode::Frameless.to_u8())));
    assert!(h.contains(&format!("MOWIN_STATE_FOCUSED = 1 << {},", wstate::FOCUSED.trailing_zeros())));
    assert!(h.contains(&format!("MOWIN_STATE_MAXIMIZED = 1 << {},", wstate::MAXIMIZED.trailing_zeros())));
    assert!(h.contains(&format!("MOWIN_STATE_FULLSCREEN = 1 << {}
", wstate::FULLSCREEN.trailing_zeros())));
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

/// `moaudio.h`: formaty vzorku jsou stejna cisla jako `mosdl.h` (spolecne
/// jadro je preklada jednou tabulkou) a smery streamu jako `audioproto`.
#[test]
fn moaudio_header_matches_mosdl() {
    let h = include_str!("../include/moaudio.h");
    let sdl = include_str!("../../mosdl/include/mosdl.h");
    for (i, name) in ["U8", "S16", "S32", "F32"].iter().enumerate() {
        assert!(h.contains(&format!("MOAUDIO_SAMPLE_{name} = {i}")), "MOAUDIO_SAMPLE_{name}");
        assert!(sdl.contains(&format!("MOSDL_SAMPLE_{name} = {i}")), "MOSDL_SAMPLE_{name}");
    }
    assert!(h.contains("MOAUDIO_OUTPUT = 0") && h.contains("MOAUDIO_INPUT = 1"));
}
