//! Testy uloh `async` ve vlaknech sluzby relaci (opravdova vlakna std,
//! sdileny FS v pameti, probouzeni podminkou).

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use shlang::error::code;
use shlang::host::{Applied, Aside, DirEntry, Fs, FsChange, FsResult, Meta, ViewRef};
use shlang::value::Value;
use shlang::vfs::mem::MemFs;

use crate::proto::{ClientMsg, Item, ServerMsg};
use crate::session::{Env, Service};
use crate::tasks::{Clock, Joinable, Notify};
use crate::tests::TestLink;

/// FS sdileny hlavnim vlaknem a vlakny uloh (jako fsd s vice relacemi).
#[derive(Clone)]
struct SharedFs(Arc<Mutex<MemFs>>);

impl Fs for SharedFs {
    fn stat(&mut self, p: &str) -> FsResult<Meta> {
        self.0.lock().unwrap().stat(p)
    }
    fn list(&mut self, p: &str) -> FsResult<Vec<DirEntry>> {
        self.0.lock().unwrap().list(p)
    }
    fn read(&mut self, p: &str) -> FsResult<Vec<u8>> {
        self.0.lock().unwrap().read(p)
    }
    fn write(&mut self, p: &str, d: &[u8]) -> FsResult<()> {
        self.0.lock().unwrap().write(p, d)
    }
    fn mkdir(&mut self, p: &str) -> FsResult<()> {
        self.0.lock().unwrap().mkdir(p)
    }
    fn remove(&mut self, p: &str) -> FsResult<()> {
        self.0.lock().unwrap().remove(p)
    }
    fn rename(&mut self, a: &str, b: &str) -> FsResult<()> {
        self.0.lock().unwrap().rename(a, b)
    }
    fn space(&mut self) -> FsResult<(u64, u64)> {
        self.0.lock().unwrap().space()
    }
    fn dataset_of(&mut self, p: &str) -> FsResult<String> {
        self.0.lock().unwrap().dataset_of(p)
    }
    fn aside_begin(&mut self, d: &str) -> FsResult<Aside> {
        self.0.lock().unwrap().aside_begin(d)
    }
    fn aside_changes(&mut self, a: &Aside) -> FsResult<Vec<FsChange>> {
        self.0.lock().unwrap().aside_changes(a)
    }
    fn aside_apply(&mut self, a: &Aside) -> FsResult<Applied> {
        self.0.lock().unwrap().aside_apply(a)
    }
    fn aside_discard(&mut self, a: &Aside) -> FsResult<()> {
        self.0.lock().unwrap().aside_discard(a)
    }
    fn view(&mut self, d: &str) -> FsResult<ViewRef> {
        self.0.lock().unwrap().view(d)
    }
    fn release_view(&mut self, v: &ViewRef) -> FsResult<()> {
        self.0.lock().unwrap().release_view(v)
    }
    fn changed_since(&mut self, v: &ViewRef) -> FsResult<Vec<FsChange>> {
        self.0.lock().unwrap().changed_since(v)
    }
    fn view_stat(&mut self, v: &ViewRef, p: &str) -> FsResult<Meta> {
        self.0.lock().unwrap().view_stat(v, p)
    }
    fn view_list(&mut self, v: &ViewRef, p: &str) -> FsResult<Vec<DirEntry>> {
        self.0.lock().unwrap().view_list(v, p)
    }
    fn view_read(&mut self, v: &ViewRef, p: &str) -> FsResult<Vec<u8>> {
        self.0.lock().unwrap().view_read(v, p)
    }
}

#[derive(Default)]
struct CondNotify {
    flag: Mutex<bool>,
    cv: Condvar,
}

impl Notify for CondNotify {
    fn notify(&self) {
        *self.flag.lock().unwrap() = true;
        self.cv.notify_all();
    }
    fn wait(&self, ns: i64) {
        let g = self.flag.lock().unwrap();
        let (mut g, _) = self.cv.wait_timeout_while(g, Duration::from_nanos(ns.max(0) as u64), |f| !*f).unwrap();
        *g = false;
    }
}

struct StdJoin(std::thread::JoinHandle<()>);

impl Joinable for StdJoin {
    fn join(self: Box<Self>) {
        self.0.join().unwrap();
    }
}

fn wall() -> Option<i64> {
    Some(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as i64)
}

fn nap(ns: i64) {
    std::thread::sleep(Duration::from_nanos(ns.max(0) as u64));
}

struct ThreadEnv {
    fs: SharedFs,
    notify: Arc<CondNotify>,
    /// Bez vlaken (ulohy na miste) - stejne vysledky.
    threads: bool,
}

impl Env for ThreadEnv {
    fn now(&mut self) -> Option<i64> {
        wall()
    }
    fn sleep(&mut self, ns: i64) {
        nap(ns)
    }
    fn spawn(&mut self, job: Box<dyn FnOnce() + Send>) -> Option<Box<dyn Joinable>> {
        if !self.threads {
            return None;
        }
        let h = std::thread::Builder::new().stack_size(8 << 20).spawn(job).ok()?;
        Some(Box::new(StdJoin(h)))
    }
    fn task_fs(&mut self, _task: u64) -> Option<Box<dyn Fs + Send>> {
        Some(Box::new(self.fs.clone()))
    }
    fn notifier(&mut self) -> Option<Arc<dyn Notify>> {
        Some(self.notify.clone())
    }
    fn clock(&self) -> Option<Clock> {
        Some(Clock { now: wall, sleep: nap })
    }
}

