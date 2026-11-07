//! Testy interaktivniho terminalu (`term::ui`) a zivych aktualizaci.

use shlang::highlight::Class;
use shlang::value::Value;
use shlang::vfs::mem::MemFs;

use crate::proto::{ClientMsg, Item, ServerMsg};
use crate::screen::Screen;
use crate::session::Service;
use crate::theme::Theme;
use crate::ui::{App, Focus, Key};

fn app() -> App {
    let mut a = App::new(Theme::dark(), 100, 30);
    a.start(0);
    a.take_out();
    a.on_msg(ServerMsg::Welcome { version: 1 });
    a.on_msg(ServerMsg::Attached { session: 7, cwd: "/home".into(), blocks: 0 });
    a
}

fn typ(a: &mut App, s: &str) {
    for c in s.chars() {
        a.key(Key::Char(c));
    }
}

fn rec(name: &str, kind: &str, size: i64) -> Value {
    Value::record(&[
        ("name", Value::str(name)),
        ("path", Value::str(alloc::format!("/home/{name}"))),
        ("kind", Value::str(kind)),
        ("size", Value::Size(size)),
    ])
}

fn plain(a: &App) -> String {
    a.draw().iter().map(|l| l.plain()).collect::<Vec<_>>().join("\n")
}

#[test]
fn typing_highlights_now_and_semantic_later() {
    let mut a = app();
    typ(&mut a, "lss");
    let out = a.take_out();
    // Kazda klavesa: dotaz na vyznamove zvyrazneni s poradim.
    let seqs: Vec<u32> = out
        .iter()
        .filter_map(|m| match m {
            ClientMsg::Highlight { session: 7, seq, .. } => Some(*seq),
            _ => None,
        })
        .collect();
    assert_eq!(seqs.len(), 3);
    // Lexikalne hned: slovo na miste prikazu je prikaz.
    assert_eq!(a.editor.highlight().spans[0].class, Class::Command);
    // Stara odpoved (seq predchoziho textu) se zahodi.
    a.on_msg(ServerMsg::Highlighted { seq: seqs[0], spans: vec![(0, 1, Class::Unknown as u8)], error: None });
    assert_eq!(a.editor.highlight().spans[0].class, Class::Command);
    // Odpoved k aktualnimu textu se pouzije.
    a.on_msg(ServerMsg::Highlighted { seq: seqs[2], spans: vec![(0, 3, Class::Unknown as u8)], error: None });
    assert_eq!(a.editor.highlight().spans[0].class, Class::Unknown);
    assert!(plain(&a).contains("/home > lss"));
}

#[test]
fn enter_runs_history_and_ghost_suggestion() {
    let mut a = app();
    typ(&mut a, "ls -r ~/foto");
    a.key(Key::Enter);
    assert!(a.take_out().iter().any(|m| matches!(m, ClientMsg::Run { session: 7, line } if line == "ls -r ~/foto")));
    assert!(a.editor.text.is_empty());
    // Napoveda z historie: sedy zbytek, sipka doprava ho prijme.
    typ(&mut a, "ls");
    assert_eq!(a.editor.suggestion(), Some(" -r ~/foto"));
    assert!(plain(&a).contains("ls -r ~/foto"));
    a.key(Key::Right);
    assert_eq!(a.editor.text, "ls -r ~/foto");
    // Sipka nahoru: historie; dolu: zpet rozepsany text.
    a.key(Key::Ctrl('u'));
    typ(&mut a, "echo x");
    a.key(Key::Up);
    assert_eq!(a.editor.text, "ls -r ~/foto");
    a.key(Key::Down);
    assert_eq!(a.editor.text, "echo x");
}

#[test]
fn lines_typed_before_attach_wait_for_the_session() {
    let mut a = App::new(Theme::dark(), 80, 24);
    a.start(0);
    a.take_out();
    typ(&mut a, "echo ahoj");
    a.key(Key::Enter);
    assert!(!a.take_out().iter().any(|m| matches!(m, ClientMsg::Run { .. })), "relace 0 neexistuje");
    a.on_msg(ServerMsg::Attached { session: 3, cwd: "/".into(), blocks: 0 });
    assert_eq!(a.take_out(), vec![ClientMsg::Run { session: 3, line: "echo ahoj".into() }]);
}

