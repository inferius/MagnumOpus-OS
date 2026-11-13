//! Testy protokolu GUI (na hostiteli: `cargo test -p guiproto`).

use crate::a11y::{self, Node, Role, Tree};
use crate::action::{self, ParamType, Value};
use crate::control::*;
use crate::display::{self, Output, Reply, Request};
use crate::window::*;
use crate::{Error, Rect};

#[test]
fn rect_math() {
    let a = Rect::new(0, 0, 10, 10);
    let b = Rect::new(5, 5, 10, 10);
    assert_eq!(a.intersect(&b), Rect::new(5, 5, 5, 5));
    assert_eq!(a.union(&b), Rect::new(0, 0, 15, 15));
    assert!(a.intersect(&Rect::new(20, 20, 1, 1)).is_empty());
    assert_eq!(Rect::default().union(&b), b);
    assert!(a.contains(9, 9) && !a.contains(10, 0));
    // Bez preteceni na krajich.
    let huge = Rect::new(i32::MAX - 1, 0, u32::MAX, 1);
    assert_eq!(huge.right(), i32::MAX as i64 - 1 + u32::MAX as i64);
    assert_eq!(huge.offset(10, 0).x, i32::MAX);
}

fn roundtrip_client(m: ClientMsg) {
    let enc = m.encode();
    assert_eq!(ClientMsg::decode(&enc), Ok(m.clone()), "{m:?}");
    // Kazde zkraceni je chyba, nikdy panika.
    for n in 0..enc.len() {
        assert!(ClientMsg::decode(&enc[..n]).is_err());
    }
}

fn roundtrip_server(m: ServerMsg) {
    let enc = m.encode();
    assert_eq!(ServerMsg::decode(&enc), Ok(m.clone()), "{m:?}");
    for n in 0..enc.len() {
        assert!(ServerMsg::decode(&enc[..n]).is_err());
    }
}

fn sample_node() -> Node {
    let mut n = Node::new(2, 1, Role::Button, "Uložit");
    n.state = a11y::state::FOCUSED;
    n.layer = 5;
    n.bounds = Rect::new(10, 20, 100, 30);
    n.actions = a11y::actions::ACTIVATE;
    n
}

#[test]
fn client_messages_roundtrip() {
    let props = LayerProps {
        mask: prop::RECT | prop::OPACITY | prop::SCROLL | prop::BUFFER,
        rect: Rect::new(-5, 3, 100, 50),
        opacity: 128,
        scroll: (0, -40),
        buffer: 7,
        src: Rect::new(0, 0, 100, 50),
        ..Default::default()
    };
    for m in [
        ClientMsg::Hello { version: crate::VERSION },
        ClientMsg::WindowCreate {
            window: 1,
            mode: Mode::CustomTitle,
            w: 640,
            h: 480,
            title: "Poznámky".into(),
        },
        ClientMsg::WindowSet {
            window: 1,
            title: "x".into(),
            min_w: 100,
            min_h: 50,
            side: ButtonSide::Left,
            button_color: 0xFF00_8000,
        },
        ClientMsg::WindowDestroy { window: 1 },
        ClientMsg::BufferCreate {
            buffer: 3,
            w: 64,
            h: 32,
            stride: 256,
        },
        ClientMsg::BufferDestroy { buffer: 3 },
        ClientMsg::LayerCreate {
            layer: 4,
            window: 1,
            parent: 0,
            kind: LayerKind::Solid,
        },
        ClientMsg::LayerSet { layer: 4, props },
        ClientMsg::LayerDestroy { layer: 4 },
        ClientMsg::Damage {
            buffer: 3,
            rect: Rect::new(1, 2, 3, 4),
        },
        ClientMsg::Animate {
            layer: 4,
            prop: AnimProp::ScrollY,
            target: -100,
            ms: 200,
            curve: Curve::EaseOut,
        },
        ClientMsg::A11yUpdate {
            window: 1,
            nodes: vec![Node::new(1, 0, Role::Window, "Okno"), sample_node()],
        },
        ClientMsg::A11yRemove { window: 1, ids: vec![2, 3] },
        ClientMsg::Regions {
            window: 1,
            regions: vec![(Rect::new(0, 0, 200, 32), RegionKind::Drag), (Rect::new(10, 0, 50, 32), RegionKind::NoDrag)],
        },
        ClientMsg::Commit {
            txn: 9,
            flags: commit::FRAME | commit::GROUP,
            window: 1,
            serial: 4,
            token: u128::MAX - 5,
        },
        ClientMsg::SyncGroup { count: 2 },
        ClientMsg::Pong { serial: 8 },
        ClientMsg::ActionResult {
            call: 1,
            ok: true,
            value: Some(Value::Int(-3)),
            message: "hotovo".into(),
        },
        ClientMsg::Clipboard { data: vec![1, 0, 0, 0, 0] },
        ClientMsg::Clipboard { data: vec![] },
        ClientMsg::WindowSize { window: 1, w: 1040, h: 520, min_w: 400, min_h: 200, max_frac_w: 850, max_frac_h: 0 },
    ] {
        roundtrip_client(m);
    }
}

