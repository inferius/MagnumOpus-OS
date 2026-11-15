//! Prikazy `theme` a `permissions` (vlastni hostitel se zaznamem
//! pozadavku): anglicka slova prikazu se prevedou do ulozeneho tvaru.

use crate::eval::Interp;
use crate::host::{Fs, Host};
use crate::sig::Builtins;
use crate::value::Value;
use crate::vfs::mem::MemFs;

struct LookHost {
    fs: MemFs,
    calls: Vec<(bool, String)>,
    perms: Vec<(String, String)>,
    emitted: Vec<Value>,
}

impl Host for LookHost {
    fn fs(&mut self) -> &mut dyn Fs {
        &mut self.fs
    }
    fn emit(&mut self, value: Value) {
        self.emitted.push(value);
    }
    fn note(&mut self, _text: &str) {}
    fn now(&mut self) -> Option<i64> {
        Some(0)
    }
    fn sleep(&mut self, _ns: i64) {}
    fn interrupted(&mut self) -> bool {
        false
    }
    fn confirm(&mut self, _question: &str, _detail: &Value) -> bool {
        false
    }
    fn appearance(&mut self, system: bool, change: &str) -> Result<String, abi::Error> {
        self.calls.push((system, change.into()));
        match change {
            "schema tmavy" => Ok("tmavy ne ne".into()),
            "kontrast ano" => Ok("auto ano ne".into()),
            _ => Ok("auto ne ne".into()),
        }
    }
    fn permissions(&mut self, app: &str, perm: &str) -> Result<String, abi::Error> {
        self.perms.push((app.into(), perm.into()));
        if app.is_empty() { Ok("org.example.rec\tmicrophone\tallow\norg.example.b\tmicrophone\tdeny\n".into()) } else { Ok("odvoláno".into()) }
    }
}

fn run(line: &str) -> (LookHost, Result<(), String>) {
    let mut h = LookHost { fs: MemFs::new(), calls: Vec::new(), perms: Vec::new(), emitted: Vec::new() };
    h.fs.mkdir("/home").unwrap();
    let mut it = Interp::new(Box::new(Builtins), "/home");
    let r = it.run_line(&mut h, line).result.map(|_| ()).map_err(|e| e.code.clone());
    (h, r)
}

#[test]
fn theme_prepne_a_vypise_stav() {
    let (h, r) = run("theme dark");
    assert_eq!(r, Ok(()));
    assert_eq!(h.calls, vec![(false, String::from("schema tmavy"))]);
    let v = h.emitted.last().expect("vypis stavu");
    assert_eq!(v.field("scheme"), Some(Value::str("dark")));
    assert_eq!(v.field("contrast"), Some(Value::Bool(false)));
    let (h, _) = run("theme");
    assert_eq!(h.calls, vec![(false, String::new())]);
    let (h, _) = run("theme contrast on --system");
    assert_eq!(h.calls, vec![(true, String::from("kontrast ano"))]);
    assert_eq!(h.emitted.last().and_then(|v| v.field("contrast")), Some(Value::Bool(true)));
    let (h, _) = run("theme reduce-motion off");
    assert_eq!(h.calls, vec![(false, String::from("pohyb ne"))]);
    let (h, _) = run("theme light");
    assert_eq!(h.calls, vec![(false, String::from("schema svetly"))]);
}

#[test]
fn theme_spatne_argumenty() {
    // Ceska slova uz nejsou (bez aliasu).
    for line in ["theme blue", "theme contrast", "theme dark on", "theme tmavy", "theme contrast ano", "theme contrast maybe"] {
        let (h, r) = run(line);
        assert_eq!(r.err().as_deref(), Some("SH_ARGS"), "{line}");
        assert!(h.calls.is_empty(), "{line}");
    }
    let (_, r) = run("vzhled tmavy");
    assert_eq!(r.err().as_deref(), Some("SH_UNKNOWN"));
}

#[test]
fn permissions_vypis_je_tabulka_a_odvolani() {
    let (h, r) = run("permissions");
    assert_eq!(r, Ok(()));
    assert_eq!(h.perms, vec![(String::new(), String::new())]);
    let Some(Value::List(rows)) = h.emitted.last() else { panic!("tabulka: {:?}", h.emitted) };
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].field("app"), Some(Value::str("org.example.rec")));
    assert_eq!(rows[0].field("permission"), Some(Value::str("microphone")));
    assert_eq!(rows[0].field("allowed"), Some(Value::Bool(true)));
    assert_eq!(rows[1].field("allowed"), Some(Value::Bool(false)));
    let (h, r) = run("permissions revoke org.example.rec");
    assert_eq!(r, Ok(()));
    assert_eq!(h.perms, vec![(String::from("org.example.rec"), String::from("microphone"))]);
    for line in ["permissions revoke", "permissions odvolat x", "permissions revoke x mikrofon"] {
        let (h, r) = run(line);
        assert_eq!(r.err().as_deref(), Some("SH_ARGS"), "{line}");
        assert!(h.perms.is_empty(), "{line}");
    }
    let (_, r) = run("povoleni");
    assert_eq!(r.err().as_deref(), Some("SH_UNKNOWN"));
}
