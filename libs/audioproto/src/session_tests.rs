//! Relace aplikace: recept a pravidla (session.rs).

use alloc::string::String;

use crate::client::Request;
use crate::session::{Capture, CaptureStep, MAX_NAME, Recipe, app_allowed, capture_rule};
use crate::{Direction, Format, Sample};

fn recipe() -> Recipe {
    Recipe { app_id: "org.libsdl.testaudio".into(), app_name: "SDL testaudio".into(), uid: 1001, capture: Capture::Ask }
}

#[test]
fn recipe_roundtrip_and_system() {
    let r = recipe();
    assert_eq!(Recipe::decode(&r.encode()), Ok(Some(r.clone())));
    assert_eq!(Recipe::decode(&[]), Ok(None), "prazdny recept = systemova relace");
    let enc = r.encode();
    for n in 1..enc.len() {
        assert!(Recipe::decode(&enc[..n]).is_err(), "zkraceni {n}");
    }
    let mut bad = enc.clone();
    *bad.last_mut().unwrap() = 7;
    assert!(Recipe::decode(&bad).is_err(), "neznamy rezim nahravani");
    let mut long = recipe();
    long.app_id = String::from_utf8(alloc::vec![b'a'; MAX_NAME + 1]).unwrap();
    assert!(Recipe::decode(&long.encode()).is_err());
    let mut empty = recipe();
    empty.app_id = String::new();
    assert!(Recipe::decode(&empty.encode()).is_err());
}

#[test]
fn app_cannot_touch_system_controls() {
    let fmt = Format::new(48_000, 2, Sample::S16);
    for ok in [Request::Open { dir: Direction::Output, format: fmt, latency_us: 50_000 }, Request::Start { stream: 1 }, Request::StreamVolume { stream: 1, percent: 50 }, Request::Status, Request::Devices] {
        assert!(app_allowed(&ok), "{ok:?}");
    }
    for no in [Request::MasterVolume { percent: 10 }, Request::Mute { on: true }, Request::Select { index: 0 }, Request::Connect] {
        assert!(!app_allowed(&no), "{no:?}");
    }
}

#[test]
fn capture_rules() {
    use CaptureStep::*;
    assert_eq!(capture_rule(Direction::Output, Capture::No, None), Open);
    assert_eq!(capture_rule(Direction::Input, Capture::No, Some(true)), Deny, "bez manifestu nic nepomuze");
    assert_eq!(capture_rule(Direction::Input, Capture::Ask, None), Ask);
    assert_eq!(capture_rule(Direction::Input, Capture::Ask, Some(true)), Open);
    assert_eq!(capture_rule(Direction::Input, Capture::Ask, Some(false)), Deny);
}
