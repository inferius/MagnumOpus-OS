//! Hlavicka netproto v2 (cislo dotazu, termin, zruseni) a limit
//! soubeznych dotazu relace.

use core::time::Duration;

use abi::Error;

use crate::auth::{Auth, DEFAULT_OPS, MAX_OPS, Quota, Restriction};
use crate::msg::{self, Header, NO_DEADLINE, REPLY_HEADER, REQUEST_HEADER, TRY};
use crate::{Writer, op, rights};

#[test]
fn request_header_roundtrip() {
    let h = Header {
        op: op::RECV,
        id: 7,
        deadline: 123_456_789,
    };
    let m = msg::request(h, &[1, 2, 3]);
    assert_eq!(m.len(), REQUEST_HEADER + 3);
    let (got, mut r) = msg::parse_request(&m).unwrap();
    assert_eq!(got, h);
    assert_eq!(r.rest(), &[1, 2, 3]);
    // Cislo 0 neni platne (klient ho nikdy nepouzije).
    let zero = msg::request(Header { id: 0, ..h }, &[]);
    assert_eq!(msg::parse_request(&zero).err(), Some(Error::InvalidArgs));
    // Zkracena hlavicka.
    assert!(msg::parse_request(&m[..REQUEST_HEADER - 1]).is_err());
}

#[test]
fn reply_carries_id_and_status() {
    let mut body = Writer::new();
    body.u32(42);
    let ok = msg::reply(9, Ok(body));
    assert_eq!(msg::reply_id(&ok), Some(9));
    assert_eq!(msg::parse_reply(&ok).unwrap().u32().unwrap(), 42);
    let err = msg::reply(10, Err(Error::Canceled));
    assert_eq!(err.len(), REPLY_HEADER);
    assert_eq!(msg::reply_id(&err), Some(10));
    assert_eq!(msg::parse_reply(&err).err(), Some(Error::Canceled));
    assert_eq!(msg::reply_id(&[1, 2]), None);
}

#[test]
fn cancel_is_reserved_op() {
    // CANCEL nesmi kolidovat s zadnou operaci.
    for code in [
        op::INFO,
        op::WAIT_CONFIGURED,
        op::TCP_CONNECT,
        op::RESOLVE,
        op::SEND,
        op::RECV,
        op::ACCEPT,
        op::RECV_FROM,
        op::SEND_TO,
    ] {
        assert_ne!(code, op::CANCEL);
    }
    let m = msg::request(
        Header {
            op: op::CANCEL,
            id: 5,
            deadline: NO_DEADLINE,
        },
        &[],
    );
    assert_eq!(m.len(), REQUEST_HEADER);
}

#[test]
fn deadlines() {
    assert_eq!(msg::deadline_after(1000, None), NO_DEADLINE);
    assert_eq!(msg::deadline_after(1000, Some(Duration::ZERO)), 1000);
    assert_eq!(msg::deadline_after(1000, Some(Duration::from_micros(2))), 3000);
    // Konecny termin nikdy nesplyne s "bez limitu".
    assert_eq!(msg::deadline_after(u64::MAX - 5, Some(Duration::from_secs(1))), NO_DEADLINE - 1);
    assert_eq!(TRY, 0);
}

#[test]
fn session_ops_limit() {
    let root = Auth::root();
    assert_eq!(root.budget().ops_limit(), DEFAULT_OPS);
    let child = root
        .restrict_with(Restriction {
            rights: rights::ALL,
            quota: Quota {
                ops: 2,
                ..Quota::UNLIMITED
            },
            ..Restriction::default()
        })
        .unwrap();
    let a = child.begin_op().unwrap();
    let b = child.begin_op().unwrap();
    assert_eq!(child.begin_op().err(), Some(Error::NoResources), "limit relace");
    // Limit je jen pro tuto relaci, rodic ho nevidi.
    let r = root.begin_op().unwrap();
    assert_eq!(root.budget().ops_used(), 1);
    drop(a);
    let c = child.begin_op().unwrap();
    assert_eq!(child.budget().ops_used(), 2);
    drop((b, c, r));
    assert_eq!(child.budget().ops_used(), 0);
    // Meze: 0 = aspon jeden, nad MAX_OPS se omezi.
    let zero = root
        .restrict_with(Restriction {
            rights: rights::ALL,
            quota: Quota { ops: 0, ..Quota::UNLIMITED },
            ..Restriction::default()
        })
        .unwrap();
    assert_eq!(zero.budget().ops_limit(), 1);
    let huge = root
        .restrict_with(Restriction {
            rights: rights::ALL,
            quota: Quota { ops: u32::MAX, ..Quota::UNLIMITED },
            ..Restriction::default()
        })
        .unwrap();
    assert_eq!(huge.budget().ops_limit(), MAX_OPS);
}
