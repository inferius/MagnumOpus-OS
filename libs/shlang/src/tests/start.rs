//! `start` a hola cesta k souboru pres sluzbu "otevri cestu" (vlastni
//! hostitel se sluzbou).

use crate::eval::Interp;
use crate::host::{Fs, Host, Opened};
use crate::sig::Builtins;
use crate::value::Value;
use crate::vfs::mem::MemFs;

struct OpenHost {
    fs: MemFs,
    out: Vec<Value>,
    notes: Vec<String>,
    /// Zadosti (cesta, app_only).
    calls: Vec<(String, bool)>,
    reply: Result<Opened, abi::Error>,
}

impl Host for OpenHost {
    fn fs(&mut self) -> &mut dyn Fs {
        &mut self.fs
    }
    fn emit(&mut self, value: Value) {
        self.out.push(value);
    }
    fn note(&mut self, text: &str) {
        self.notes.push(text.into());
    }
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
    fn open_path(&mut self, path: &str, app_only: bool) -> Result<Opened, abi::Error> {
        self.calls.push((path.into(), app_only));
        self.reply.clone()
    }
}

fn run(line: &str, reply: Result<Opened, abi::Error>) -> (OpenHost, Result<(), crate::ShError>) {
    let mut h = OpenHost { fs: MemFs::new(), out: Vec::new(), notes: Vec::new(), calls: Vec::new(), reply };
    h.fs.put("/home/zprava.txt", b"ahoj\n");
    h.fs.put("/home/data.xyz", b"x");
    h.fs.mkdir("/home/d").unwrap();
    let mut it = Interp::new(Box::new(Builtins), "/home");
    let r = it.run_line(&mut h, line).result.map(|_| ());
    (h, r)
}

#[test]
fn start_opens_in_application() {
    let (h, r) = run("start zprava.txt", Ok(Opened::App("org.magnumopus.editor".into())));
    r.unwrap();
    assert_eq!(h.calls, vec![("/home/zprava.txt".into(), false)]);
    assert!(h.notes.iter().any(|n| n.contains("org.magnumopus.editor")), "{:?}", h.notes);
    // Slozka jde take pres sluzbu (spravce souboru).
    let (h, r) = run("start d", Ok(Opened::App("org.magnumopus.files".into())));
    r.unwrap();
    assert_eq!(h.calls, vec![("/home/d".into(), false)]);
}

#[test]
fn start_reveal_and_errors() {
    let (h, r) = run("start data.xyz", Ok(Opened::Revealed("org.magnumopus.files".into())));
    r.unwrap();
    assert!(h.notes.iter().any(|n| n.contains("Pro typ .xyz není aplikace")), "{:?}", h.notes);
    // Neexistujici cesta: chyba bez zadosti sluzbe.
    let (h, r) = run("start neni.txt", Ok(Opened::NoApp));
    assert_eq!(r.unwrap_err().code, "NOT_FOUND");
    assert!(h.calls.is_empty());
    // Chyba sluzby se jmenem chyby systemu = ten kod, jinak SH_OPEN.
    let (_, r) = run("start zprava.txt", Ok(Opened::Failed("/home/zprava.txt: ACCESS_DENIED".into())));
    assert_eq!(r.unwrap_err().code, "ACCESS_DENIED");
    let (_, r) = run("start zprava.txt", Ok(Opened::Failed("org.x neodpověděla".into())));
    let e = r.unwrap_err();
    assert_eq!(e.code, "SH_OPEN");
    assert!(e.message.contains("neodpověděla"), "{e:?}");
    // Bez sluzby (konzole): NOT_SUPPORTED s radou.
    let (_, r) = run("start zprava.txt", Err(abi::Error::NotSupported));
    assert_eq!(r.unwrap_err().code, "NOT_SUPPORTED");
}

#[test]
fn bare_file_path_opens_app_or_shows_summary() {
    let (h, r) = run("./zprava.txt", Ok(Opened::App("org.magnumopus.editor".into())));
    r.unwrap();
    assert_eq!(h.calls, vec![("/home/zprava.txt".into(), true)]);
    assert!(h.out.is_empty(), "{:?}", h.out);
    // Bez aplikace: souhrn jako open a poznamka.
    let (h, r) = run("./data.xyz", Ok(Opened::NoApp));
    r.unwrap();
    assert_eq!(h.out, vec![Value::Bytes(b"x".to_vec())]);
    assert!(h.notes.iter().any(|n| n == "Pro typ .xyz není aplikace."), "{:?}", h.notes);
    // Bez sluzby: souhrn jako driv, bez poznamky.
    let (h, r) = run("./data.xyz", Err(abi::Error::NotSupported));
    r.unwrap();
    assert_eq!(h.out, vec![Value::Bytes(b"x".to_vec())]);
    assert!(h.notes.is_empty(), "{:?}", h.notes);
    // Slozka zustava cd (sluzba se nevola).
    let (h, r) = run("./d", Ok(Opened::NoApp));
    r.unwrap();
    assert!(h.calls.is_empty());
}