#[test]
fn server_messages_roundtrip() {
    for m in [
        ServerMsg::Welcome {
            version: 1,
            w: 1280,
            h: 800,
            scale: 100,
            app: "Ukázka".into(),
        },
        ServerMsg::Configure {
            window: 1,
            w: 300,
            h: 200,
            state: wstate::FOCUSED,
            serial: 3,
        },
        ServerMsg::Frame {
            seq: 5,
            deadline: 1_000_000,
            period: 16_666_667,
        },
        ServerMsg::Presented { txn: 2, time: 99 },
        ServerMsg::Released { buffer: 1 },
        ServerMsg::Input {
            window: 1,
            records: vec![
                InputRecord {
                    event: InputEvent::Key {
                        code: 30,
                        action: 1,
                        ch: Some('á'),
                    },
                    mods: 1,
                    synthetic: false,
                    time: 10,
                    class: 0,
                    serial: 77,
                },
                InputRecord {
                    event: InputEvent::Button {
                        code: 0x110,
                        pressed: true,
                        x: -1,
                        y: 7,
                    },
                    mods: 0,
                    synthetic: true,
                    time: 11,
                    class: 2,
                    serial: 0,
                },
                InputRecord {
                    event: InputEvent::PointerLeave,
                    mods: 0,
                    synthetic: false,
                    time: 12,
                    class: 0,
                    serial: 0,
                },
            ],
        },
        ServerMsg::Close { window: 1 },
        ServerMsg::Ping { serial: 2 },
        ServerMsg::SyncToken { token: 1 << 100 },
        ServerMsg::AnimationDone { layer: 3 },
        ServerMsg::A11yAction {
            window: 1,
            node: 2,
            action: a11y::actions::SET_VALUE,
            value: "text".into(),
        },
        ServerMsg::ActionInvoke {
            call: 4,
            action: "note.create".into(),
            args: vec![("title".into(), Value::Text("A".into())), ("pin".into(), Value::Bool(true))],
            caller: "shell".into(),
            synthetic: false,
        },
        ServerMsg::Error {
            op: op::LAYER_SET,
            code: Error::InvalidArgs,
        },
        ServerMsg::Clipboard { data: vec![104, 0, 0, 0] },
        ServerMsg::WindowPlace { window: 1, rect: Rect::new(-20, 40, 300, 200) },
        ServerMsg::WindowContext {
            window: 1,
            generation: 3,
            output: 0,
            screen: Rect::new(0, 0, 1280, 800),
            work_area: Rect::new(0, 0, 1280, 744),
            scale: 125,
            refresh_mhz: 60_000,
            ext: crate::context::ContextExt {
                posture: crate::context::Posture::Tablet,
                distance: crate::context::Distance::Near,
                safe: crate::context::Insets { top: 27, right: 48, bottom: 27, left: 48 },
                pointers: vec![
                    crate::context::Pointer { kind: crate::context::PointerKind::Touch, precise: false, hover: false, pressure: false },
                    crate::context::Pointer { kind: crate::context::PointerKind::Pen, precise: true, hover: true, pressure: true },
                ],
                keyboard: false,
                obscured: vec![(crate::context::ObstacleKind::Keyboard, Rect::new(0, 400, 1280, 330))],
                tile: crate::context::Tile::Left,
                min_hit: 44,
                ..Default::default()
            },
        },
    ] {
        roundtrip_server(m);
    }
}