#[test]
fn file_names_are_data_not_code() {
    use crate::ui::{Action, quote, row_action};
    // Vlozeny vyraz ve jmenu se escapuje; jmeno se po parsovani nezmeni.
    let evil = "/home/x{exec(\"zle\")}.txt";
    let q = quote(evil);
    assert_eq!(q, "\"/home/x\\{exec(\\\"zle\\\")\\}.txt\"");
    struct H(MemFs, Vec<Value>);
    impl shlang::host::Host for H {
        fn fs(&mut self) -> &mut dyn shlang::host::Fs {
            &mut self.0
        }
        fn emit(&mut self, v: Value) {
            self.1.push(v);
        }
        fn note(&mut self, _: &str) {}
        fn now(&mut self) -> Option<i64> {
            None
        }
        fn sleep(&mut self, _: i64) {}
        fn interrupted(&mut self) -> bool {
            false
        }
        fn confirm(&mut self, _: &str, _: &Value) -> bool {
            false
        }
    }
    let mut h = H(MemFs::new(), Vec::new());
    let mut it = shlang::eval::Interp::new(Box::new(shlang::sig::Builtins), "/");
    it.run_line(&mut h, &alloc::format!("echo {q}")).result.unwrap();
    assert_eq!(h.1, vec![Value::str(evil)], "jmeno je text, ne kod");
    let line = row_action(Some(&rec("x{exec(\"zle\")}.txt", "file", 1)), Action::Open).unwrap();
    assert!(line.contains("\\{"), "{line}");
}

#[test]
fn links_and_stopped_progress_render_cleanly() {
    let mut a = app();
    a.on_msg(ServerMsg::BlockStart { session: 7, block: 1, line: "help".into() });
    a.on_msg(ServerMsg::Item { session: 7, block: 1, item: Item::Link { text: "dokumentace".into(), target: "https://x/y".into() } });
    a.on_msg(ServerMsg::Item { session: 7, block: 1, item: Item::Progress { done: 3, total: 10, label: "for".into() } });
    a.on_msg(ServerMsg::BlockEnd { session: 7, block: 1, ok: true, duration: 0 });
    let mut s = Screen::new(100, 30);
    s.write(&a.frame());
    let lines: Vec<String> = (0..30).map(|y| s.line(y)).collect();
    assert!(lines.iter().any(|l| l.trim() == "dokumentace"), "odkaz bez zbytku escape: {lines:?}");
    assert!(!lines.iter().any(|l| l.contains("]8;;")));
    assert!(lines.iter().any(|l| l.contains("30 %") && l.contains("zastaveno")), "{lines:?}");
}

#[test]
fn completion_menu_from_service() {
    let mut a = app();
    typ(&mut a, "open d");
    a.take_out();
    a.key(Key::Tab);
    assert!(matches!(a.take_out().last(), Some(ClientMsg::Complete { cursor: 6, .. })));
    a.on_msg(ServerMsg::Completions {
        start: 5,
        items: vec![("data.json".into(), 0, "soubor".into()), ("docs/".into(), 1, "slozka".into())],
    });
    assert!(a.menu.is_some());
    assert!(plain(&a).contains("docs/"));
    a.key(Key::Down);
    a.key(Key::Enter);
    assert_eq!(a.editor.text, "open docs/");
    assert!(a.menu.is_none());
    // Jedina moznost se doplni rovnou.
    a.on_msg(ServerMsg::Completions { start: 5, items: vec![("docs/a.txt".into(), 0, String::new())] });
    assert_eq!(a.editor.text, "open docs/a.txt");
}

