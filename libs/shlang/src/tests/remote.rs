//! Prikazy `sshd`, `ssh-key`, `remote-access` nad hostitelem se zaznamem
//! pozadavku (heslo jen pres dotaz, nikdy v prikazu).

use crate::eval::Interp;
use crate::host::{Fs, Host};
use crate::sig::Builtins;
use crate::value::Value;
use crate::vfs::mem::MemFs;

struct RemoteHost {
    fs: MemFs,
    calls: Vec<(String, String, String)>,
    answer: Option<String>,
    emitted: Vec<Value>,
}

impl Host for RemoteHost {
    fn fs(&mut self) -> &mut dyn Fs {
        &mut self.fs
    }
    fn emit(&mut self, v: Value) {
        self.emitted.push(v);
    }
    fn note(&mut self, _: &str) {}
    fn now(&mut self) -> Option<i64> {
        Some(0)
    }
    fn sleep(&mut self, _: i64) {}
    fn interrupted(&mut self) -> bool {
        false
    }
    fn confirm(&mut self, _: &str, _: &Value) -> bool {
        false
    }
    fn ask(&mut self, _q: &str, secret: bool, _d: &str) -> Option<String> {
        assert!(secret, "heslo se pta maskovane");
        self.answer.clone()
    }
    fn ssh(&mut self, op: &str, arg: &str, pw: &str) -> Result<String, abi::Error> {
        self.calls.push((op.into(), arg.into(), pw.into()));
        Ok(match op {
            "key-list" => String::from("1\tssh-ed25519\tSHA256:abc\talice@pc\t60\n"),
            "key-add" => String::from("2\tSHA256:def\n"),
            "access" | "access-on" => String::from("on\n"),
            "sshd-status" => String::from("enabled\ton\nrunning\tyes\nport\t22\nhost_key\tSHA256:host\nsessions\t1\nconnected\talice@10.0.2.2\n"),
            _ => String::new(),
        })
    }
}

fn run(line: &str, answer: Option<&str>) -> (RemoteHost, Result<(), String>) {
    let mut h = RemoteHost { fs: MemFs::new(), calls: Vec::new(), answer: answer.map(String::from), emitted: Vec::new() };
    h.fs.put("/home/klic.pub", b"# komentar\nssh-ed25519 AAAAC3Nz alice@pc\n");
    let mut it = Interp::new(Box::new(Builtins), "/home");
    let r = it.run_line(&mut h, line).result.map(|_| ()).map_err(|e| e.code.clone());
    (h, r)
}

#[test]
fn ssh_key_vypis_a_pridani_s_heslem() {
    let (h, r) = run("ssh-key", None);
    assert_eq!(r, Ok(()));
    let Some(Value::List(rows)) = h.emitted.last() else { panic!("{:?}", h.emitted) };
    assert_eq!(rows[0].field("fingerprint"), Some(Value::str("SHA256:abc")));
    assert_eq!(rows[0].field("added"), Some(Value::Time(60_000_000_000)));
    // Ze souboru: prvni radek, ktery neni komentar; heslo z dotazu.
    let (h, r) = run("ssh-key add klic.pub", Some("tajne"));
    assert_eq!(r, Ok(()));
    assert_eq!(h.calls, vec![(String::from("key-add"), String::from("ssh-ed25519 AAAAC3Nz alice@pc"), String::from("tajne"))]);
    assert_eq!(h.emitted.last().and_then(|v| v.field("fingerprint")), Some(Value::str("SHA256:def")));
    // Bez hesla: zruseno, nic se neposle.
    let (h, r) = run("ssh-key add klic.pub", None);
    assert_eq!(r.err().as_deref(), Some("SH_REJECTED"));
    assert!(h.calls.is_empty());
    // Soukromy klic se odmitne hned.
    let (h, r) = run("\"-----BEGIN OPENSSH PRIVATE KEY-----\" | ssh-key add -", Some("x"));
    assert_eq!(r.err().as_deref(), Some("SH_ARGS"));
    assert!(h.calls.is_empty());
    let (h, _) = run("ssh-key remove 2", None);
    assert_eq!(h.calls[0].0, "key-remove");
    assert_eq!(h.calls[0].1, "2");
}

#[test]
fn remote_access_a_sshd() {
    let (h, r) = run("remote-access on", Some("tajne"));
    assert_eq!(r, Ok(()));
    assert_eq!(h.calls[0], (String::from("access-on"), String::new(), String::from("tajne")));
    assert_eq!(h.emitted.last().and_then(|v| v.field("remote_access")), Some(Value::Bool(true)));
    let (h, _) = run("remote-access off", None);
    assert_eq!(h.calls[0].2, "", "vypnuti bez hesla");
    let (h, r) = run("sshd", None);
    assert_eq!(r, Ok(()));
    let v = h.emitted.last().unwrap();
    assert_eq!(v.field("enabled"), Some(Value::Bool(true)));
    assert_eq!(v.field("port"), Some(Value::Int(22)));
    assert_eq!(v.field("host_key"), Some(Value::str("SHA256:host")));
    let (_, r) = run("sshd zapni", None);
    assert_eq!(r.err().as_deref(), Some("SH_ARGS"));
}