#[test]
fn bad_messages_are_errors() {
    assert_eq!(ClientMsg::decode(&[]), Err(Error::InvalidArgs));
    assert_eq!(ClientMsg::decode(&999u32.to_le_bytes()), Err(Error::NotSupported));
    // Neznamy bit masky vlastnosti.
    let mut w = wire::Writer::op(op::LAYER_SET);
    w.u32(1).u32(1 << 20);
    assert_eq!(ClientMsg::decode(&w.finish()), Err(Error::InvalidArgs));
    // Obri pocet uzlu v kratke zprave neni alokace na miliardy prvku.
    let mut w = wire::Writer::op(op::A11Y_UPDATE);
    w.u32(1).u32(u32::MAX);
    assert_eq!(ClientMsg::decode(&w.finish()), Err(Error::InvalidArgs));
    // Data navic za zpravou.
    let mut enc = ClientMsg::Hello { version: crate::VERSION }.encode();
    enc.push(0);
    assert_eq!(ClientMsg::decode(&enc), Err(Error::InvalidArgs));
    // Neplatny bool.
    let mut w = wire::Writer::op(op::REGIONS);
    w.u32(1).u32(1).i32(0).i32(0).u32(1).u32(1).u8(9);
    assert_eq!(ClientMsg::decode(&w.finish()), Err(Error::InvalidArgs));
}

#[test]
fn layer_props_apply_mask() {
    let mut p = LayerProps::default();
    p.apply(&LayerProps {
        mask: prop::OPACITY,
        opacity: 10,
        rect: Rect::new(1, 1, 1, 1),
        ..Default::default()
    });
    assert_eq!(p.opacity, 10);
    assert_eq!(p.rect, Rect::default(), "RECT neni v masce");
    assert_eq!(p.mask, prop::OPACITY);
}

#[test]
fn curves() {
    assert_eq!(Curve::Linear.eval(500), 500);
    assert_eq!(Curve::EaseOut.eval(0), 0);
    assert_eq!(Curve::EaseOut.eval(1000), 1000);
    assert_eq!(Curve::EaseOut.eval(5000), 1000);
    assert!(Curve::EaseOut.eval(500) > 800, "ease-out je na zacatku rychly");
}

#[test]
fn a11y_tree_update_remove_find() {
    let mut t = Tree::new();
    t.update([Node::new(1, 0, Role::Window, "W"), sample_node(), Node::new(3, 2, Role::Label, "child")]);
    assert_eq!(t.len(), 3);
    assert_eq!(t.find(Role::Button, "Uložit").map(|n| n.id), Some(2));
    // Nahrada stejneho id.
    t.update([Node::new(2, 1, Role::Button, "Zrušit")]);
    assert!(t.find(Role::Button, "Uložit").is_none());
    assert_eq!(t.children(1).count(), 1);
    // Odebrani i s potomky.
    t.remove(&[2]);
    assert_eq!(t.len(), 1);
    // Id 0 se ignoruje.
    t.update([Node::new(0, 0, Role::Label, "x")]);
    assert_eq!(t.len(), 1);
}

#[test]
fn a11y_messages_split_under_limit() {
    let nodes: Vec<Node> = (1..2000)
        .map(|i| {
            let mut n = Node::new(i, 0, Role::ListItem, &"x".repeat(100));
            n.value = "y".repeat(50);
            n
        })
        .collect();
    let msgs = a11y_messages(1, &nodes);
    assert!(msgs.len() > 1);
    let mut total = 0;
    for m in &msgs {
        assert!(m.len() <= crate::MAX_MESSAGE + 1024);
        match ClientMsg::decode(m).unwrap() {
            ClientMsg::A11yUpdate { nodes, .. } => total += nodes.len(),
            _ => panic!(),
        }
    }
    assert_eq!(total, nodes.len());
    // Obri text se zkrati.
    let big = Node::new(1, 0, Role::Label, &"é".repeat(40_000));
    let m = a11y_messages(1, &[big]);
    assert_eq!(m.len(), 1);
    assert!(m[0].len() < crate::MAX_MESSAGE);
}

#[test]
fn display_roundtrip() {
    for m in [
        Request::Info,
        Request::Attach {
            output: 0,
            w: 1280,
            h: 800,
            stride: 5120,
        },
        Request::Present {
            seq: 3,
            output: 0,
            rects: vec![Rect::new(0, 0, 10, 10)],
        },
        Request::Sync { token: u64::MAX - 7 },
        Request::OpenRender,
    ] {
        assert_eq!(Request::decode(&m.encode()), Ok(m));
    }
    for m in [
        Reply::Modes {
            caps: display::caps::VIRGL,
            outputs: vec![Output {
                w: 1280,
                h: 800,
                enabled: true,
            }],
        },
        Reply::Attached { output: 0 },
        Reply::Presented { seq: 3, time: 5 },
        Reply::Synced { token: 9 },
        Reply::Render,
        Reply::Error {
            op: display::op::ATTACH,
            code: Error::NoMemory,
        },
    ] {
        assert_eq!(Reply::decode(&m.encode()), Ok(m));
    }
}

