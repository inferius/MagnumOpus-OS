//! Testy prikazu `shutdown` a `reboot` (vlastni hostitel s napajenim).

use crate::eval::Interp;
use crate::host::{Fs, Host};
use crate::sig::Builtins;
use crate::value::Value;
use crate::vfs::mem::MemFs;

struct PowerHost {
    fs: MemFs,
    answer: bool,
    asked: Vec<String>,
    /// Pozadavky na napajeni (true = restart).
    calls: Vec<bool>,
    result: Result<(), abi::Error>,
}

impl Host for PowerHost {
    fn fs(&mut self) -> &mut dyn Fs {
        &mut self.fs
    }
    fn emit(&mut self, _value: Value) {}
    fn note(&mut self, _text: &str) {}
    fn now(&mut self) -> Option<i64> {
        Some(0)
    }
    fn sleep(&mut self, _ns: i64) {}
    fn interrupted(&mut self) -> bool {
        false
    }
    fn confirm(&mut self, question: &str, _detail: &Value) -> bool {
        self.asked.push(question.into());
        self.answer
    }
    fn power(&mut self, reboot: bool) -> Result<(), abi::Error> {
        self.calls.push(reboot);
        self.result
    }
}

fn run(line: &str, answer: bool, result: Result<(), abi::Error>) -> (PowerHost, Result<(), String>) {
    let mut h = PowerHost { fs: MemFs::new(), answer, asked: Vec::new(), calls: Vec::new(), result };
    h.fs.mkdir("/home").unwrap();
    let mut it = Interp::new(Box::new(Builtins), "/home");
    let r = it.run_line(&mut h, line).result.map(|_| ()).map_err(|e| e.code.clone());
    (h, r)
}

#[test]
fn shutdown_s_potvrzenim() {
    let (h, r) = run("shutdown", true, Ok(()));
    assert_eq!(r, Ok(()));
    assert_eq!(h.calls, vec![false]);
    assert_eq!(h.asked.len(), 1);
}

#[test]
fn reboot_bez_dotazu() {
    let (h, r) = run("reboot -y", false, Ok(()));
    assert_eq!(r, Ok(()));
    assert_eq!(h.calls, vec![true]);
    assert!(h.asked.is_empty());
}

#[test]
fn odmitnute_potvrzeni_nic_nevypne() {
    let (h, r) = run("shutdown", false, Ok(()));
    assert_eq!(r, Err(String::from(crate::error::code::REJECTED)));
    assert!(h.calls.is_empty());
}

#[test]
fn chyba_napajeni_je_chyba_prikazu() {
    let (_, r) = run("shutdown --yes", true, Err(abi::Error::NotSupported));
    assert_eq!(r, Err(String::from("NOT_SUPPORTED")));
    // Hostitel bez napajeni (vychozi Host::power).
    let (_, r) = run("reboot -y", true, Err(abi::Error::AccessDenied));
    assert_eq!(r, Err(String::from("ACCESS_DENIED")));
}
