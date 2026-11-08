use alloc::string::String;
use alloc::vec::Vec;

use crate::json::parse;
use crate::policy::{Decision, Mode, Policy, ReadScope, glob, normalize, under};
use crate::tool::{Effect, Schema, ToolSpec};

fn tool(name: &str, effect: Effect) -> ToolSpec {
    ToolSpec::new(name, "", effect, Schema::object().build())
}

fn policy(mode: Mode, ws: Option<&str>) -> Policy {
    Policy { mode, workspace: ws.map(String::from), read_scope: ReadScope::User, never_leave: Vec::new(), can_confirm: true }
}

#[test]
fn normalizace_cest() {
    assert_eq!(normalize("/a//b/").unwrap(), "/a/b");
    assert_eq!(normalize("/").unwrap(), "/");
    for bad in ["a/b", "/a/../b", "/a/./b", "", "/a\0"] {
        assert!(normalize(bad).is_err(), "{bad:?}");
    }
    assert!(under("/home/a/proj/x", "/home/a/proj"));
    assert!(under("/home/a/proj", "/home/a/proj"));
    assert!(!under("/home/a/projekt", "/home/a/proj"));
    assert!(under("/x", "/"));
}

#[test]
fn vzory_cest() {
    assert!(glob("**/.ssh/**", "/home/a/.ssh/id_ed25519"));
    assert!(glob("**/*.key", "/home/a/anthropic.key"));
    assert!(glob("**/*.key", "/x.key"));
    assert!(!glob("**/*.key", "/home/a/key.txt"));
    assert!(glob("/system/agent/**", "/system/agent/keys/1.claude"));
    assert!(glob("/system/agent/**", "/system/agent"));
    assert!(glob("/a/*/c", "/a/bb/c"));
    assert!(!glob("/a/*/c", "/a/b/b/c"));
    assert!(glob("**/x*y*z", "/q/xAyBz"));
}

#[test]
fn cteni_volne_zapis_podle_prostoru() {
    let p = policy(Mode::Chat, Some("/home/a/proj"));
    let read = tool("fs.read", Effect::Read);
    let write = tool("fs.write", Effect::Write);
    assert_eq!(p.decide(&read, &parse(r#"{"path":"/etc/x"}"#).unwrap()), Decision::Allow);
    assert_eq!(p.decide(&write, &parse(r#"{"path":"/home/a/proj/a.txt"}"#).unwrap()), Decision::Workspace);
    assert_eq!(p.decide(&write, &parse(r#"{"path":"/home/a/jinde.txt"}"#).unwrap()), Decision::Confirm);
    // Prejmenovani: obe cesty musi byt v prostoru.
    let ren = tool("fs.rename", Effect::Write);
    assert_eq!(p.decide(&ren, &parse(r#"{"from":"/home/a/proj/a","to":"/home/a/b"}"#).unwrap()), Decision::Confirm);
    // Neplatna cesta.
    assert!(matches!(p.decide(&write, &parse(r#"{"path":"/home/a/proj/../x"}"#).unwrap()), Decision::Deny(_)));
    // Bez UI potvrzeni = odmitnout.
    let mut q = p.clone();
    q.can_confirm = false;
    assert!(matches!(q.decide(&write, &parse(r#"{"path":"/home/a/x"}"#).unwrap()), Decision::Deny(_)));
    assert!(matches!(q.decide(&tool("dev.build", Effect::Exec), &parse("{}").unwrap()), Decision::Deny(_)));
}

#[test]
fn cesty_ktere_neopusti_pocitac() {
    let mut p = policy(Mode::Chat, Some("/home/a"));
    let read = tool("fs.read", Effect::Read);
    for path in ["/home/a/.ssh/id_rsa", "/system/agent/machine.key", "/home/a/x.pem"] {
        assert!(matches!(p.decide(&read, &parse(&alloc::format!(r#"{{"path":"{path}"}}"#)).unwrap()), Decision::Deny(_)), "{path}");
    }
    // I zapis do prostoru na takovou cestu je odmitnut.
    let write = tool("fs.write", Effect::Write);
    assert!(matches!(p.decide(&write, &parse(r#"{"path":"/home/a/.ssh/authorized_keys"}"#).unwrap()), Decision::Deny(_)));
    p.never_leave.push(String::from("/home/a/tajne/**"));
    assert!(matches!(p.decide(&read, &parse(r#"{"path":"/home/a/tajne/x"}"#).unwrap()), Decision::Deny(_)));
}

#[test]
fn rezimy_a_rozsah_cteni() {
    let shell = policy(Mode::Shell, None);
    assert!(shell.mode.offers(&tool("shell.propose", Effect::Propose)));
    assert!(!shell.mode.offers(&tool("fs.write", Effect::Write)));
    assert!(!Mode::Chat.offers(&tool("shell.propose", Effect::Propose)));
    assert!(!Mode::Diag.offers(&tool("fs.write", Effect::Write)));
    assert!(Mode::Diag.offers(&tool("sys.services", Effect::Read)));
    assert!(Mode::Dev.offers(&tool("dev.build", Effect::Exec)));
    assert!(!Mode::Chat.offers(&tool("dev.build", Effect::Exec)));
    assert!(matches!(shell.decide(&tool("fs.write", Effect::Write), &parse(r#"{"path":"/a"}"#).unwrap()), Decision::Deny(_)));
    let mut ws = policy(Mode::Chat, Some("/p"));
    ws.read_scope = ReadScope::Workspace;
    let read = tool("fs.read", Effect::Read);
    assert_eq!(ws.decide(&read, &parse(r#"{"path":"/p/a"}"#).unwrap()), Decision::Allow);
    assert!(matches!(ws.decide(&read, &parse(r#"{"path":"/q"}"#).unwrap()), Decision::Deny(_)));
}
