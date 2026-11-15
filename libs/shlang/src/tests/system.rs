//! Prikazy `system` (zmena v /system pres ukon spravy) a `devmode` (jen
//! vyvojove sestaveni).

use crate::eval::Interp;
use crate::host::{Fs, Host};
use crate::sig::Builtins;
use crate::value::Value;
use crate::vfs::mem::MemFs;

struct SysHost {
    fs: MemFs,
    calls: Vec<(u8, String, Vec<u8>)>,
    dev: Vec<u8>,
    emitted: Vec<Value>,
}

impl Host for SysHost {
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
    fn system_file(&mut self, op: u8, path: &str, data: &[u8]) -> Result<(), abi::Error> {
        self.calls.push((op, path.into(), data.to_vec()));
        if path.starts_with("/system/accounts") { Err(abi::Error::AccessDenied) } else { Ok(()) }
    }
    #[cfg(feature = "dev-mode")]
    fn dev_mode(&mut self, set: u8) -> Result<bool, abi::Error> {
        self.dev.push(set);
        Ok(set == 2)
    }
}

fn run(line: &str) -> (SysHost, Result<(), String>) {
    let mut h = SysHost { fs: MemFs::new(), calls: Vec::new(), dev: Vec::new(), emitted: Vec::new() };
    h.fs.mkdir("/home").unwrap();
    h.fs.write("/home/nastaveni.toml", b"pasmo = \"Europe/Prague\"\n").unwrap();
    let mut it = Interp::new(Box::new(Builtins), "/home");
    let r = it.run_line(&mut h, line).result.map(|_| ()).map_err(|e| e.code.clone());
    (h, r)
}

#[test]
fn system_zapis_smazani_slozka() {
    let (h, r) = run("system write /system/config/x.toml nastaveni.toml");
    assert_eq!(r, Ok(()));
    assert_eq!(h.calls, vec![(1, String::from("/system/config/x.toml"), b"pasmo = \"Europe/Prague\"\n".to_vec())]);
    let (h, r) = run("system rm /system/config/x.toml");
    assert_eq!(r, Ok(()));
    assert_eq!(h.calls, vec![(2, String::from("/system/config/x.toml"), Vec::new())]);
    let (h, _) = run("system mkdir /system/zkouska");
    assert_eq!(h.calls, vec![(3, String::from("/system/zkouska"), Vec::new())]);
    let (_, r) = run("system rm /system/accounts/users");
    assert_eq!(r.err().as_deref(), Some("ACCESS_DENIED"));
}

#[test]
fn system_spatne_argumenty() {
    for line in ["system", "system write /system/x", "system rm", "system smaz /system/x", "system rm /system/x navic"] {
        let (h, r) = run(line);
        assert_eq!(r.err().as_deref(), Some("SH_ARGS"), "{line}");
        assert!(h.calls.is_empty(), "{line}");
    }
}

/// Vydani: prikaz `devmode` neexistuje (ani v napovede).
#[cfg(not(feature = "dev-mode"))]
#[test]
fn devmode_ve_vydani_neni() {
    assert!(!crate::sig::BUILTINS.iter().any(|b| b.name == "devmode"));
    assert!(!crate::sig::GROUPS.iter().any(|(_, g)| g.contains(&"devmode")));
    assert!(crate::doc::get("devmode").is_none());
    let (h, r) = run("devmode on");
    assert!(r.is_err(), "neznamy prikaz");
    assert!(h.dev.is_empty());
}

#[cfg(feature = "dev-mode")]
#[test]
fn devmode_prepne_a_vypise_stav() {
    let (h, r) = run("devmode on");
    assert_eq!(r, Ok(()));
    assert_eq!(h.dev, vec![2]);
    assert_eq!(h.emitted.last().and_then(|v| v.field("devmode")), Some(Value::str("on")));
    let (h, _) = run("devmode");
    assert_eq!(h.dev, vec![0]);
    let (h, _) = run("devmode off");
    assert_eq!(h.dev, vec![1]);
    let (h, r) = run("devmode ano");
    assert_eq!(r.err().as_deref(), Some("SH_ARGS"));
    assert!(h.dev.is_empty());
}
