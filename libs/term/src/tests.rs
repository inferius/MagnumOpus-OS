//! Testy terminalu: protokol, sluzba relaci (odpojeni a navrat, vyzvy,
//! Ctrl-C), adapter VT100, obrazovka a pismo, pripojene cesty.

use std::collections::VecDeque;

use shlang::host::{ExecNeeds, Fs};
use shlang::value::Value;
use shlang::vfs::mem::MemFs;

use crate::perm::{MountSpec, mounts};
use crate::proto::{ClientMsg, Item, ServerMsg};
use crate::screen::{PixelFormat, Screen, Surface, paint};
use crate::session::{Env, Link, Service};

#[derive(Default)]
pub(crate) struct TestLink {
    pub incoming: VecDeque<ClientMsg>,
    pub sent: Vec<ServerMsg>,
}

impl Link for TestLink {
    fn send(&mut self, msg: &ServerMsg) {
        // Kazda zprava musi projit kodovanim.
        let bytes = msg.encode();
        assert_eq!(&ServerMsg::decode(&bytes).unwrap(), msg);
        self.sent.push(msg.clone());
    }
    fn poll(&mut self) -> Option<ClientMsg> {
        self.incoming.pop_front()
    }
    fn wait(&mut self) -> Option<ClientMsg> {
        self.incoming.pop_front()
    }
}

#[derive(Default)]
pub(crate) struct TestEnv(i64);

impl Env for TestEnv {
    fn now(&mut self) -> Option<i64> {
        self.0 += 1000;
        Some(self.0)
    }
    fn sleep(&mut self, ns: i64) {
        self.0 += ns;
    }
}

fn fs() -> MemFs {
    let mut f = MemFs::new();
    f.put("/home/a.txt", b"alfa");
    f.put("/home/b.txt", b"beta");
    f
}