#[test]
fn display_sync_marker() {
    assert!(Reply::Synced { token: 9 }.ends_sync(9));
    // Znacka predchozi instance (jiny token) synchronizaci nekonci.
    assert!(!Reply::Synced { token: 8 }.ends_sync(9));
    assert!(!Reply::Presented { seq: 1, time: 2 }.ends_sync(9));
    // Ovladac bez SYNC odpovi chybou na op SYNC.
    assert!(Reply::Error { op: display::op::SYNC, code: Error::NotSupported }.ends_sync(9));
    assert!(!Reply::Error { op: display::op::PRESENT, code: Error::NotSupported }.ends_sync(9));
}

#[test]
fn normalize_rects_clips_and_merges() {
    let screen = Rect::new(0, 0, 100, 100);
    let r = display::normalize_rects(&[Rect::new(-10, -10, 20, 20), Rect::new(200, 0, 5, 5)], screen);
    assert_eq!(r, vec![Rect::new(0, 0, 10, 10)]);
    let many: Vec<Rect> = (0..100).map(|i| Rect::new(i, i, 1, 1)).collect();
    let r = display::normalize_rects(&many, screen);
    assert_eq!(r, vec![Rect::new(0, 0, 100, 100)]);
}

#[test]
fn display_modes_roundtrip() {
    use crate::modes::{self, Connector, Mode, kind, mode_flags};
    let mut dp = Connector::fixed(1, kind::DISPLAYPORT, 1920, 1080);
    dp.monitor = "2243W".into();
    dp.size_mm = (477, 268);
    dp.modes.push(Mode { w: 1280, h: 720, refresh_mhz: 50_000, clock_khz: 74_250, flags: 0 });
    let off = Connector { output: 0, kind: kind::EDP, connected: false, monitor: String::new(), size_mm: (0, 0), modes: vec![] };
    for m in [Request::Connectors, Request::SetMode { output: 1, w: 1280, h: 720, refresh_mhz: 0 }] {
        assert_eq!(Request::decode(&m.encode()), Ok(m));
    }
    for m in [
        Reply::Connectors { connectors: vec![off.clone(), dp.clone()] },
        Reply::ModeSet { output: 1, w: 1280, h: 720, refresh_mhz: 50_000 },
        Reply::Hotplug { output: 1 },
        Reply::Reset,
    ] {
        assert_eq!(Reply::decode(&m.encode()), Ok(m));
    }
    assert!(Reply::Hotplug { output: 0 }.is_event());
    assert!(!Reply::Attached { output: 0 }.is_event());
    // Vyber vystupu: odpojeny panel se preskoci; vyber rezimu.
    let cs = vec![off, dp];
    let p = modes::primary(&cs).unwrap();
    assert_eq!(p.output, 1);
    assert_eq!(p.current().unwrap().w, 1920);
    assert!(p.preferred().unwrap().flags & mode_flags::PREFERRED != 0);
    assert_eq!(modes::find_mode(p, 1280, 720, 60_000).unwrap().refresh_mhz, 50_000);
    assert!(modes::find_mode(p, 800, 600, 0).is_none());
    // Pocet konektoru pres limit = chyba (nevyrabet obri alokace).
    let mut w = crate::Writer::op(display::op::CONNECTORS_REPLY);
    w.u32(1000);
    assert!(Reply::decode(&w.finish()).is_err());
}

const MANIFEST: &str = r#"
[app]
id = "org.magnumopus.demo"
name = "Ukazka"

[[action]]
id = "note.create"
title = "Nova poznamka"
needs = ["fs:write"]

[[action.param]]
name = "title"
type = "text"
required = true

[[action.param]]
name = "color"
type = "choice"
choices = ["zluta", "modra"]

[[action]]
id = "counter.add"
confirm = true

[[action.param]]
name = "by"
type = "int"
"#;

