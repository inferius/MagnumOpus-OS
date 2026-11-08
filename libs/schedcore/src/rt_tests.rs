//! Testy Realtime rozpoctu.

use crate::rt::{Admission, Rt, params_from_us};

#[test]
fn params_validation() {
    assert_eq!(params_from_us(0, 0), Some((1_000_000, 10_000_000)));
    assert_eq!(params_from_us(1000, 5000), Some((1_000_000, 5_000_000)));
    assert_eq!(params_from_us(3000, 5000), None, "nad polovinu periody");
    assert_eq!(params_from_us(10, 5000), None, "rozpocet pod minimum");
    assert_eq!(params_from_us(100, 100), None, "perioda pod minimum");
    assert_eq!(params_from_us(0, 5000), None);
}

#[test]
fn budget_refresh_and_cbs_wake() {
    let mut rt = Rt::new(1000, 10_000);
    rt.refresh(0);
    assert_eq!(rt.remaining, 1000);
    rt.charge(400);
    assert_eq!(rt.remaining, 600);
    // Probuzeni tesne pred koncem periody s velkym zbytkem: nova perioda.
    rt.wake(9_000);
    assert_eq!(rt.period_end, 19_000);
    assert_eq!(rt.remaining, 1000);
    // Probuzeni brzy s malym zbytkem: perioda zustava.
    rt.charge(900);
    rt.wake(10_000);
    assert_eq!(rt.period_end, 19_000);
    assert_eq!(rt.remaining, 100);
}

#[test]
fn admission_limits_total() {
    let a = Admission::new();
    assert!(a.admit(300, 1));
    assert!(a.admit(200, 1));
    assert!(!a.admit(1, 1), "nad 50 % jednoho CPU");
    a.release(200);
    assert!(a.admit(200, 1));
    assert!(a.admit(1000, 4));
}