fn items(sent: &[ServerMsg]) -> Vec<Item> {
    sent.iter()
        .filter_map(|m| match m {
            ServerMsg::Item { item, .. } => Some(item.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn protocol_roundtrip() {
    let table = Value::List(vec![
        Value::record(&[("name", Value::str("a")), ("size", Value::Size(5)), ("mtime", Value::Time(7))]),
        Value::record(&[("name", Value::str("b")), ("size", Value::Size(6)), ("mtime", Value::Null)]),
    ]);
    let msgs = vec![
        ServerMsg::Welcome { version: 1 },
        ServerMsg::Item { session: 1, block: 2, item: Item::Value(table.clone()) },
        ServerMsg::Item {
            session: 1,
            block: 2,
            item: Item::Value(Value::Quantity(shlang::value::Quantity { dim: shlang::units::Dim::Frequency, value: 2.4e9 })),
        },
        ServerMsg::Item { session: 1, block: 2, item: Item::Value(Value::Missing(shlang::value::Why::Hidden)) },
        ServerMsg::Item { session: 1, block: 2, item: Item::Error { code: "X".into(), message: "m".into(), report: "r".into() } },
        ServerMsg::Item { session: 1, block: 2, item: Item::Progress { done: 1, total: 3, label: "kopiruji".into() } },
        ServerMsg::Item { session: 1, block: 2, item: Item::Link { text: "napoveda".into(), target: "help:ls".into() } },
        ServerMsg::Prompt { prompt: 3, question: "Prevzit?".into(), detail: Item::Diff(Value::List(vec![])), choices: vec!["a".into()] },
        ServerMsg::ItemUpdate { session: 1, block: 2, index: 0, item: Item::Progress { done: 1, total: 4, label: "x".into() } },
        ServerMsg::Completions { seq: 4, start: 3, items: vec![("ls".into(), 0, "vypise".into())] },
        ServerMsg::SessionList { sessions: vec![(1, 2, "/home".into())] },
        ServerMsg::BlockEnd { session: 1, block: 2, ok: false, duration: -1 },
    ];
    for m in msgs {
        assert_eq!(ServerMsg::decode(&m.encode()).unwrap(), m);
    }
    let cm = vec![
        ClientMsg::Hello { version: 1, cols: 80, rows: 25 },
        ClientMsg::Run { session: 1, line: "ls | where size > 1KB".into() },
        ClientMsg::Complete { session: 1, line: "l".into(), cursor: 1, seq: 2 },
        ClientMsg::Answer { prompt: 1, yes: true },
        ClientMsg::Choose { prompt: 1, index: 2 },
        ClientMsg::Format { session: 1, line: "ls".into(), style: 1, seq: 3 },
        ClientMsg::Sessions,
        ClientMsg::AttachRecent { session: 4 },
    ];
    for m in cm {
        assert_eq!(ClientMsg::decode(&m.encode()).unwrap(), m);
    }
    // tabulka se posila se schematem: jmena sloupcu jen jednou
    let enc = ServerMsg::Item { session: 1, block: 1, item: Item::Value(table) }.encode();
    assert_eq!(enc.windows(4).filter(|w| w == b"name").count(), 1);
    // tabulka bez sloupcu a "bomba" s dlouhym jmenem sloupce se odmitnou
    let mut zero = vec![4u8];
    zero.extend_from_slice(&1u64.to_le_bytes());
    zero.extend_from_slice(&1u64.to_le_bytes());
    zero.extend_from_slice(&[0, 13, 0, 0, 0, 0, 0, 0, 16, 0]);
    assert!(ServerMsg::decode(&zero).is_err());
    let mut bomb = vec![4u8];
    bomb.extend_from_slice(&1u64.to_le_bytes());
    bomb.extend_from_slice(&1u64.to_le_bytes());
    bomb.extend_from_slice(&[0, 13, 1, 0, 0, 0]);
    bomb.extend_from_slice(&(1u32 << 20).to_le_bytes());
    bomb.extend(std::iter::repeat_n(b'x', 1 << 20));
    bomb.extend_from_slice(&(1u32 << 20).to_le_bytes());
    bomb.extend(std::iter::repeat_n(0u8, 1 << 20));
    assert!(ServerMsg::decode(&bomb).is_err());
    // poskozena data se odmitnou
    assert!(ServerMsg::decode(&[4, 1, 2]).is_err());
    assert!(ClientMsg::decode(&[4, 0, 0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 255]).is_err());
}

#[test]
fn session_survives_detach_and_replays() {
    let mut svc = Service::new("/home");
    let mut link = TestLink::default();
    let mut f = fs();
    let mut env = TestEnv(0);
    svc.handle(ClientMsg::Attach { session: 0 }, &mut link, &mut f, &mut env);
    let id = match link.sent.last().unwrap() {
        ServerMsg::Attached { session, .. } => *session,
        other => panic!("{other:?}"),
    };
    svc.handle(ClientMsg::Run { session: id, line: "ls | count".into() }, &mut link, &mut f, &mut env);
    assert_eq!(items(&link.sent), vec![Item::Value(Value::Int(2))]);
    // okno se zavre; prikaz pobezi bez nej
    svc.handle(ClientMsg::Detach { session: id }, &mut link, &mut f, &mut env);
    link.sent.clear();
    svc.handle(ClientMsg::Run { session: id, line: "echo bez okna".into() }, &mut link, &mut f, &mut env);
    assert!(link.sent.is_empty(), "odpojenemu oknu se nic neposila");
    // nove okno: prehraji se oba bloky
    let mut link2 = TestLink::default();
    svc.handle(ClientMsg::Attach { session: id }, &mut link2, &mut f, &mut env);
    let starts: Vec<&ServerMsg> = link2.sent.iter().filter(|m| matches!(m, ServerMsg::BlockStart { .. })).collect();
    assert_eq!(starts.len(), 2);
    assert_eq!(items(&link2.sent), vec![Item::Value(Value::Int(2)), Item::Value(Value::str("bez okna"))]);
    assert!(matches!(link2.sent.last(), Some(ServerMsg::Attached { blocks: 2, .. })));
}

#[test]
fn prompts_go_to_window_and_refuse_without_it() {
    let mut svc = Service::new("/home");
    let mut link = TestLink::default();
    let mut f = fs();
    let mut env = TestEnv(0);
    svc.handle(ClientMsg::Attach { session: 0 }, &mut link, &mut f, &mut env);
    link.incoming.push_back(ClientMsg::Answer { prompt: 1, yes: true });
    svc.handle(ClientMsg::Run { session: 1, line: "preview { rm a.txt }".into() }, &mut link, &mut f, &mut env);
    assert!(link.sent.iter().any(|m| matches!(m, ServerMsg::Prompt { prompt: 1, .. })));
    assert!(!f.exists("/home/a.txt"));
    assert!(items(&link.sent).iter().any(|i| matches!(i, Item::Diff(_))));
    // bez okna se dotaz odmitne a nic se nezmeni
    svc.handle(ClientMsg::Detach { session: 1 }, &mut link, &mut f, &mut env);
    svc.handle(ClientMsg::Run { session: 1, line: "preview { rm b.txt }".into() }, &mut link, &mut f, &mut env);
    assert!(f.exists("/home/b.txt"));
    let last = svc.sessions[0].blocks.last().unwrap();
    assert_eq!(last.ok, Some(false));
    assert!(last.items.iter().any(|i| matches!(i, Item::Error { code, .. } if code == "SH_REJECTED")));
}

#[test]
fn ctrl_c_from_window_stops_command() {
    let mut svc = Service::new("/home");
    let mut link = TestLink::default();
    let mut f = fs();
    let mut env = TestEnv(0);
    svc.handle(ClientMsg::Attach { session: 0 }, &mut link, &mut f, &mut env);
    link.incoming.push_back(ClientMsg::Interrupt { session: 1 });
    svc.handle(ClientMsg::Run { session: 1, line: "while true { let x = 1 }".into() }, &mut link, &mut f, &mut env);
    assert!(items(&link.sent).iter().any(|i| matches!(i, Item::Error { code, .. } if code == "SH_INTERRUPTED")));
    // sleep jde prerusit take
    link.incoming.push_back(ClientMsg::Interrupt { session: 1 });
    svc.handle(ClientMsg::Run { session: 1, line: "sleep 1h".into() }, &mut link, &mut f, &mut env);
    assert!(env.0 < 3_600_000_000_000);
}

#[test]
fn completion_format_and_jobs() {
    let mut svc = Service::new("/home");
    let mut link = TestLink::default();
    let mut f = fs();
    let mut env = TestEnv(0);
    svc.handle(ClientMsg::Attach { session: 0 }, &mut link, &mut f, &mut env);
    // Neznama relace: odpoved Fail, ne ticho (okno by cekalo navzdy).
    svc.handle(ClientMsg::Complete { session: 99, line: "l".into(), cursor: 1, seq: 1 }, &mut link, &mut f, &mut env);
    assert!(matches!(link.sent.last(), Some(ServerMsg::Fail { .. })));
    svc.handle(ClientMsg::Complete { session: 1, line: "open a".into(), cursor: 6, seq: 5 }, &mut link, &mut f, &mut env);
    assert!(matches!(link.sent.last(), Some(ServerMsg::Completions { seq: 5, start: 5, items }) if items[0].0 == "a.txt"));
    svc.handle(ClientMsg::Format { session: 1, line: "ls ~ -r | where size > 1KB".into(), style: 1, seq: 6 }, &mut link, &mut f, &mut env);
    assert_eq!(
        link.sent.last(),
        Some(&ServerMsg::Formatted { seq: 6, text: "ls(\"~\", recursive: true).where(it => it.size > 1KB)".into() })
    );
    svc.handle(ClientMsg::Run { session: 1, line: "echo hotovo &".into() }, &mut link, &mut f, &mut env);
    link.sent.clear();
    svc.idle(&mut link, &mut f, &mut env);
    assert_eq!(items(&link.sent), vec![Item::Value(Value::str("hotovo"))]);
}

#[test]
fn vt_adapter() {
    let t = crate::vt::message(&ServerMsg::Item { session: 1, block: 1, item: Item::Note("pozn".into()) }, Some(&crate::theme::Theme::dark()));
    assert_eq!(t, "\x1b[0;38;2;154;161;176mpozn\x1b[0m\n", "poznamka barvou muted motivu");
    assert_eq!(crate::vt::strip(&t), "pozn\n");
    let l = crate::vt::item(&Item::Link { text: "a".into(), target: "b".into() }, Some(&crate::theme::Theme::dark()));
    assert_eq!(crate::vt::strip(&l), "a\n");
    let p = crate::vt::item(&Item::Progress { done: 1, total: 2, label: "x".into() }, None);
    assert_eq!(p, "[##########----------] 50% x\r");
}

#[test]
fn screen_and_font() {
    let mut s = Screen::new(10, 3);
    s.write("ahoj\n\x1b[31mčervená\x1b[0m\nřádek3\nřádek4");
    assert_eq!(s.line(0), "červená");
    assert_eq!(s.cell(0, 0).fg, crate::screen::PALETTE[3]);
    assert_eq!(s.line(2), "řádek4");
    let (w, h) = (200, 80);
    let mut buf = vec![0u32; w * h];
    let mut surf = Surface { buf: &mut buf, width: w, height: h, stride: w, format: PixelFormat::Bgr };
    paint(&s, &mut surf);
    let bg_bgr = 0x20 | (0x18 << 8) | (0x10 << 16);
    assert!(buf.iter().any(|&p| p != bg_bgr), "text se musi vykreslit");
    assert_eq!(buf[0], bg_bgr, "okraj je pozadi");
    let (cw, ch) = crate::screen::cell_size(crate::screen::font_for(768));
    assert_eq!(crate::screen::grid_for(1024, 768), ((1024 - 16) / cw, (768 - 16) / ch));
}

#[test]
fn program_mounts_from_needs() {
    let n = ExecNeeds {
        read: vec!["/home/foto".into(), "/home/foto/2024".into(), "/data".into()],
        write: vec!["/home/foto/out".into(), "/data".into()],
        net: false,
        all: false,
        aside: Vec::new(),
        cwd: String::new(),
    };
    let m = mounts(&n);
    let spec = |p: &str, ro| MountSpec { name: p.into(), target: p.into(), read_only: ro };
    assert_eq!(m, vec![spec("/data", false), spec("/home/foto", true), spec("/home/foto/out", false)]);
    assert_eq!(mounts(&ExecNeeds { all: true, ..Default::default() }), vec![spec("/", false)]);
    let _ = MemFs::new().stat("/");
}

/// Uloha `bg` se zapisem: jeji klon se zahodi ve stejne volne chvili, ve
/// ktere dobehla (ne az v dalsi - ta po prevzeti relace novym termd
/// nemusi prijit a klon by zustal v /tmp/.aside).
#[test]
fn bg_job_clone_is_discarded_in_same_idle() {
    let mut svc = Service::new("/home");
    let mut link = TestLink::default();
    let mut f = fs();
    let mut env = TestEnv(0);
    svc.handle(ClientMsg::Attach { session: 0 }, &mut link, &mut f, &mut env);
    svc.handle(ClientMsg::Run { session: 1, line: "bg { echo prezil > /home/bg.txt }".into() }, &mut link, &mut f, &mut env);
    svc.idle(&mut link, &mut f, &mut env);
    assert!(!svc.sessions.iter().any(|s| s.interp.has_flush()), "klon ulohy ceka na dalsi volnou chvili");
    let tf: &mut dyn shlang::host::Fs = &mut f;
    assert_eq!(tf.read("/home/bg.txt").unwrap(), b"prezil".to_vec());
}
