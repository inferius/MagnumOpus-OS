//! Testy protokolu listku a pravidel obnovy (na hostiteli).

use abi::Error;

use crate::policy::{self, After, Kind, after_loss};
use crate::proto::{MAX_WAIT_MS, Reply, Request, State, op};

#[test]
fn request_roundtrip() {
    for req in [
        Request::Reconnect { id: 7, wait_ms: 10_000 },
        Request::Narrow {
            id: 8,
            narrowing: b"/home/alice".to_vec(),
        },
        Request::Narrow {
            id: 9,
            narrowing: Vec::new(),
        },
        Request::Status { id: u32::MAX },
    ] {
        assert_eq!(Request::decode(&req.encode()), Ok(req.clone()));
    }
}

#[test]
fn reconnect_wait_is_capped() {
    let req = Request::Reconnect {
        id: 1,
        wait_ms: u32::MAX,
    };
    assert_eq!(
        Request::decode(&req.encode()),
        Ok(Request::Reconnect {
            id: 1,
            wait_ms: MAX_WAIT_MS
        })
    );
    assert_eq!(policy::wait_ms(core::time::Duration::from_secs(3600)), MAX_WAIT_MS);
    assert_eq!(policy::wait_ms(policy::DEFAULT_WAIT), 10_000);
}

#[test]
fn bad_requests_are_rejected() {
    assert_eq!(Request::decode(&[]), Err(Error::InvalidArgs));
    assert_eq!(Request::decode(&[1, 0, 0]), Err(Error::InvalidArgs));
    // Neznama operace.
    let mut w = wire::Writer::op(99);
    w.u32(1);
    assert_eq!(Request::decode(&w.finish()), Err(Error::InvalidArgs));
    // Prebytecna data za RECONNECT a STATUS.
    let mut w = wire::Writer::op(op::RECONNECT);
    w.u32(1).u32(5).u8(0);
    assert_eq!(Request::decode(&w.finish()), Err(Error::InvalidArgs));
    let mut w = wire::Writer::op(op::STATUS);
    w.u32(1).u8(0);
    assert_eq!(Request::decode(&w.finish()), Err(Error::InvalidArgs));
}

#[test]
fn reply_roundtrip() {
    for r in [
        Reply {
            op: op::RECONNECT,
            id: 3,
            status: Ok(()),
            generation: 2,
            state: State::Running,
        },
        Reply {
            op: op::RECONNECT,
            id: 4,
            status: Err(Error::ServiceUnavailable),
            generation: 5,
            state: State::GaveUp,
        },
        Reply {
            op: op::STATUS,
            id: 5,
            status: Ok(()),
            generation: 1,
            state: State::Restarting,
        },
        Reply {
            op: op::NARROW,
            id: 6,
            status: Err(Error::AccessDenied),
            generation: 1,
            state: State::Stopped,
        },
    ] {
        assert_eq!(Reply::decode(&r.encode()), Ok(r));
    }
}

#[test]
fn bad_replies_are_rejected() {
    let good = Reply {
        op: op::STATUS,
        id: 1,
        status: Ok(()),
        generation: 1,
        state: State::Running,
    }
    .encode();
    // Zkracena odpoved.
    for n in 0..good.len() {
        assert_eq!(Reply::decode(&good[..n]), Err(Error::InvalidArgs), "delka {n}");
    }
    // Neznamy stav sluzby.
    let mut bad = good.clone();
    *bad.last_mut().unwrap() = 9;
    assert_eq!(Reply::decode(&bad), Err(Error::InvalidArgs));
    // Neznamy kod chyby.
    let mut w = wire::Writer::op(op::STATUS);
    w.u32(1).i32(-9999).u64(1).u8(0);
    assert_eq!(Reply::decode(&w.finish()), Err(Error::InvalidArgs));
}

#[test]
fn handles_per_reply() {
    assert_eq!(Reply::handles_for(op::RECONNECT), 1);
    assert_eq!(Reply::handles_for(op::NARROW), 2);
    assert_eq!(Reply::handles_for(op::STATUS), 0);
}

#[test]
fn state_codes() {
    for s in [State::Running, State::Restarting, State::GaveUp, State::Stopped] {
        assert_eq!(State::from_u8(s.to_u8()), Ok(s));
    }
    assert_eq!(State::from_u8(4), Err(Error::InvalidArgs));
}

/// Odsouhlasena pravidla: opakovat jen idempotentni, s handly nikdy.
#[test]
fn retry_rules() {
    assert_eq!(after_loss(Kind::Idempotent, true, false), After::Resend);
    assert_eq!(
        after_loss(Kind::Effect, true, false),
        After::Fail(Error::ServiceRestarted)
    );
    // Pozadavek ke sluzbe nedorazil (zapis selhal): poslat znovu i s ucinkem.
    assert_eq!(after_loss(Kind::Effect, false, false), After::Resend);
    assert_eq!(after_loss(Kind::Idempotent, false, false), After::Resend);
    // Handly zanikly se starou instanci.
    for kind in [Kind::Idempotent, Kind::Effect] {
        for delivered in [true, false] {
            assert_eq!(
                after_loss(kind, delivered, true),
                After::Fail(Error::ServiceRestarted)
            );
        }
    }
}