struct Rig {
    svc: Service,
    link: TestLink,
    fs: SharedFs,
    env: ThreadEnv,
    session: u64,
}

impl Rig {
    fn new(threads: bool) -> Self {
        let mut m = MemFs::new();
        m.put("/home/a.txt", b"alfa");
        let fs = SharedFs(Arc::new(Mutex::new(m)));
        let env = ThreadEnv { fs: fs.clone(), notify: Arc::default(), threads };
        let mut r = Rig { svc: Service::new("/home"), link: TestLink::default(), fs, env, session: 0 };
        r.handle(ClientMsg::Attach { session: 0 });
        r.session = r
            .link
            .sent
            .iter()
            .find_map(|m| match m {
                ServerMsg::Attached { session, .. } => Some(*session),
                _ => None,
            })
            .unwrap();
        r
    }

    fn handle(&mut self, m: ClientMsg) {
        self.svc.handle(m, &mut self.link, &mut self.fs, &mut self.env);
    }

    /// Spusti radek; vrati polozky jeho bloku a jestli prosel.
    fn run(&mut self, line: &str) -> (Vec<Item>, bool) {
        let from = self.link.sent.len();
        self.handle(ClientMsg::Run { session: self.session, line: line.into() });
        let sent = &self.link.sent[from..];
        let block = sent
            .iter()
            .find_map(|m| match m {
                ServerMsg::BlockStart { block, line: l, .. } if l == line => Some(*block),
                _ => None,
            })
            .unwrap();
        let items = sent
            .iter()
            .filter_map(|m| match m {
                ServerMsg::Item { block: b, item, .. } if *b == block => Some(item.clone()),
                _ => None,
            })
            .collect();
        let ok = sent.iter().any(|m| matches!(m, ServerMsg::BlockEnd { block: b, ok: true, .. } if *b == block));
        (items, ok)
    }

    fn error_code(items: &[Item]) -> Option<String> {
        items.iter().find_map(|i| match i {
            Item::Error { code, .. } => Some(code.clone()),
            _ => None,
        })
    }

    /// Bloky uloh (radek zacina `async `).
    fn task_blocks(&self) -> Vec<u64> {
        self.link
            .sent
            .iter()
            .filter_map(|m| match m {
                ServerMsg::BlockStart { block, line, .. } if line.starts_with("async ") => Some(*block),
                _ => None,
            })
            .collect()
    }

    fn block_end(&self, block: u64) -> Option<bool> {
        self.link.sent.iter().find_map(|m| match m {
            ServerMsg::BlockEnd { block: b, ok, .. } if *b == block => Some(*ok),
            _ => None,
        })
    }

    /// Volna chvile, dokud `f` neplati (nejvys 5 s).
    fn idle_until(&mut self, f: impl Fn(&Self) -> bool) {
        let t = Instant::now();
        while !f(self) {
            assert!(t.elapsed() < Duration::from_secs(5), "cekani na ulohy vyprselo");
            self.svc.idle(&mut self.link, &mut self.fs, &mut self.env);
            nap(10_000_000);
        }
    }

    fn file(&self, p: &str) -> Option<Vec<u8>> {
        self.fs.0.lock().unwrap().get(p).map(|b| b.to_vec())
    }
}

fn last_value(items: &[Item]) -> Option<Value> {
    items.iter().rev().find_map(|i| match i {
        Item::Value(v) => Some(v.clone()),
        _ => None,
    })
}

#[test]
fn tasks_run_in_parallel_threads_with_own_blocks() {
    let mut r = Rig::new(true);
    let t = Instant::now();
    let (items, ok) = r.run("let a = async { sleep 400ms; 1 }\nlet b = async { sleep 400ms; 2 }\nawait all [a, b]");
    let took = t.elapsed();
    assert!(ok, "{items:?}");
    assert_eq!(last_value(&items), Some(Value::List(vec![Value::Int(1), Value::Int(2)])));
    // Soubezne: obe po 400 ms, dohromady zretelne min nez 800 ms.
    assert!(took < Duration::from_millis(750), "{took:?}");
    let blocks = r.task_blocks();
    assert_eq!(blocks.len(), 2);
    for b in blocks {
        assert_eq!(r.block_end(b), Some(true));
        // Vysledek ulohy je i v jejim bloku.
        assert!(r.svc.sessions[0].blocks.iter().any(|x| x.id == b && x.ok == Some(true) && !x.items.is_empty()));
    }
}

