use super::*;
use alloc::string::ToString;
use alloc::vec;
use notifyproto::Action;

const S: u64 = 1_000_000_000;
const T0: u64 = 1_700_000_000 * S;

fn app(id: &str, uid: u64) -> Identity {
    Identity { app_id: id.to_string(), app_name: id.to_string(), uid, system: false }
}

fn sys() -> Identity {
    Identity { app_id: "system".into(), app_name: "Systém".into(), uid: 0, system: true }
}

fn notify(key: u64, title: &str, p: Priority) -> Vec<u8> {
    Send::Notify { key, title: title.into(), body: "telo".into(), priority: p, actions: vec![Action { id: "open".into(), label: "Otevřít".into() }] }.encode()
}

fn sender_replies(out: &[Out], s: SenderId) -> Vec<SendReply> {
    out.iter().filter_map(|o| if let Out::Sender(x, d) = o && *x == s { Some(SendReply::decode(d).unwrap()) } else { None }).collect()
}

fn viewer_replies(out: &[Out], v: ViewerId) -> Vec<ViewReply> {
    out.iter().filter_map(|o| if let Out::Viewer(x, d) = o && *x == v { Some(ViewReply::decode(d).unwrap()) } else { None }).collect()
}

fn added(out: &[Out], v: ViewerId) -> Vec<Notification> {
    viewer_replies(out, v).into_iter().filter_map(|r| if let ViewReply::Added(n) = r { Some(n) } else { None }).collect()
}

#[test]
fn send_view_invoke_dismiss() {
    let mut c = Core::new();
    let s = c.add_sender(app("org.example.dl", 1000));
    let v = c.add_viewer(1000);
    let other = c.add_viewer(1001);
    let out = c.viewer_msg(v, &View::Subscribe.encode(), T0);
    assert_eq!(viewer_replies(&out, v), vec![ViewReply::Snapshot { list: vec![], last: true }, ViewReply::Dnd { until: 0 }]);
    c.viewer_msg(other, &View::Subscribe.encode(), T0);
    let out = c.sender_msg(s, &notify(7, "Stahování", Priority::Normal), T0, 0);
    assert_eq!(sender_replies(&out, s), vec![SendReply::Sent { key: 7, popup: true }]);
    let n = added(&out, v);
    assert_eq!((n.len(), n[0].popup, n[0].app_id.as_str(), n[0].replaced), (1, true, "org.example.dl", false));
    // Jiny uzivatel ho nevidi.
    assert!(viewer_replies(&out, other).is_empty());
    // Nahrazeni (prubeh): stejne cislo, nevyskakuje znovu.
    let out = c.sender_msg(s, &notify(7, "Stahování 50 %", Priority::Normal), T0, 1);
    let m = added(&out, v);
    assert_eq!((m[0].id, m[0].replaced, m[0].title.as_str()), (n[0].id, true, "Stahování 50 %"));
    assert_eq!(c.len(), 1);
    // Klik na akci: odesilatel dostane akci, prohlizec odebrani.
    let out = c.viewer_msg(v, &View::Invoke { id: n[0].id, action: "open".into() }.encode(), T0);
    assert_eq!(sender_replies(&out, s), vec![SendReply::Action { key: 7, action: "open".into() }]);
    assert!(viewer_replies(&out, v).contains(&ViewReply::Removed { id: n[0].id, why: Closed::User }));
    assert!(c.is_empty());
    // Neznama akce a cizi oznameni.
    let out = c.sender_msg(s, &notify(8, "Druhé", Priority::Normal), T0, 2);
    let id = added(&out, v)[0].id;
    let out = c.viewer_msg(v, &View::Invoke { id, action: "smazat".into() }.encode(), T0);
    assert_eq!(viewer_replies(&out, v)[0], ViewReply::Result { op: op::INVOKE, error: Some(Error::InvalidArgs) });
    let out = c.viewer_msg(other, &View::Dismiss { id }.encode(), T0);
    assert_eq!(viewer_replies(&out, other)[0], ViewReply::Result { op: op::DISMISS, error: Some(Error::NotFound) });
    // Zavreni krizkem: odesilatel to vi.
    let out = c.viewer_msg(v, &View::Dismiss { id }.encode(), T0);
    assert_eq!(sender_replies(&out, s), vec![SendReply::Closed { key: 8, why: Closed::User }]);
    // Zavreni odesilatelem.
    let out = c.sender_msg(s, &notify(9, "Třetí", Priority::Low), T0, 3);
    assert_eq!(sender_replies(&out, s), vec![SendReply::Sent { key: 9, popup: false }]);
    let out = c.sender_msg(s, &Send::Close { key: 9 }.encode(), T0, 3);
    assert!(matches!(viewer_replies(&out, v)[0], ViewReply::Removed { why: Closed::Sender, .. }));
}