#[test]
fn interactive_table_sort_filter_and_actions() {
    let mut a = app();
    a.on_msg(ServerMsg::BlockStart { session: 7, block: 1, line: "ls".into() });
    let rows = Value::List(vec![rec("foto", "dir", 0), rec("b.jpg", "file", 5000), rec("a.txt", "file", 10)]);
    a.on_msg(ServerMsg::Item { session: 7, block: 1, item: Item::Value(rows) });
    a.on_msg(ServerMsg::BlockEnd { session: 7, block: 1, ok: true, duration: 0 });
    // PgUp: do tabulky posledniho bloku.
    a.key(Key::PageUp);
    assert_eq!(a.focus, Focus::Block(0));
    // `s` razeni podle prvniho sloupce (name), `S` obrati.
    a.key(Key::Char('s'));
    let t = a.blocks[0].table.as_ref().unwrap();
    assert_eq!(t.selected().unwrap().field("name"), Some(Value::str("a.txt")));
    a.key(Key::Char('S'));
    assert_eq!(a.blocks[0].table.as_ref().unwrap().selected().unwrap().field("name"), Some(Value::str("foto")));
    // Filtr `/jpg`.
    a.key(Key::Char('/'));
    typ(&mut a, "jpg");
    a.key(Key::Enter);
    let t = a.blocks[0].table.as_ref().unwrap();
    assert_eq!(t.view.len(), 1);
    assert!(plain(&a).contains("filtr: jpg"));
    // Enter na souboru: open; d: smazat s nahledem.
    a.key(Key::Enter);
    assert!(a.take_out().iter().any(|m| matches!(m, ClientMsg::Run { line, .. } if line == "open /home/b.jpg")));
    a.key(Key::PageUp);
    a.key(Key::Char('d'));
    assert!(a.take_out().iter().any(|m| matches!(m, ClientMsg::Run { line, .. } if line == "preview { rm /home/b.jpg }")));
    // Slozka: cd a vypis; `c` vlozi cestu do radku.
    a.key(Key::PageUp);
    a.key(Key::Char('/'));
    a.key(Key::Esc);
    a.key(Key::Home);
    a.key(Key::Enter);
    assert!(a.take_out().iter().any(|m| matches!(m, ClientMsg::Run { line, .. } if line == "cd /home/foto; ls")));
    a.key(Key::PageUp);
    a.key(Key::Char('c'));
    assert_eq!(a.editor.text, "/home/foto");
    assert_eq!(a.focus, Focus::Editor);
}

#[test]
fn progress_bar_updates_in_place_with_rate_and_eta() {
    let mut a = app();
    a.tick(Some(1_000_000_000));
    a.on_msg(ServerMsg::BlockStart { session: 7, block: 2, line: "cp -r a b".into() });
    a.on_msg(ServerMsg::Item { session: 7, block: 2, item: Item::Progress { done: 0, total: 100, label: "soubory".into() } });
    for (k, done) in [(2, 20u64), (3, 40)] {
        a.tick(Some(k * 1_000_000_000));
        a.on_msg(ServerMsg::ItemUpdate {
            session: 7,
            block: 2,
            index: 0,
            item: Item::Progress { done, total: 100, label: "soubory".into() },
        });
    }
    assert_eq!(a.blocks[0].items.len(), 1, "jeden pruh, zmeneny na miste");
    let p = plain(&a);
    assert!(p.contains("40 %"), "{p}");
    assert!(p.contains("40 / 100 soubory"), "{p}");
    assert!(p.contains("20 pol./s"), "{p}");
    assert!(p.contains("zbývá 3 s"), "{p}");
    // Ctrl-C pri bezicim prikazu = preruseni.
    a.key(Key::Ctrl('c'));
    assert!(a.take_out().iter().any(|m| matches!(m, ClientMsg::Interrupt { session: 7 })));
}

#[test]
fn prompts_yes_no_and_choices() {
    let mut a = app();
    a.on_msg(ServerMsg::Prompt { prompt: 9, question: "Prevzit 2 zmeny?".into(), detail: Item::Note(String::new()), choices: vec![] });
    assert!(plain(&a).contains("Ano (a)"));
    a.key(Key::Char('a'));
    assert_eq!(a.take_out(), vec![ClientMsg::Answer { prompt: 9, yes: true }]);
    a.on_msg(ServerMsg::Prompt {
        prompt: 10,
        question: "Kam?".into(),
        detail: Item::Note(String::new()),
        choices: vec!["zaloha".into(), "kos".into(), "smazat".into()],
    });
    a.key(Key::Down);
    a.key(Key::Enter);
    assert_eq!(a.take_out(), vec![ClientMsg::Choose { prompt: 10, index: 1 }]);
    a.on_msg(ServerMsg::Prompt { prompt: 11, question: "?".into(), detail: Item::Note(String::new()), choices: vec!["x".into(), "y".into()] });
    a.key(Key::Char('2'));
    assert_eq!(a.take_out(), vec![ClientMsg::Choose { prompt: 11, index: 1 }]);
}