#[test]
fn same_results_with_and_without_threads() {
    for threads in [true, false] {
        let mut r = Rig::new(threads);
        let (items, ok) = r.run("let x = 5\nlet a = async { x * 2 }\nlet b = async { \"y\" }\nawait any [a]");
        assert!(ok, "{items:?}");
        assert_eq!(last_value(&items), Some(Value::Int(10)));
        let (items, _) = r.run("let e = async { error \"spatne\" }\nawait e");
        assert_eq!(Rig::error_code(&items).as_deref(), Some(code::USER), "{threads}");
        let (items, ok) = r.run("let e = async { error \"spatne\" }\nlet v = try await e\nv.failed");
        assert!(ok);
        assert_eq!(last_value(&items), Some(Value::Bool(true)));
        let (items, ok) = r.run("[1, 2, 3] | each --parallel 2 x => x + 1");
        assert!(ok, "{items:?}");
        assert_eq!(last_value(&items), Some(Value::List(vec![Value::Int(2), Value::Int(3), Value::Int(4)])));
    }
}

#[test]
fn timeout_then_cancel_and_await_all_fails_fast() {
    let mut r = Rig::new(true);
    let (items, _) = r.run("let s = async { sleep 3s; 1 }\nlet v = try await s timeout 100ms\nv.error.code");
    assert_eq!(last_value(&items), Some(Value::str("TIMED_OUT")), "{items:?}");
    // Uloha bezi dal (relace), zrusi ji `cancel`.
    let (_, ok) = r.run("cancel $s");
    assert!(ok);
    let b = r.task_blocks()[0];
    r.idle_until(|r| r.block_end(b).is_some());
    assert_eq!(r.block_end(b), Some(false));
    // await all: chyba jedne ulohy hned, druha (dlouha) se zrusi.
    let t = Instant::now();
    let (items, _) = r.run("let a = async { sleep 3s }\nlet b = async { error \"b\" }\nawait all [a, b]");
    assert_eq!(Rig::error_code(&items).as_deref(), Some(code::USER), "{items:?}");
    assert!(t.elapsed() < Duration::from_secs(2));
}

#[test]
fn ctrl_c_interrupts_await_and_cancels() {
    let mut r = Rig::new(true);
    r.link.incoming.push_back(ClientMsg::Interrupt { session: r.session });
    let t = Instant::now();
    let (items, ok) = r.run("let s = async { sleep 3s }\nawait s");
    assert!(!ok);
    assert_eq!(Rig::error_code(&items).as_deref(), Some(code::INTERRUPTED), "{items:?}");
    assert!(t.elapsed() < Duration::from_secs(2));
    let b = r.task_blocks()[0];
    r.idle_until(|r| r.block_end(b).is_some());
}

#[test]
fn task_changes_files_with_own_undo_and_cancel_discards_draft() {
    let mut r = Rig::new(true);
    let (items, ok) = r.run("let w = async { echo novy > c.txt }\nawait w");
    assert!(ok, "{items:?}");
    assert_eq!(r.file("/home/c.txt").as_deref(), Some(&b"novy"[..]));
    let (items, ok) = r.run("undo");
    assert!(ok, "{items:?}");
    assert_eq!(r.file("/home/c.txt"), None);
    // Zrusena uloha: rozepsany soubor se nezapise.
    let (_, ok) = r.run("let z = async { echo x > d.txt; sleep 3s }\nsleep 200ms\ncancel $z");
    assert!(ok);
    let b = *r.task_blocks().last().unwrap();
    r.idle_until(|r| r.block_end(b).is_some());
    assert_eq!(r.file("/home/d.txt"), None);
    // Konec skriptu (blok) zrusi jeho ulohy.
    let (items, ok) = r.run("fn f() { let q = async { echo y > e.txt; sleep 3s }; 1 }\nf");
    assert!(ok, "{items:?}");
    let b = *r.task_blocks().last().unwrap();
    r.idle_until(|r| r.block_end(b).is_some());
    assert_eq!(r.file("/home/e.txt"), None);
}

#[test]
fn closures_cannot_cross_into_threads() {
    let mut r = Rig::new(true);
    let (items, ok) = r.run("let f = x => x + 1\nlet t = async { [1] | each $f }");
    assert!(!ok);
    assert_eq!(Rig::error_code(&items).as_deref(), Some(code::TYPE), "{items:?}");
}

#[test]
fn task_prompt_is_routed_and_refused_without_window() {
    let mut r = Rig::new(true);
    r.svc.sessions[0].interp.lock_threshold = 0;
    // Okno odpovi ano: uloha zapise.
    r.link.incoming.push_back(ClientMsg::Answer { prompt: 1, yes: true });
    let (items, ok) = r.run("let w = async { echo p > p.txt }\nawait w");
    assert!(ok, "{items:?}");
    assert!(r.link.sent.iter().any(|m| matches!(m, ServerMsg::Prompt { .. })));
    assert_eq!(r.file("/home/p.txt").as_deref(), Some(&b"p"[..]));
    // Bez okna: odmitnuto.
    r.handle(ClientMsg::Detach { session: r.session });
    let s = r.session;
    r.handle(ClientMsg::Run { session: s, line: "let w = async { echo q > q.txt }\nawait w".into() });
    assert_eq!(r.file("/home/q.txt"), None);
}
