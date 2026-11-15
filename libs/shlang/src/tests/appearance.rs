//! Prikaz `vzhled` (vlastni hostitel se zaznamem pozadavku).

use crate::eval::Interp;
use crate::host::{Fs, Host};
use crate::sig::Builtins;
use crate::value::Value;
use crate::vfs::mem::MemFs;

struct LookHost {
    fs: MemFs,
    calls: Vec<(bool, String)>,
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
            "kontrast moc" => Err(abi::Error::InvalidArgs),
            "schema tmavy" => Ok("tmavy ne ne".into()),
            _ => Ok("auto ne ne".into()),
        }
    }
}

fn run(line: &str) -> (LookHost, Result<(), String>) {
    let mut h = LookHost { fs: MemFs::new(), calls: Vec::new(), emitted: Vec::new() };
    h.fs.mkdir("/home").unwrap();
    let mut it = Interp::new(Box::new(Builtins), "/home");
    let r = it.run_line(&mut h, line).result.map(|_| ()).map_err(|e| e.code.clone());
    (h, r)
}

#[test]
fn vzhled_prepne_a_vypise_stav() {
    let (h, r) = run("vzhled tmavy");
    assert_eq!(r, Ok(()));
    assert_eq!(h.calls, vec![(false, String::from("schema tmavy"))]);
    let v = h.emitted.last().expect("vypis stavu");
    assert_eq!(v.field("schema"), Some(Value::str("tmavy")));
    let (h, _) = run("vzhled");
    assert_eq!(h.calls, vec![(false, String::new())]);
    let (h, _) = run("vzhled kontrast ano --system");
    assert_eq!(h.calls, vec![(true, String::from("kontrast ano"))]);
    let (h, _) = run("vzhled pohyb ne");
    assert_eq!(h.calls, vec![(false, String::from("pohyb ne"))]);
}

#[test]
fn vzhled_spatne_argumenty() {
    for line in ["vzhled modry", "vzhled kontrast", "vzhled tmavy ano"] {
        let (h, r) = run(line);
        assert_eq!(r.err().as_deref(), Some("SH_ARGS"), "{line}");
        assert!(h.calls.is_empty());
    }
    let (_, r) = run("vzhled kontrast moc");
    assert_eq!(r.err().as_deref(), Some("SH_ARGS"));
}
