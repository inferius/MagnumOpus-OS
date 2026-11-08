//! Testovaci hostitel: FS v pameti, zachyceny vystup, odpovedi na dotazy.

use crate::ShError;
use crate::eval::{Interp, Outcome};
use crate::host::{AudioReq, ExecNeeds, Fs, Host};
use crate::sig::Builtins;
use crate::value::Value;
use crate::vfs::mem::MemFs;

pub struct TestHost {
    pub fs: MemFs,
    pub out: Vec<Value>,
    pub notes: Vec<String>,
    /// Odpoved na dotazy (nahled, zamek, opravneni).
    pub answer: bool,
    pub asked: Vec<String>,
    /// Po kolika dotazech na preruseni nahlasit Ctrl-C.
    pub interrupt_after: Option<usize>,
    calls: usize,
    pub time: i64,
    /// Spustene programy (jmeno, argumenty, needs).
    pub execs: Vec<(String, Vec<String>, ExecNeeds)>,
    /// Pri dotazu zapsat do FS (jina relace meni soubor behem dotazu).
    pub write_on_confirm: Option<(String, Vec<u8>)>,
    /// Pozadavky prikazu `audio`.
    pub audio: Vec<AudioReq>,
}

impl TestHost {
    pub fn new() -> Self {
        let mut fs = MemFs::new();
        fs.put("/home/a.txt", b"alfa\n");
        fs.put("/home/b.txt", b"beta beta\n");
        fs.put("/home/data.json", br#"[{"name":"Eva","age":31},{"name":"Jan","age":12}]"#);
        fs.put("/home/docs/big.pdf", &vec![0u8; 3000]);
        fs.put("/home/docs/small.pdf", b"x");
        fs.put("/home/.hidden", b"h");
        fs.mkdir("/tmp").unwrap();
        Self {
            fs,
            out: Vec::new(),
            notes: Vec::new(),
            answer: true,
            asked: Vec::new(),
            interrupt_after: None,
            calls: 0,
            time: 1_700_000_000_000_000_000,
            execs: Vec::new(),
            write_on_confirm: None,
            audio: Vec::new(),
        }
    }

    pub fn text(&self) -> Vec<String> {
        self.out.iter().map(|v| v.to_text()).collect()
    }
}

impl Host for TestHost {
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
        self.time += 1_000_000;
        Some(self.time)
    }
    fn sleep(&mut self, ns: i64) {
        self.time += ns;
    }
    fn interrupted(&mut self) -> bool {
        self.calls += 1;
        self.interrupt_after.is_some_and(|n| self.calls > n)
    }
    fn confirm(&mut self, question: &str, _detail: &Value) -> bool {
        self.asked.push(question.into());
        if let Some((p, d)) = self.write_on_confirm.take() {
            self.fs.write(&p, &d).unwrap();
        }
        self.answer
    }
    fn exec(
        &mut self,
        name: &str,
        args: &[(Option<String>, Value)],
        _input: Value,
        needs: &ExecNeeds,
    ) -> Option<Result<Value, ShError>> {
        if name != "tool" && name != "ro-tool" {
            return None;
        }
        self.execs.push((name.into(), args.iter().map(|(_, v)| v.to_text()).collect(), needs.clone()));
        // ro-tool jen cte.
        if name == "ro-tool" {
            return Some(Ok(Value::str("jen cteni")));
        }
        // Program zapise do slozky, kam smi; v nahledu do klonu.
        let mut target = String::from("/home/tool.out");
        if let Some((ds, root)) = needs.aside.iter().find(|(ds, _)| crate::vfs::path::within(&target, ds)) {
            target = crate::vfs::path::rebase(&target, ds, root);
        }
        let _ = self.fs.write(&target, b"z programu");
        Some(Ok(Value::str("tool ok")))
    }
    fn audio(&mut self, req: AudioReq) -> Option<Result<Value, ShError>> {
        let out = match &req {
            AudioReq::Volume(None) => Value::Int(70),
            _ => Value::Nothing,
        };
        self.audio.push(req);
        Some(Ok(out))
    }
    fn home(&self) -> String {
        "/home".into()
    }
}

pub fn interp() -> Interp {
    Interp::new(Box::new(Builtins), "/home")
}

/// Spusti radky po sobe; vrati hostitele, interpret a vysledek posledniho.
pub fn run_lines(lines: &[&str]) -> (TestHost, Interp, Outcome) {
    let mut h = TestHost::new();
    let mut it = interp();
    let mut last = None;
    for l in lines {
        last = Some(it.run_line(&mut h, l));
    }
    (h, it, last.unwrap())
}

/// Jeden radek, ktery musi projit; vrati vypsane hodnoty.
pub fn ok(src: &str) -> Vec<Value> {
    let (h, _, o) = run_lines(&[src]);
    if let Err(e) = &o.result {
        panic!("{}", e.report(src));
    }
    h.out
}

/// Jeden radek, ktery musi selhat; vrati chybu.
pub fn fails(src: &str) -> ShError {
    let (_, _, o) = run_lines(&[src]);
    o.result.expect_err(src)
}