#[test]
fn manifest_actions_parse_and_check() {
    let actions = action::parse_manifest(MANIFEST).unwrap();
    assert_eq!(actions.len(), 2);
    let a = &actions[0];
    assert_eq!(a.id, "note.create");
    assert_eq!(a.needs, vec!["fs:write"]);
    assert_eq!(a.params.len(), 2);
    assert!(a.params[0].required);
    assert_eq!(a.params[1].ty, ParamType::Choice(vec!["zluta".into(), "modra".into()]));
    assert!(actions[1].confirm);
    assert_eq!(actions[1].title, "counter.add");

    // Spravne argumenty.
    assert!(a.check(&[("title".into(), Value::Text("A".into()))]).is_ok());
    assert!(a
        .check(&[("title".into(), Value::Text("A".into())), ("color".into(), Value::Choice("modra".into()))])
        .is_ok());
    // Chybi povinny, spatny typ, nedeklarovany, dvakrat, volba mimo vycet.
    assert_eq!(a.check(&[]), Err(Error::InvalidArgs));
    assert_eq!(a.check(&[("title".into(), Value::Int(1))]), Err(Error::InvalidArgs));
    assert_eq!(
        a.check(&[("title".into(), Value::Text("A".into())), ("x".into(), Value::Bool(true))]),
        Err(Error::InvalidArgs)
    );
    assert_eq!(
        a.check(&[("title".into(), Value::Text("A".into())), ("title".into(), Value::Text("B".into()))]),
        Err(Error::InvalidArgs)
    );
    assert_eq!(
        a.check(&[("title".into(), Value::Text("A".into())), ("color".into(), Value::Choice("cerna".into()))]),
        Err(Error::InvalidArgs)
    );
    // Manifest bez akci.
    assert!(action::parse_manifest("[app]\nid = \"x\"\n").unwrap().is_empty());
    // Vadne manifesty.
    assert!(action::parse_manifest("[[action]]\ntitle = \"x\"\n").is_err());
    assert!(action::parse_manifest("[[action]]\nid = \"a b\"\n").is_err());
    assert!(action::parse_manifest("[[action]]\nid = \"a\"\n[[action.param]]\nname = \"p\"\ntype = \"choice\"\n").is_err());
    assert!(action::parse_manifest("[[action]]\nid = \"a\"\n[[action]]\nid = \"a\"\n").is_err());
}

#[test]
fn connect_and_capability_protocols_roundtrip() {
    let info = ConnectInfo {
        system: false,
        id: "org.magnumopus.demo".into(),
        name: "Ukázka".into(),
        perms: vec!["síť".into()],
        actions: action::parse_manifest(MANIFEST).unwrap(),
    };
    assert_eq!(ConnectInfo::decode(&info.encode()), Ok(info.clone()));
    assert!(ConnectInfo::decode(b"connect\n").is_err());

    for m in [
        DialogRequest::Ask {
            id: 1,
            kind: DialogKind::Permission,
            app: "Ukázka".into(),
            text: "Povolit síť?".into(),
            options: vec!["Povolit".into(), "Zamítnout".into()],
        },
        DialogRequest::Cancel { id: 1 },
        DialogRequest::Phrase { text: "modrý slon".into() },
    ] {
        assert_eq!(DialogRequest::decode(&m.encode()), Ok(m));
    }
    // Dialog bez voleb je neplatny.
    let bad = DialogRequest::Ask {
        id: 1,
        kind: DialogKind::Confirm,
        app: "a".into(),
        text: "t".into(),
        options: vec![],
    };
    assert!(DialogRequest::decode(&bad.encode()).is_err());
    for m in [
        DialogReply::Answer {
            id: 1,
            choice: 0,
            text: String::new(),
        },
        DialogReply::Cancelled { id: 2 },
    ] {
        assert_eq!(DialogReply::decode(&m.encode()), Ok(m));
    }
    for m in [
        CallerMsg::List,
        CallerMsg::Call {
            call: 1,
            app: "org.magnumopus.demo".into(),
            action: "counter.add".into(),
            args: vec![("by".into(), Value::Int(2))],
            synthetic: true,
        },
    ] {
        assert_eq!(CallerMsg::decode(&m.encode()), Ok(m));
    }
    for m in [
        CallerReply::Actions {
            list: vec![("org.magnumopus.demo".into(), info.actions[0].clone())],
        },
        CallerReply::Result {
            call: 1,
            ok: false,
            value: None,
            message: "zamítnuto".into(),
        },
    ] {
        assert_eq!(CallerReply::decode(&m.encode()), Ok(m));
    }
    for m in [
        InspectRequest::Windows,
        InspectRequest::Tree { window: 1 },
        InspectRequest::Act {
            window: 1,
            node: 2,
            action: 1,
            value: String::new(),
        },
        InspectRequest::Pixel { x: 1, y: 2 },
        InspectRequest::Settle,
    ] {
        assert_eq!(InspectRequest::decode(&m.encode()), Ok(m));
    }
    for m in [
        InspectReply::Windows(vec![WindowInfo {
            id: 1,
            app: "a".into(),
            title: "t".into(),
            rect: Rect::new(1, 2, 3, 4),
            focused: true,
        }]),
        InspectReply::Tree(vec![sample_node()]),
        InspectReply::Pixel(0xFF00_FF00),
        InspectReply::Done,
        InspectReply::Error(Error::NotFound),
    ] {
        assert_eq!(InspectReply::decode(&m.encode()), Ok(m));
    }
}

