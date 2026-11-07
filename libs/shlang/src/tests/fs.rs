//! Testy souborovych vrstev: liny undo, nahled, zamek, needs, programy.

use super::host::{TestHost, interp, run_lines};
use crate::error::code;
use crate::host::Fs;
use crate::value::Value;

#[test]
fn reading_commands_cost_nothing() {
    let (mut h, it, o) = run_lines(&["ls -r | count", "open a.txt"]);
    o.result.unwrap();
    assert!(it.undo.entries.is_empty(), "cteni nesmi delat snapshot");
    assert!(h.fs.snapshot_list("shell-undo-1", "/").is_err());
}

#[test]
fn lazy_undo_restores_last_command() {
    let mut h = TestHost::new();
    let mut it = interp();
    it.run_line(&mut h, "echo novy > a.txt").result.unwrap();
    it.run_line(&mut h, "rm b.txt").result.unwrap();
    it.run_line(&mut h, "mkdir /home/nova && echo x > /home/nova/f.txt").result.unwrap();
    assert_eq!(it.undo.entries.len(), 3);
    assert!(h.fs.exists("/home/nova/f.txt"));
    it.run_line(&mut h, "undo").result.unwrap();
    assert!(!h.fs.exists("/home/nova"));
    it.run_line(&mut h, "undo").result.unwrap();
    assert_eq!(h.fs.get("/home/b.txt"), Some(&b"beta beta\n"[..]));
    it.run_line(&mut h, "undo").result.unwrap();
    assert_eq!(h.fs.get("/home/a.txt"), Some(&b"alfa\n"[..]));
    assert_eq!(it.run_line(&mut h, "undo").result.unwrap_err().code, code::NO_UNDO);
}

#[test]
fn danger_lock_asks_for_bulk_changes() {
    let mut h = TestHost::new();
    let mut it = interp();
    it.lock_threshold = 3;
    h.answer = false;
    let o = it.run_line(&mut h, "for i in (range 0 5) { echo $i > f{i}.txt }");
    assert_eq!(o.result.unwrap_err().code, code::REJECTED);
    assert_eq!(h.asked.len(), 1);
    assert!(!h.fs.exists("/home/f0.txt"), "nic se nesmi zapsat");
    // v /tmp zamek neplati
    it.run_line(&mut h, "for i in (range 0 5) { echo $i > /tmp/f{i}.txt }").result.unwrap();
    assert!(h.fs.exists("/tmp/f4.txt"));
    // s potvrzenim projde
    h.answer = true;
    let o = it.run_line(&mut h, "for i in (range 0 5) { echo $i > f{i}.txt }");
    o.result.unwrap();
    assert_eq!(o.diff.len(), 5);
    assert!(h.fs.exists("/home/f4.txt"));
}

#[test]
fn preview_shows_diff_and_applies_after_confirm() {
    let mut h = TestHost::new();
    let mut it = interp();
    h.answer = false;
    let o = it.run_line(&mut h, "preview { rm *.txt\n echo n > n.txt }");
    assert_eq!(o.result.unwrap_err().code, code::REJECTED);
    assert!(h.fs.exists("/home/a.txt"));
    assert!(!h.fs.exists("/home/n.txt"));
    h.answer = true;
    let o = it.run_line(&mut h, "preview { rm *.txt\n echo n > n.txt }");
    o.result.unwrap();
    let changes = h.out.last().unwrap().field("changes").unwrap();
    let kinds: Vec<String> = match changes {
        Value::List(v) => v.iter().map(|r| r.field("change").unwrap().to_text()).collect(),
        _ => panic!(),
    };
    assert_eq!(kinds, vec!["removed", "removed", "added"]);
    assert!(!h.fs.exists("/home/a.txt"));
    assert!(h.fs.exists("/home/n.txt"));
    // nahled jde vratit jako kazdy prikaz
    it.run_line(&mut h, "undo").result.unwrap();
    assert!(h.fs.exists("/home/a.txt"));
    // --yes bez dotazu
    h.asked.clear();
    it.run_line(&mut h, "preview -y { rm a.txt }").result.unwrap();
    assert!(h.asked.is_empty());
}