#[test]
fn frame_draws_on_screen_with_colors() {
    let mut a = app();
    a.on_msg(ServerMsg::BlockStart { session: 7, block: 1, line: "ls".into() });
    a.on_msg(ServerMsg::Item { session: 7, block: 1, item: Item::Value(Value::List(vec![rec("foto", "dir", 0)])) });
    a.on_msg(ServerMsg::BlockEnd { session: 7, block: 1, ok: true, duration: 0 });
    typ(&mut a, "echo ahoj");
    let mut s = Screen::new(100, 30);
    s.write(&a.frame());
    let lines: Vec<String> = (0..30).map(|y| s.line(y)).collect();
    assert!(lines[0].contains("relace 7"), "{:?}", lines[0]);
    assert!(lines.iter().any(|l| l.contains("/home > echo ahoj")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("foto")));
    // Stavovy radek ma pozadi, jmeno slozky barvu slozek.
    assert!(s.cell(0, 0).bg.is_some());
    let row = lines.iter().position(|l| l.contains("foto")).unwrap();
    let col = lines[row].find("foto").unwrap();
    assert_eq!(s.cell(col, row).fg, {
        let c = Theme::dark().file(crate::theme::FileClass::Dir).fg;
        (c.0, c.1, c.2)
    });
    // Druhy snimek prekresli (smazani obrazovky).
    a.key(Key::Ctrl('u'));
    s.write(&a.frame());
    assert!(!(0..30).any(|y| s.line(y).contains("echo ahoj")));
}

#[test]
fn service_sends_live_progress_for_long_work() {
    let mut svc = Service::single("/home", 1);
    let mut link = super::tests::TestLink::default();
    let mut f = MemFs::new();
    f.put("/home/a.txt", b"a");
    let mut env = super::tests::TestEnv::default();
    svc.handle(ClientMsg::Attach { session: 1 }, &mut link, &mut f, &mut env);
    svc.handle(ClientMsg::Run { session: 1, line: "for i in (range 0 50) { let x = i }".into() }, &mut link, &mut f, &mut env);
    let first = link.sent.iter().filter(|m| matches!(m, ServerMsg::Item { item: Item::Progress { .. }, .. })).count();
    let updates: Vec<u64> = link
        .sent
        .iter()
        .filter_map(|m| match m {
            ServerMsg::ItemUpdate { item: Item::Progress { done, .. }, .. } => Some(*done),
            _ => None,
        })
        .collect();
    assert_eq!(first, 1, "jeden pruh");
    assert!(updates.len() >= 10 && updates.len() <= 51, "aktualizace jen pri zmene procenta: {}", updates.len());
    assert_eq!(*updates.last().unwrap(), 50, "konec na 100 %");
    // sleep od 1 s: pruh casu.
    link.sent.clear();
    svc.handle(ClientMsg::Run { session: 1, line: "sleep 1s".into() }, &mut link, &mut f, &mut env);
    assert!(link.sent.iter().any(|m| matches!(m, ServerMsg::Item { item: Item::Progress { label, .. }, .. } if label == "sleep")));
}

