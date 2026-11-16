use super::*;

fn n(id: u64) -> Notification {
    Notification {
        id,
        app_id: "org.example.app".into(),
        app_name: "Příklad".into(),
        title: "Stahování dokončeno".into(),
        body: "soubor.zip\n12 MB".into(),
        priority: Priority::High,
        actions: alloc::vec![Action { id: "open".into(), label: "Otevřít".into() }],
        time: 1_700_000_000_000_000_000,
        popup: true,
        replaced: false,
        seen: false,
    }
}

#[test]
fn send_round_trip() {
    for m in [
        Send::Notify { key: 7, title: "T".into(), body: "B".into(), priority: Priority::Low, actions: alloc::vec![] },
        Send::Notify { key: 1, title: "x".into(), body: "".into(), priority: Priority::Urgent, actions: n(0).actions },
        Send::Close { key: 9 },
    ] {
        assert_eq!(Send::decode(&m.encode()), Ok(m));
    }
    for m in [
        SendReply::Sent { key: 1, popup: false },
        SendReply::Error { key: 2, error: Error::NoResources },
        SendReply::Action { key: 3, action: "open".into() },
        SendReply::Closed { key: 4, why: Closed::User },
    ] {
        assert_eq!(SendReply::decode(&m.encode()), Ok(m));
    }
}

#[test]
fn view_round_trip() {
    for m in [View::Subscribe, View::Dismiss { id: 3 }, View::Invoke { id: 4, action: "".into() }, View::DismissAll, View::SetDnd { until: u64::MAX }, View::Seen { id: 5 }] {
        assert_eq!(View::decode(&m.encode()), Ok(m));
    }
    for m in [
        ViewReply::Snapshot { list: alloc::vec![n(1), n(2)], last: true },
        ViewReply::Added(n(3)),
        ViewReply::Removed { id: 3, why: Closed::Limit },
        ViewReply::Dnd { until: 0 },
        ViewReply::Result { op: op::DISMISS, error: None },
        ViewReply::Result { op: op::INVOKE, error: Some(Error::NotFound) },
    ] {
        assert_eq!(ViewReply::decode(&m.encode()), Ok(m));
    }
}

#[test]
fn garbage_is_error() {
    let good = Send::Notify { key: 1, title: "t".into(), body: "b".into(), priority: Priority::Normal, actions: n(0).actions }.encode();
    for i in 0..good.len() {
        let _ = Send::decode(&good[..i]);
    }
    assert_eq!(Send::decode(&[0xFF, 0, 0, 0]), Err(Error::InvalidArgs));
    // Pocet akci nad MAX_ACTIONS.
    let mut w = Writer::op(op::NOTIFY);
    w.u64(1).str("t").str("b").u8(1).u8(9);
    assert_eq!(Send::decode(&w.finish()), Err(Error::InvalidArgs));
    let v = ViewReply::Snapshot { list: alloc::vec![n(1)], last: false }.encode();
    for i in 0..v.len() {
        let _ = ViewReply::decode(&v[..i]);
    }
}

#[test]
fn clean_strips_and_truncates() {
    assert_eq!(clean("a\u{7}b\nc", 10, false), "abc");
    assert_eq!(clean("a\nb", 10, true), "a\nb");
    assert_eq!(clean("žluťoučký kůň", 8, false), "žluťo...");
    assert_eq!(clean("krátké", 6, false), "krátké");
}