#[test]
fn status_roundtrip() {
    let s = Status {
        clients: 2,
        windows: 3,
        frames: 100,
        presented: 99,
        dialogs: 1,
        secure: true,
        splash: true,
    };
    let mut t = String::new();
    s.format(&mut t).unwrap();
    assert_eq!(Status::parse(&t), Some(s));
}

#[test]
fn paths_value_roundtrip_and_check() {
    let v = Value::Paths(vec!["/a".into(), "/b".into()]);
    let mut w = wire::Writer::new();
    v.encode(&mut w);
    let e = w.finish();
    assert_eq!(Value::decode(&mut wire::Reader::new(&e)), Ok(v.clone()));
    let mut w = wire::Writer::new();
    w.u8(6).u32(u32::MAX);
    assert!(Value::decode(&mut wire::Reader::new(&w.finish())).is_err());
    let a = action::parse_manifest("[[action]]\nid = \"files.delete\"\n[[action.param]]\nname = \"files\"\ntype = \"paths\"\nrequired = true\n").unwrap();
    assert!(a[0].check(&[("files".into(), v)]).is_ok());
    assert!(a[0].check(&[("files".into(), Value::Path("/a".into()))]).is_err());
}

#[test]
fn menu_update_roundtrip_and_limits() {
    use crate::menu::{MAX_DEPTH, MenuItem, Role, flatten};
    use crate::window::ClientMsg;
    let leaf = MenuItem {
        label: "Kopírovat".into(),
        action: "edit.copy".into(),
        role: Role::Copy,
        checked: Some(true),
        disabled: "nic nevybráno".into(),
        shortcut: "Ctrl+C".into(),
        keywords: vec!["schránka".into()],
        args: vec![("n".into(), crate::action::Value::Int(1))],
        ..Default::default()
    };
    let m = ClientMsg::MenuUpdate { window: 1, items: vec![MenuItem { label: "Úpravy".into(), children: vec![leaf], ..Default::default() }] };
    let back = ClientMsg::decode(&m.encode()).unwrap();
    assert_eq!(back, m);
    let ClientMsg::MenuUpdate { items, .. } = back else { unreachable!() };
    let f = flatten(&items);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].path, "Úpravy › Kopírovat", "role ma systemovy text");
    // Prilis hluboky strom se odmitne.
    let mut deep = MenuItem { label: "x".into(), ..Default::default() };
    for _ in 0..=MAX_DEPTH {
        deep = MenuItem { label: "x".into(), children: vec![deep], ..Default::default() };
    }
    assert!(ClientMsg::decode(&ClientMsg::MenuUpdate { window: 1, items: vec![deep] }.encode()).is_err());
}

/// Kody roli: kazda znama role projde kodovanim tam a zpet, jmena se
/// neopakuji, neznamy kod zustane zachovany.
#[test]
fn a11y_roles_roundtrip() {
    let mut names: alloc::vec::Vec<&str> = alloc::vec::Vec::new();
    for v in 1..=30u16 {
        let r = Role::from_u16(v);
        assert!(!matches!(r, Role::Unknown(_)), "kod {v}");
        assert_eq!(r.to_u16(), v);
        assert!(!names.contains(&r.name()), "{}", r.name());
        names.push(r.name());
    }
    assert_eq!(Role::from_u16(31), Role::Unknown(31));
    assert_eq!(Role::Unknown(31).to_u16(), 31);
}
