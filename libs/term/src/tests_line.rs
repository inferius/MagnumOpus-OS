//! Radky relace ve vlaknech: hlavni vlakno obsluhuje okno, Ctrl+C,
//! Ctrl+Z (na pozadi), `&`, fronta radku, `jobs`, `fg`, `kill`.

use std::time::{Duration, Instant};

use crate::proto::{ClientMsg, Item, ServerMsg};
use crate::tests_tasks::Rig;

fn rig() -> Rig {
    let mut r = Rig::new(true);
    r.svc.line_threads = true;
    r
}

/// Posle radek (vrati cislo jeho bloku, bez cekani na konec).
fn start(r: &mut Rig, line: &str) -> u64 {
    let from = r.link.sent.len();
    r.handle(ClientMsg::Run { session: r.session, line: line.into() });
    r.link.sent[from..]
        .iter()
        .find_map(|m| match m {
            ServerMsg::BlockStart { block, line: l, .. } if l == line => Some(*block),
            _ => None,
        })
        .unwrap_or_else(|| panic!("blok radku '{line}' nezacal: {:?}", &r.link.sent[from..]))
}

/// Ceka (volna chvile hlavniho vlakna) na konec bloku; vrati vysledek.
fn wait_end(r: &mut Rig, block: u64) -> bool {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        r.idle();
        if let Some(ok) = r.link.sent.iter().find_map(|m| match m {
            ServerMsg::BlockEnd { block: b, ok, .. } if *b == block => Some(*ok),
            _ => None,
        }) {
            return ok;
        }
        assert!(Instant::now() < end, "blok {block} neskoncil: {:?}", r.link.sent);
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn items(r: &Rig, block: u64) -> Vec<Item> {
    r.link
        .sent
        .iter()
        .filter_map(|m| match m {
            ServerMsg::Item { block: b, item, .. } if *b == block => Some(item.clone()),
            _ => None,
        })
        .collect()
}

fn notes(r: &Rig, block: u64) -> String {
    items(r, block)
        .iter()
        .filter_map(|i| match i {
            Item::Note(n) => Some(n.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

#[test]
fn line_runs_in_thread_and_returns_session_state() {
    let mut r = rig();
    let b = start(&mut r, "let x = 41; cd /home");
    assert!(wait_end(&mut r, b));
    let b = start(&mut r, "x + 1");
    assert!(wait_end(&mut r, b));
    assert!(items(&r, b).contains(&Item::Value(shlang::value::Value::Int(42))), "{:?}", items(&r, b));
}

#[test]
fn ctrl_c_interrupts_line_in_thread() {
    let mut r = rig();
    let b = start(&mut r, "sleep 30s");
    let t = Instant::now();
    r.handle(ClientMsg::Interrupt { session: r.session });
    assert!(!wait_end(&mut r, b));
    assert!(t.elapsed() < Duration::from_secs(5), "preruseni trvalo {:?}", t.elapsed());
    assert!(items(&r, b).iter().any(|i| matches!(i, Item::Error { code, .. } if code == "SH_INTERRUPTED")));
}

#[test]
fn lines_sent_while_busy_run_after_in_order() {
    let mut r = rig();
    let a = start(&mut r, "sleep 300ms; let v = 1");
    // Dalsi radek behem behu: do fronty, ne ztracen.
    r.handle(ClientMsg::Run { session: r.session, line: "v + 1".into() });
    assert!(wait_end(&mut r, a));
    let b = r
        .link
        .sent
        .iter()
        .find_map(|m| match m {
            ServerMsg::BlockStart { block, line, .. } if line == "v + 1" => Some(*block),
            _ => None,
        })
        .expect("radek z fronty po prvnim");
    assert!(wait_end(&mut r, b));
    assert!(items(&r, b).contains(&Item::Value(shlang::value::Value::Int(2))));
}

#[test]
fn ctrl_z_moves_line_to_background_and_session_continues() {
    let mut r = rig();
    let b = start(&mut r, "sleep 400ms; let pozde = 1; echo hotovo > /home/bg.txt");
    r.handle(ClientMsg::Background { session: r.session });
    assert!(notes(&r, b).contains("[1] na pozadí"), "{}", notes(&r, b));
    // Relace prijima dalsi radek hned.
    let p = start(&mut r, "pwd");
    assert!(wait_end(&mut r, p));
    // jobs vidi ulohu.
    let j = start(&mut r, "jobs");
    assert!(wait_end(&mut r, j));
    assert!(format!("{:?}", items(&r, j)).contains("sleep 400ms"));
    // Uloha dobehne a ohlasi se; jeji promenne se do relace nepromitnou.
    assert!(wait_end(&mut r, b));
    assert!(notes(&r, b).contains("[1] hotovo"), "{}", notes(&r, b));
    let v = start(&mut r, "pozde");
    assert!(!wait_end(&mut r, v), "promenna z ulohy na pozadi v relaci");
    let f = start(&mut r, "open /home/bg.txt | from text");
    assert!(wait_end(&mut r, f));
}

#[test]
fn ampersand_runs_in_background_and_kill_stops_it() {
    let mut r = rig();
    let b = start(&mut r, "sleep 30s &");
    assert!(notes(&r, b).contains("[1] běží na pozadí"), "{}", notes(&r, b));
    let k = start(&mut r, "kill 1");
    assert!(wait_end(&mut r, k));
    assert!(!wait_end(&mut r, b));
    assert!(notes(&r, b).contains("[1] přerušeno"), "{}", notes(&r, b));
}

#[test]
fn fg_brings_job_back_and_ctrl_c_stops_it() {
    let mut r = rig();
    let b = start(&mut r, "sleep 30s &");
    let f = start(&mut r, "fg 1");
    assert!(notes(&r, f).contains("v popředí"), "{}", notes(&r, f));
    r.handle(ClientMsg::Interrupt { session: r.session });
    assert!(!wait_end(&mut r, b));
    wait_end(&mut r, f);
    // Neexistujici uloha: chyba, ne ticho.
    let x = start(&mut r, "fg 7");
    assert!(!wait_end(&mut r, x));
}

/// Okno: Ctrl+Z pri bezicim radku posle `Background`, vyzva se vrati
/// a radek z fronty se posle.
#[test]
fn window_ctrl_z_sends_background_and_releases_queue() {
    use crate::ui::{App, Key};
    let mut a = App::new(crate::theme::Theme::dark(), 80, 24);
    a.on_msg(ServerMsg::Attached { session: 7, cwd: "/".into(), blocks: 0 });
    for c in "sleep 1h".chars() {
        a.key(Key::Char(c));
    }
    a.key(Key::Enter);
    a.on_msg(ServerMsg::BlockStart { session: 7, block: 3, line: "sleep 1h".into() });
    for c in "ls".chars() {
        a.key(Key::Char(c));
    }
    a.key(Key::Enter);
    a.take_out();
    a.key(Key::Ctrl('z'));
    let out = a.take_out();
    assert!(out.iter().any(|m| matches!(m, ClientMsg::Background { session: 7 })), "{out:?}");
    assert!(out.iter().any(|m| matches!(m, ClientMsg::Run { line, .. } if line == "ls")), "fronta po Ctrl+Z: {out:?}");
    assert!(!a.fg_busy());
}

/// Uloha `&` se zapisem si sve klony na konci zahodi (relace jeji
/// interpret nedostane, volna chvile relace by je neuklidila).
#[test]
fn background_job_discards_its_clones() {
    let mut r = rig();
    let b = start(&mut r, "echo x > /home/bgclone.txt &");
    assert!(wait_end(&mut r, b));
    let c = start(&mut r, "ls -a /tmp/.aside | count");
    assert!(wait_end(&mut r, c));
    assert!(items(&r, c).contains(&Item::Value(shlang::value::Value::Int(0))), "{:?}", items(&r, c));
}

/// Falesny installd: rozlozeni (s jednim hlasenim prubehu), disky zadne.
fn fake_installer(req: &[u8]) -> Vec<crate::session::InstallerMsg> {
    use crate::session::InstallerMsg;
    use installplan::proto::{KeyboardEntry, Reply, Request};
    let kb = |n: &str| KeyboardEntry { name: n.into(), description: n.into(), rows: vec![n.into()], keys: vec![] };
    match Request::decode(req) {
        Ok(Request::Keyboards) => vec![
            InstallerMsg::Progress(Reply::Progress { step: 0, text: "rozlozeni".into() }.encode()),
            InstallerMsg::Done(Reply::Keyboards(vec![kb("cz"), kb("us")]).encode()),
        ],
        _ => vec![InstallerMsg::Done(Reply::Failed(abi::Error::NotFound, "zadny disk".into()).encode())],
    }
}

/// Ceka na zpravu okna od indexu `from`.
fn wait_msg<T>(r: &mut Rig, from: usize, f: impl Fn(&ServerMsg) -> Option<T>) -> T {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        r.idle();
        if let Some(v) = r.link.sent[from..].iter().find_map(&f) {
            return v;
        }
        assert!(Instant::now() < end, "zprava neprisla: {:?}", &r.link.sent[from..]);
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn text_question_from_line_in_thread() {
    let mut r = rig();
    r.env.installer = Some(fake_installer);
    let b = start(&mut r, "install");
    // Hlavni rozlozeni, pak "hotovo" u dalsich (vyber), pak pasmo (text).
    let mut from = 0;
    for pick in [0u32, 1] {
        let (id, n) = wait_msg(&mut r, from, |m| match m {
            ServerMsg::Prompt { prompt, choices, .. } => Some((*prompt, choices.len() as u32)),
            _ => None,
        });
        from = r.link.sent.len();
        r.handle(ClientMsg::Choose { prompt: id, index: pick.min(n - 1) });
    }
    let (id, secret, default) = wait_msg(&mut r, from, |m| match m {
        ServerMsg::Ask { prompt, secret, default, .. } => Some((*prompt, *secret, default.clone())),
        _ => None,
    });
    assert!(!secret);
    assert_eq!(default, "Europe/Prague");
    r.handle(ClientMsg::Text { prompt: id, text: "Europe/Berlin".into() });
    // Bez disku pruvodce skonci chybou; odpoved i prubeh jsou v bloku.
    assert!(!wait_end(&mut r, b));
    let n = notes(&r, b);
    assert!(n.contains("Časové pásmo -> Europe/Berlin"), "{n}");
    assert!(n.contains("[1/8] rozlozeni"), "{n}");
}