#[test]
fn identity_survives_reconnect_and_system_is_shared() {
    let mut c = Core::new();
    let s = c.add_sender(app("org.example.dl", 1000));
    let v = c.add_viewer(1000);
    let v2 = c.add_viewer(1001);
    c.viewer_msg(v, &View::Subscribe.encode(), T0);
    c.viewer_msg(v2, &View::Subscribe.encode(), T0);
    c.sender_msg(s, &notify(1, "A", Priority::Normal), T0, 0);
    c.remove_sender(s);
    // Po restartu aplikace stejna identita nahradi sve oznameni.
    let s2 = c.add_sender(app("org.example.dl", 1000));
    let out = c.sender_msg(s2, &notify(1, "A znovu", Priority::Normal), T0, 1);
    assert!(added(&out, v)[0].replaced);
    assert_eq!(c.len(), 1);
    // Systemove oznameni vidi vsichni; Urgent jen system.
    let sy = c.add_sender(sys());
    let out = c.sender_msg(sy, &notify(1, "Baterie", Priority::Urgent), T0, 2);
    assert_eq!((added(&out, v).len(), added(&out, v2).len()), (1, 1));
    assert_eq!(added(&out, v)[0].priority, Priority::Urgent);
    let out = c.sender_msg(s2, &notify(2, "Falešně naléhavé", Priority::Urgent), T0, 3);
    assert_eq!(added(&out, v)[0].priority, Priority::High);
}

#[test]
fn dnd_suppresses_popup_except_urgent() {
    let mut c = Core::new();
    let s = c.add_sender(app("a", 1000));
    let sy = c.add_sender(sys());
    let v = c.add_viewer(1000);
    c.viewer_msg(v, &View::Subscribe.encode(), T0);
    let out = c.viewer_msg(v, &View::SetDnd { until: T0 + 60 * S }.encode(), T0);
    assert!(viewer_replies(&out, v).contains(&ViewReply::Dnd { until: T0 + 60 * S }));
    let out = c.sender_msg(s, &notify(1, "tiché", Priority::High), T0 + S, 0);
    assert_eq!(sender_replies(&out, s), vec![SendReply::Sent { key: 1, popup: false }]);
    assert!(!added(&out, v)[0].popup);
    let out = c.sender_msg(sy, &notify(1, "naléhavé", Priority::Urgent), T0 + S, 0);
    assert!(added(&out, v)[0].popup);
    // Konec Nerusit.
    let (out, next) = c.tick(T0 + 30 * S);
    assert!(out.is_empty() && next == Some(T0 + 60 * S));
    let (out, next) = c.tick(T0 + 61 * S);
    assert_eq!(viewer_replies(&out, v), vec![ViewReply::Dnd { until: 0 }]);
    assert_eq!(next, None);
    // Snimek po restartu prohlizece: uz videne nevyskoci.
    let id = c.list[1].n.id;
    c.viewer_msg(v, &View::Seen { id }.encode(), T0);
    let v2 = c.add_viewer(1000);
    let out = c.viewer_msg(v2, &View::Subscribe.encode(), T0 + 62 * S);
    let ViewReply::Snapshot { list, last: true } = &viewer_replies(&out, v2)[0] else { panic!() };
    assert_eq!(list.len(), 2);
    assert!(list.iter().all(|n| !n.popup), "tiché bylo v Nerušit, naléhavé už viděné");
    assert_eq!(list.iter().filter(|n| n.seen).count(), 1, "videne jen naléhavé");
}

