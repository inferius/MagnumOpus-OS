//! Testy trid QoS.

use crate::class::{Class, background_floor_permille};

#[test]
fn raw_roundtrip() {
    for c in [Class::Realtime, Class::Interactive, Class::Normal, Class::Background] {
        assert_eq!(Class::from_raw(c as u8), Some(c));
    }
    assert_eq!(Class::from_raw(4), None);
}

#[test]
fn ordering_helpers() {
    assert_eq!(Class::Normal.max(Class::Interactive), Class::Interactive);
    assert_eq!(Class::Realtime.min(Class::Normal), Class::Normal);
    assert!(Class::Interactive.above(Class::Background));
    assert!(!Class::Background.above(Class::Background));
    assert_eq!(Class::Realtime.fair_index(), None);
    assert_eq!(Class::Background.fair_index(), Some(2));
}

#[test]
fn background_floor_is_five_percent() {
    assert_eq!(background_floor_permille(), 50);
}