/// Snimek konzole do souboru PPM (jen rucne: `TERM_SNAPSHOT=cesta cargo
/// test -p term snapshot -- --ignored`). Stejna cesta jako konzole:
/// `term::ui` -> `term::screen::paint`.
#[test]
#[ignore]
fn snapshot() {
    use crate::screen::{PixelFormat, Surface, grid_for, paint};
    let Ok(path) = std::env::var("TERM_SNAPSHOT") else { return };
    let (w, h) = (1280usize, 800usize);
    let (cols, rows) = grid_for(w, h);
    let mut a = App::new(Theme::dark(), cols, rows);
    a.start(0);
    a.on_msg(ServerMsg::Welcome { version: 1 });
    a.on_msg(ServerMsg::Attached { session: 1, cwd: "/home/foto".into(), blocks: 0 });
    let day = 86_400_000_000_000i64;
    let now = 20_000 * day;
    a.tick(Some(now));
    let file = |name: &str, kind: &str, ext: &str, size: i64, age: i64| {
        Value::record(&[
            ("name", Value::str(name)),
            ("path", Value::str(alloc::format!("/home/foto/{name}"))),
            ("kind", Value::str(kind)),
            ("size", Value::Size(size)),
            ("mtime", Value::Time(now - age)),
            ("ext", Value::str(ext)),
            ("access", Value::str(if kind == "dir" { "rwx" } else { "rw-" })),
        ])
    };
    a.on_msg(ServerMsg::BlockStart { session: 1, block: 1, line: "ls -r | where size > 5MB | sort mtime".into() });
    a.on_msg(ServerMsg::Item {
        session: 1,
        block: 1,
        item: Item::Value(Value::List(vec![
            file("dovolena-2024", "dir", "", 0, 2 * day),
            file("IMG_4012.jpg", "file", "jpg", 8_400_000, day / 2),
            file("IMG_4013.jpg", "file", "jpg", 12_100_000, day / 3),
            file("zaloha.tar.zst", "file", "zst", 1_200_000_000, 900 * day),
            file("oprav.sh", "file", "sh", 2_000, 10 * day),
        ])),
    });
    a.on_msg(ServerMsg::BlockEnd { session: 1, block: 1, ok: true, duration: 41_000_000 });
    a.on_msg(ServerMsg::BlockStart { session: 1, block: 2, line: "for f in (ls *.jpg) { zmensi $f --width 1600 }".into() });
    a.on_msg(ServerMsg::Item { session: 1, block: 2, item: Item::Progress { done: 0, total: 312, label: "for".into() } });
    for (k, d) in [(1, 60u64), (2, 120), (3, 187)] {
        a.tick(Some(now + k * 1_000_000_000));
        a.on_msg(ServerMsg::ItemUpdate { session: 1, block: 2, index: 0, item: Item::Progress { done: d, total: 312, label: "for".into() } });
    }
    for c in "preview { rm *.tmp }".chars() {
        a.key(Key::Char(c));
    }
    a.key(Key::PageUp);
    a.key(Key::Down);
    let mut s = Screen::new(cols, rows);
    s.write(&a.frame());
    let mut buf = vec![0u32; w * h];
    let mut surf = Surface { buf: &mut buf, width: w, height: h, stride: w, format: PixelFormat::Rgb };
    paint(&s, &mut surf);
    let mut out = alloc::format!("P6\n{w} {h}\n255\n").into_bytes();
    for p in &buf {
        out.extend_from_slice(&[(*p & 0xff) as u8, ((*p >> 8) & 0xff) as u8, ((*p >> 16) & 0xff) as u8]);
    }
    std::fs::write(path, out).unwrap();
}

#[test]
fn welcome_shows_once_for_a_new_session_and_reconnect_keeps_session() {
    let mut a = App::new(Theme::dark(), 100, 30);
    a.welcome = true;
    a.start(0);
    a.take_out();
    a.on_msg(ServerMsg::Welcome { version: 1 });
    a.on_msg(ServerMsg::Attached { session: 3, cwd: "/".into(), blocks: 0 });
    assert_eq!(a.blocks.len(), 1);
    assert!(a.frame().contains("help"), "{}", a.frame());
    // Spadlo spojeni (termd): nove pripojeni ke stejne relaci, napoveda znovu ne.
    a.disconnected();
    assert!(!a.connected);
    a.start(a.session);
    let out = a.take_out();
    assert!(out.iter().any(|m| matches!(m, ClientMsg::Attach { session: 3 })), "{out:?}");
    a.on_msg(ServerMsg::Welcome { version: 1 });
    a.on_msg(ServerMsg::Attached { session: 3, cwd: "/".into(), blocks: 0 });
    assert_eq!(a.blocks.len(), 1);
    // Relace s bloky (prehrava se): bez napovedy.
    let mut b = App::new(Theme::dark(), 100, 30);
    b.welcome = true;
    b.on_msg(ServerMsg::Attached { session: 4, cwd: "/".into(), blocks: 2 });
    assert!(b.blocks.is_empty());
    // run_line jako akce okna.
    a.run_line("ls");
    assert!(a.take_out().iter().any(|m| matches!(m, ClientMsg::Run { line, .. } if line == "ls")));
}
