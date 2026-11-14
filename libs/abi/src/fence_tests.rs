//! Testy stavoveho automatu plotu (`abi::fence`).

use crate::error::Error;
use crate::fence::{Info, Timeline, WaitItem};

#[test]
fn binary_fence() {
    let mut t = Timeline::new(0);
    assert!(!t.signaled());
    assert_eq!(t.check(1), None);
    assert_eq!(t.check(0), Some(Ok(())));
    t.signal(1, false).unwrap();
    assert!(t.signaled());
    assert_eq!(t.check(1), Some(Ok(())));
    // Opakovany signal stejne hodnoty nic nemeni.
    t.signal(1, false).unwrap();
    assert_eq!(t.value(), 1);
}

#[test]
fn timeline_is_monotonic() {
    let mut t = Timeline::new(5);
    assert!(t.signaled());
    assert_eq!(t.signal(4, false), Err(Error::InvalidArgs));
    t.signal(9, false).unwrap();
    assert_eq!(t.check(9), Some(Ok(())));
    assert_eq!(t.check(10), None);
    t.signal(u64::MAX, false).unwrap();
    assert_eq!(t.check(u64::MAX), Some(Ok(())));
}

#[test]
fn error_is_sticky() {
    let mut t = Timeline::new(0);
    t.signal(3, false).unwrap();
    // Chyba musi lezet za dosazenou hodnotou.
    assert_eq!(t.signal(3, true), Err(Error::InvalidArgs));
    t.signal(5, true).unwrap();
    assert_eq!(t.value(), 3);
    // Body pred chybou jsou dosazene, vsechno dal skonci chybou.
    assert_eq!(t.check(3), Some(Ok(())));
    assert_eq!(t.check(4), Some(Err(Error::FenceFailed)));
    assert_eq!(t.check(100), Some(Err(Error::FenceFailed)));
    // Po chybe uz nic.
    assert_eq!(t.signal(10, false), Err(Error::BadState));
    assert_eq!(t.signal(10, true), Err(Error::BadState));
    // Opusteni po chybe chybu neprepise.
    t.abandon();
    assert_eq!(t.status(), Some(Error::FenceFailed));
    let i = t.info();
    assert_eq!((i.value, i.failed_at, i.status), (3, 5, -37));
    assert_eq!(i.error(), Some(Error::FenceFailed));
}

#[test]
fn binary_failure_signals() {
    let mut t = Timeline::new(0);
    t.signal(1, true).unwrap();
    assert!(t.signaled());
    assert_eq!(t.check(1), Some(Err(Error::FenceFailed)));
}

#[test]
fn abandoned_fails_only_unreached() {
    let mut t = Timeline::new(0);
    t.signal(2, false).unwrap();
    t.abandon();
    assert_eq!(t.check(2), Some(Ok(())));
    assert_eq!(t.check(3), Some(Err(Error::FenceAbandoned)));
    assert_eq!(t.status(), Some(Error::FenceAbandoned));
    let mut b = Timeline::new(0);
    b.abandon();
    assert!(b.signaled());
    assert_eq!(b.info().status, Error::FenceAbandoned.code());
}

#[test]
fn encoding_roundtrip() {
    let w = WaitItem::new(0x1234, u64::MAX - 1);
    assert_eq!(WaitItem::decode(&w.encode()), w);
    let i = Info {
        value: 7,
        failed_at: 9,
        status: -37,
        _reserved: 0,
    };
    assert_eq!(Info::decode(&i.encode()), i);
    assert_eq!(Info::default().error(), None);
}