#[test]
fn preview_refuses_external_programs() {
    let (h, _, o) = run_lines(&["preview { tool }"]);
    assert_eq!(o.result.unwrap_err().code, code::NEEDS);
    assert!(h.execs.is_empty());
}

#[test]
fn overlay_reads_see_own_writes() {
    let (h, _, o) = run_lines(&[
        "echo x > /tmp/n.txt\nexists /tmp/n.txt\nrm /tmp/n.txt\nexists /tmp/n.txt\nmkdir /tmp/d\nmv a.txt /tmp/d\nls /tmp/d | count",
    ]);
    o.result.unwrap();
    assert_eq!(h.out, vec![Value::Bool(true), Value::Bool(false), Value::Int(1)]);
    assert!(!h.fs.exists("/home/a.txt"));
    assert!(h.fs.exists("/tmp/d/a.txt"));
}

#[test]
fn script_needs_are_enforced() {
    let mut h = TestHost::new();
    let mut it = interp();
    h.fs.put(
        "/home/s.sh",
        b"lang 1\nneeds {\n    read: [\"~/docs\"]\n    write: [\"/tmp\"]\n    net: none\n}\nls ~/docs | count\necho ok > /tmp/out.txt\n",
    );
    it.run_line(&mut h, "run s.sh").result.unwrap();
    assert_eq!(h.out, vec![Value::Int(2)]);
    assert!(h.fs.exists("/tmp/out.txt"));
    assert_eq!(h.asked.len(), 1, "opravneni skriptu se potvrzuji");
    h.fs.put("/home/z.sh", b"lang 1\nneeds {\n    read: [\"~/docs\"]\n}\nopen ~/a.txt\n");
    let e = it.run_line(&mut h, "run z.sh").result.unwrap_err();
    assert_eq!(e.code, code::NEEDS);
    h.fs.put("/home/w.sh", b"lang 1\nneeds {\n    read: [\"~\"]\n}\necho x > ~/a.txt\n");
    let e = it.run_line(&mut h, "run w.sh").result.unwrap_err();
    assert_eq!(e.code, code::NEEDS);
    assert_eq!(h.fs.get("/home/a.txt"), Some(&b"alfa\n"[..]));
    // skript bez lang: varovani, ale bezi
    h.fs.put("/home/n.sh", b"echo bez\n");
    it.run_line(&mut h, "run n.sh").result.unwrap();
    assert!(h.notes.iter().any(|n| n.contains("lang")));
    // nepodporovana verze
    h.fs.put("/home/v.sh", b"lang 9\necho x\n");
    assert_eq!(it.run_line(&mut h, "run v.sh").result.unwrap_err().code, code::LANG);
}

#[test]
fn external_program_gets_needs_and_undo() {
    let mut h = TestHost::new();
    let mut it = interp();
    it.run_line(&mut h, "echo pred > /tmp/p.txt\ntool *.txt --mode=x").result.unwrap();
    assert_eq!(h.execs.len(), 1);
    assert_eq!(h.execs[0].1, vec!["a.txt", "b.txt", "x"]);
    assert!(h.execs[0].2.all);
    // zmeny pred programem uz jsou v FS
    assert!(h.fs.exists("/tmp/p.txt"));
    assert!(h.fs.exists("/home/tool.out"));
    it.run_line(&mut h, "undo").result.unwrap();
    assert!(!h.fs.exists("/home/tool.out"), "undo vrati i zmeny programu");
}

#[test]
fn path_rules() {
    use crate::vfs::path::*;
    assert_eq!(resolve("/home/a", "/home", "../b/./c"), "/home/b/c");
    assert_eq!(resolve("/x", "/home", "~/d"), "/home/d");
    assert_eq!(resolve("/x", "/home", "/"), "/");
    assert!(within("/Home/A/b", "/home/a"));
    assert!(!within("/home/ab", "/home/a"));
    assert!(glob_match("*.TXT", "a.txt"));
    assert!(glob_match("f[0-9].?", "f3.c"));
    assert!(!glob_match("f[!0-9]", "f3"));
    assert_eq!(ext("a.tar.gz"), "gz");
    assert_eq!(ext(".bashrc"), "");
}
