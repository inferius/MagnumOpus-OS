//! Sada "ucty": spousteni programu ze shellu relace uzivatele (sluzba exec
//! initu, `user/init/src/exec.rs`, program `user/std-exec`).
//!
//! Shell sam sit nema a initu posila jen jmeno, argumenty a cesty: sit
//! programu muze prijit jen od initu. Overuje se, ze ji dostane program
//! s `[network]` v manifestu, ne program bez manifestu (`std-exec-bez`,
//! stejny obraz) ani program ze skriptu s `needs { net: none }`.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use accountproto::client::SelfService;
use libos::Startup;
use shlang::value::Value;
use term::proto::{ClientMsg, Item, ServerMsg};

use super::accounts::{alice_uid, sign_in};
use super::shell::{Reg, Window, start_termd_fs};
use super::{TestResult, fail};
use crate::check;

pub fn programs(s: &Startup) -> TestResult {
    let uid = alice_uid()?;
    let session = sign_in(uid)?;
    let r = run_in(s, session);
    crate::exec::close_session(session);
    let out = SelfService(crate::session::take_self(session).ok_or_else(|| String::from("kanal self"))?).logout();
    r?;
    out.map_err(|e| fail("odhlaseni", e))
}

/// Texty vystupu bloku (hodnoty) a poznamky (stderr).
fn texts(items: &[Item]) -> (Vec<String>, Vec<String>) {
    let mut v = Vec::new();
    let mut n = Vec::new();
    for i in items {
        match i {
            Item::Value(Value::Str(s)) => v.push(s.clone()),
            Item::Note(s) => n.push(s.clone()),
            _ => {}
        }
    }
    (v, n)
}

fn error_code(items: &[Item]) -> Option<String> {
    items.iter().find_map(|i| match i {
        Item::Error { code, .. } => Some(code.clone()),
        _ => None,
    })
}

fn run_in(s: &Startup, session: u64) -> TestResult {
    let fs = crate::session::connect(session).map_err(|e| fail("relace FS", e))?;
    let exec = crate::exec::endpoint(session).map_err(|e| fail("kanal exec", e))?;
    let mut reg = Reg { store: Default::default(), chan: None };
    reg.store.home = String::from("/home/alice");
    let (proc, w) = start_termd_fs(s, &mut reg, fs, None, Some(exec))?;
    let r = (|| -> TestResult {
        w.send(ClientMsg::Attach { session: 0 })?;
        let id = match w.recv()? {
            ServerMsg::Attached { session, .. } => session,
            other => return Err(format!("cekal jsem Attached: {other:?}")),
        };
        let out = |line: &str| -> Result<Vec<String>, String> { Ok(texts(&w.ok(id, line)?).0) };
        // Sit podle manifestu: s [network] ano, bez manifestu ne.
        let v = out("std-exec net")?;
        check!(v == ["sit: ano"], format!("program s manifestem: {v:?}"));
        let v = out("std-exec-bez net")?;
        check!(v == ["sit: ne"], format!("program bez manifestu: {v:?}"));
        // Pracovni slozka a identita uzivatele (zapis do domova ano, do
        // /system ne).
        let v = out("std-exec cwd")?;
        check!(v == ["/home/alice"], format!("pracovni slozka: {v:?}"));
        let v = out("std-exec write ~/exec.txt")?;
        check!(v == ["zapsano"], format!("zapis do domova: {v:?}"));
        let v = w.value(id, "open ~/exec.txt | from text")?;
        check!(matches!(&v, Value::Str(t) if t == "std-exec"), format!("obsah: {v:?}"));
        let (items, ok) = w.run(id, "std-exec write /system/exec.txt")?;
        check!(!ok && error_code(&items).as_deref() == Some("SH_EXIT"), format!("zapis do /system: {items:?}"));
        // Navratovy kod a stderr.
        let (items, ok) = w.run(id, "std-exec exit 3")?;
        let (_, notes) = texts(&items);
        check!(!ok && error_code(&items).as_deref() == Some("SH_EXIT"), format!("kod 3: {items:?}"));
        check!(notes.iter().any(|n| n == "konec 3"), format!("stderr: {notes:?}"));
        // Vstup z roury.
        let v = out("\"a\\nb\\n\" | std-exec echo")?;
        check!(v == ["a", "b"], format!("stdin z roury: {v:?}"));
        // Skript bez site: program sit nedostane ani s manifestem.
        // Skript zapsat primo (v retezci shellu je `{` vlozeny vyraz).
        let user = crate::session::connect(session).map_err(|e| fail("relace FS", e))?;
        let script = b"lang 1\nneeds {\n    read: [\"~\"]\n    net: none\n}\nstd-exec net\n";
        let _ = user.create("/home/alice/bez-site.mos", fsproto::kind::FILE);
        user.write("/home/alice/bez-site.mos", 0, script).map_err(|e| fail("skript", e))?;
        let v = texts(&confirmed(&w, id, "~/bez-site.mos")?).0;
        check!(v == ["sit: ne"], format!("skript s net: none: {v:?}"));
        // Neznamy program = neznamy prikaz shellu.
        let (items, ok) = w.run(id, "std-exec-neni")?;
        check!(!ok && error_code(&items).as_deref() == Some("SH_UNKNOWN"), format!("neznamy program: {items:?}"));
        interactive(&w, id)?;
        w.ok(id, "rm ~/exec.txt ~/bez-site.mos")?;
        Ok(())
    })();
    let _ = proc.kill();
    r
}

/// Spusti radek a potvrdi vyzvy (povoleni `needs` skriptu); polozky bloku.
fn confirmed(w: &Window, id: u64, line: &str) -> Result<Vec<Item>, String> {
    w.send(ClientMsg::Run { session: id, line: line.into() })?;
    let mut ours = None;
    let mut items = Vec::new();
    loop {
        match w.recv()? {
            ServerMsg::BlockStart { block, .. } if ours.is_none() => ours = Some(block),
            ServerMsg::Prompt { prompt, .. } => w.send(ClientMsg::Answer { prompt, yes: true })?,
            ServerMsg::Item { item, block, .. } if Some(block) == ours => items.push(item),
            ServerMsg::BlockEnd { ok, block, .. } if Some(block) == ours => {
                check!(ok, format!("'{line}' selhal: {items:?}"));
                return Ok(items);
            }
            _ => {}
        }
    }
}

/// Interaktivni stdin: radek z vyzvy jde programu, Ctrl+C ho ukonci.
fn interactive(w: &Window, id: u64) -> TestResult {
    w.send(ClientMsg::Run { session: id, line: "std-exec echo".into() })?;
    let mut ours = None;
    let mut answered = false;
    let mut echoed = false;
    loop {
        match w.recv()? {
            ServerMsg::BlockStart { block, .. } if ours.is_none() => ours = Some(block),
            ServerMsg::Ask { prompt, .. } if !answered => {
                answered = true;
                w.send(ClientMsg::Text { prompt, text: "ahoj-stdin".into() })?;
            }
            ServerMsg::Item { item: Item::Value(Value::Str(s)), block, .. } if Some(block) == ours && s == "ahoj-stdin" => {
                echoed = true;
                w.send(ClientMsg::Interrupt { session: id })?;
            }
            ServerMsg::BlockEnd { ok, block, .. } if Some(block) == ours => {
                check!(echoed, "radek z vyzvy se nevratil z programu");
                check!(!ok, "Ctrl+C: blok ma skoncit chybou");
                return Ok(());
            }
            _ => {}
        }
    }
}