#[test]
fn limits_rate_and_count() {
    let mut c = Core::new();
    let s = c.add_sender(app("spam", 1000));
    let v = c.add_viewer(1000);
    c.viewer_msg(v, &View::Subscribe.encode(), T0);
    let mut pops = 0;
    for k in 0..10 {
        let out = c.sender_msg(s, &notify(k, "x", Priority::Normal), T0, 0);
        if added(&out, v)[0].popup {
            pops += 1;
        }
    }
    assert_eq!(pops, BURST, "vyskoci jen zasobnik");
    // Po case se doplni.
    let out = c.sender_msg(s, &notify(100, "x", Priority::Normal), T0, 2 * REFILL);
    assert!(added(&out, v)[0].popup);
    // Pocet na identitu: nejstarsi vytlaceny, odesilatel to vi.
    for k in 200..230 {
        c.sender_msg(s, &notify(k, "x", Priority::Low), T0, 3 * REFILL);
    }
    assert_eq!(c.len(), MAX_PER_IDENTITY);
    let out = c.sender_msg(s, &notify(300, "x", Priority::Low), T0, 3 * REFILL);
    assert!(sender_replies(&out, s).iter().any(|r| matches!(r, SendReply::Closed { why: Closed::Limit, .. })));
    // Na uzivatele (mnoho aplikaci).
    for a in 0..15 {
        let s = c.add_sender(app(&alloc::format!("app{a}"), 1000));
        for k in 0..20 {
            c.sender_msg(s, &notify(k, "x", Priority::Low), T0, 0);
        }
    }
    assert_eq!(c.list.iter().filter(|e| e.uid == 1000).count(), MAX_PER_USER);
}

#[test]
fn bad_input() {
    let mut c = Core::new();
    let s = c.add_sender(app("a", 1000));
    let out = c.sender_msg(s, &notify(1, "   ", Priority::Normal), T0, 0);
    assert_eq!(sender_replies(&out, s), vec![SendReply::Error { key: 1, error: Error::InvalidArgs }]);
    let dup = Send::Notify {
        key: 2,
        title: "t".into(),
        body: "".into(),
        priority: Priority::Normal,
        actions: vec![Action { id: "a".into(), label: "A".into() }, Action { id: "a".into(), label: "B".into() }],
    };
    let out = c.sender_msg(s, &dup.encode(), T0, 0);
    assert_eq!(sender_replies(&out, s), vec![SendReply::Error { key: 2, error: Error::InvalidArgs }]);
    let out = c.sender_msg(s, &[1, 2, 3], T0, 0);
    assert_eq!(sender_replies(&out, s).len(), 1);
    // Dlouhy text se zkrati, ridici znaky pryc.
    let v = c.add_viewer(1000);
    c.viewer_msg(v, &View::Subscribe.encode(), T0);
    let long = Send::Notify { key: 3, title: "x\u{1b}[31m".repeat(100), body: "b".repeat(5000), priority: Priority::Normal, actions: vec![] };
    let out = c.sender_msg(s, &long.encode(), T0, 0);
    let n = &added(&out, v)[0];
    assert!(n.title.chars().count() <= MAX_TITLE && !n.title.contains('\u{1b}'));
    assert_eq!(n.body.chars().count(), MAX_BODY);
    // Neplatna zprava prohlizece.
    let out = c.viewer_msg(v, &[9, 9], T0);
    assert!(matches!(viewer_replies(&out, v)[0], ViewReply::Result { error: Some(_), .. }));
}

#[test]
fn snapshot_in_chunks_and_dismiss_all() {
    let mut c = Core::new();
    for a in 0..5 {
        let s = c.add_sender(app(&alloc::format!("a{a}"), 1000));
        for k in 0..15 {
            c.sender_msg(s, &notify(k, "x", Priority::Low), T0, 0);
        }
    }
    let v = c.add_viewer(1000);
    let out = c.viewer_msg(v, &View::Subscribe.encode(), T0);
    let r = viewer_replies(&out, v);
    let total: usize = r.iter().map(|x| if let ViewReply::Snapshot { list, .. } = x { list.len() } else { 0 }).sum();
    assert_eq!(total, 75);
    assert!(matches!(r[r.len() - 2], ViewReply::Snapshot { last: true, .. }));
    // Kazda cast se vejde do zpravy kanalu.
    assert!(out.iter().all(|o| match o {
        Out::Viewer(_, d) => d.len() < 64 * 1024,
        _ => true,
    }));
    c.viewer_msg(v, &View::DismissAll.encode(), T0);
    assert!(c.is_empty());
}
