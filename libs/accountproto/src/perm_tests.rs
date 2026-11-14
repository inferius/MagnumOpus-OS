//! Povoleni aplikaci: zpravy a ulozeny tvar (perm.rs).

use alloc::vec;

use crate::perm::*;
use crate::{Request, msg};

#[test]
fn ask_answer_roundtrip_and_limits() {
    let a = Ask { id: 7, uid: 1001, app_id: "org.libsdl.testaudio".into(), app_name: "SDL testaudio".into(), perm: Perm::Microphone };
    assert_eq!(Ask::decode(&a.encode()), Ok(a.clone()));
    let enc = a.encode();
    for n in 0..enc.len() {
        assert!(Ask::decode(&enc[..n]).is_err());
    }
    let mut bad = a.clone();
    bad.app_id = "a\tb".into();
    assert!(Ask::decode(&bad.encode()).is_err(), "tabulator v identite");
    for d in [Decision::Deny, Decision::Allow, Decision::Once] {
        let x = Answer { id: 3, decision: d };
        assert_eq!(Answer::decode(&x.encode()), Ok(x));
    }
    assert!(Decision::from_u8(3).is_err());
}

#[test]
fn dialog_choices() {
    assert_eq!(DIALOG_OPTIONS[0], "Povolit");
    assert_eq!(decision_of_choice(0), Decision::Allow);
    assert_eq!(decision_of_choice(1), Decision::Once);
    assert_eq!(decision_of_choice(2), Decision::Deny, "Esc = posledni volba = nepovolit");
    assert_eq!(DIALOG_OPTIONS[2], "Nepovolit");
    assert_eq!(decision_of_choice(99), Decision::Deny, "zavreni dialogu = nepovolit");
}

#[test]
fn store_format() {
    let mut v = vec![];
    set(&mut v, "org.a", Perm::Microphone, true);
    set(&mut v, "org.b", Perm::Microphone, false);
    set(&mut v, "org.a", Perm::Microphone, false);
    assert_eq!(lookup(&v, "org.a", Perm::Microphone), Some(false), "posledni rozhodnuti plati");
    assert_eq!(lookup(&v, "org.c", Perm::Microphone), None);
    let text = serialize(&v);
    assert_eq!(text, "org.a\tmikrofon\tdeny\norg.b\tmikrofon\tdeny\n");
    assert_eq!(parse(&text), {
        let mut s = v.clone();
        s.sort_by(|a, b| a.app_id.cmp(&b.app_id));
        s
    });
    // Vadne radky se preskoci, CRLF toleruje.
    let p = parse("x\tmikrofon\tallow\r\nspatne\n\tmikrofon\tallow\ny\tkamera\tallow\nz\tmikrofon\tmozna\n");
    assert_eq!(p, vec![Entry { app_id: "x".into(), perm: Perm::Microphone, allow: true }]);
    assert_eq!(decode_entries(&encode_entries(&p)), Ok(p));
}

#[test]
fn self_requests() {
    for r in [Request::PermList, Request::PermRevoke { app_id: "org.a".into(), perm: Perm::Microphone }] {
        assert_eq!(Request::decode(&r.encode()), Ok(r.clone()));
    }
    let _ = msg::Request::PermList;
}

#[test]
fn channel_messages() {
    let a = Ask { id: 1, uid: 5, app_id: "org.a".into(), app_name: "A".into(), perm: Perm::Microphone };
    for m in [ChanMsg::Ask(a), ChanMsg::List { uid: 5 }, ChanMsg::Revoke { uid: 5, app_id: "org.a".into(), perm: Perm::Microphone }] {
        assert_eq!(ChanMsg::decode(&m.encode()), Ok(m.clone()));
    }
    assert!(ChanMsg::decode(&[]).is_err());
    assert!(ChanMsg::decode(&[9]).is_err());
}
