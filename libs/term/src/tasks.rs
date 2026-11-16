//! Ulohy `async` ve vlaknech sluzby relaci.
//!
//! Kazda uloha bezi ve vlastnim vlakne s vlastnim interpretem, vlastni
//! relaci FS (koncept souboru a undo uloh se neplete s radkem) a vlastnim
//! blokem v terminalu. Hodnoty mezi vlakny jdou zakodovane (stejne jako
//! pres protokol: interpret pouziva `Rc`, ktere vlakna sdilet nesmi), takze
//! closure do ulohy predat nejde.
//!
//! - vlakno uloh pise zpravy pro okno do sve schranky (`outbox`) a na konci
//!   ulozi vysledek; probudi cekajiciho (`Notify`, v systemu udalost jadra),
//! - hlavni vlakno relace zpravy preposila oknu a uklada do bloku,
//!   vyzvy (zamek nebezpecnych operaci) posila jako `Prompt` a odpoved
//!   vraci do ulohy,
//! - zruseni = priznak, ktery interpret ulohy cte (`interrupted`);
//!   zrusena uloha svuj koncept zahodi.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use shlang::eval::Interp;
use shlang::host::{Fs, Host, TaskDone, TaskSpec};
use shlang::sig::Builtins;
use shlang::value::Value;
use shlang::vfs::undo::UndoEntry;
use shlang::{ShError, error::code};
use spin::Mutex;

use crate::proto::{Item, R, ServerMsg, W, get_value, put_value};

/// Probuzeni cekajiciho (v systemu udalost jadra, v testech podminka).
pub trait Notify: Send + Sync {
    fn notify(&self);
    /// Pocka na `notify` nejvys `ns` nanosekund.
    fn wait(&self, ns: i64);
}

/// Vlakno, na jehoz konec jde pockat (uvolni zasobnik).
pub trait Joinable: Send {
    fn join(self: Box<Self>);
}

/// Cas pro vlakno uloh (funkce, ne `Env` - ten patri hlavnimu vlaknu).
#[derive(Clone, Copy)]
pub struct Clock {
    pub now: fn() -> Option<i64>,
    pub sleep: fn(i64),
}

/// Vyzva ulohy (zamek nebezpecnych operaci).
struct PromptWire {
    question: String,
    detail: Vec<u8>,
    /// Cislo vyzvy v protokolu (az ji hlavni vlakno posle oknu).
    sent: Option<u64>,
    answer: Option<bool>,
}

struct DoneWire {
    result: Result<Vec<u8>, ShError>,
    undo: Vec<UndoEntry>,
}

/// Rizeni ulohy z hlavniho vlakna: zruseni a probuzeni (odpoved na
/// vyzvu, zruseni). Uloha ceka na udalost, ne v kratkych spankach.
pub struct Control {
    cancel: AtomicBool,
    /// Vlastni probouzeni ulohy (`None` = prostredi ho nema: spanek).
    wake: Option<Arc<dyn Notify>>,
}

impl Control {
    fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
        self.wake();
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    fn wake(&self) {
        if let Some(w) = &self.wake {
            w.notify();
        }
    }

    /// Pocka nejvys `ns` (drive pri probuzeni).
    fn wait(&self, ns: i64, clock: &Clock) {
        match &self.wake {
            Some(w) => w.wait(ns),
            None => (clock.sleep)(ns.min(20_000_000)),
        }
    }
}

struct Entry {
    id: u64,
    control: Arc<Control>,
    outbox: Vec<Vec<u8>>,
    prompt: Option<PromptWire>,
    done: Option<DoneWire>,
    join: Option<Box<dyn Joinable>>,
    /// Nikdo uz na vysledek neceka (zrusena): po dobehu uklidit.
    abandoned: bool,
}

#[derive(Default)]
pub struct Table {
    entries: Vec<Entry>,
    /// Zpravy vyzvednutych uloh, ktere jeste nesly oknu (uloha posle konec
    /// bloku a hned nastavi vysledek - vyzvednuti nesmi zpravy ztratit).
    orphans: Vec<Vec<u8>>,
}

/// Sdilena tabulka uloh relace.
pub type Shared = Arc<Mutex<Table>>;

pub fn shared() -> Shared {
    Arc::new(Mutex::new(Table::default()))
}

fn has_closure(v: &Value) -> bool {
    match v {
        Value::Closure(_) => true,
        Value::List(xs) => xs.iter().any(has_closure),
        Value::Record(r) => r.iter().any(|(_, x)| has_closure(x)),
        _ => false,
    }
}

pub fn encode_value(v: &Value) -> Vec<u8> {
    let mut w = W::default();
    put_value(&mut w, v);
    w.0
}

pub fn decode_value(b: &[u8]) -> Value {
    get_value(&mut R::new(b), 0).unwrap_or(Value::Null)
}

/// Zadani ulohy pro vlakno (bez `Rc`).
pub struct SpecWire {
    pub code: String,
    pub vars: Vec<(String, Vec<u8>)>,
    pub needs: Option<shlang::host::TaskNeeds>,
    pub cwd: String,
    pub label: String,
    pub lock_threshold: usize,
}

impl SpecWire {
    /// Zakoduje zadani; closure v promennych = chyba (do vlakna nejde).
    pub fn of(spec: &TaskSpec) -> Result<SpecWire, ShError> {
        let mut vars = Vec::new();
        for (k, v) in &spec.vars {
            if has_closure(v) {
                return Err(ShError::new(code::TYPE, alloc::format!("promennou '{k}' (closure) nejde predat do ulohy"))
                    .hint("closure definujte uvnitr async { }"));
            }
            vars.push((k.clone(), encode_value(v)));
        }
        Ok(SpecWire { code: spec.code.clone(), vars, needs: spec.needs.clone(), cwd: spec.cwd.clone(), label: spec.label.clone(), lock_threshold: spec.lock_threshold })
    }
}

/// Zaregistruje ulohu (pred spustenim vlakna).
pub fn register(t: &Shared, id: u64, wake: Option<Arc<dyn Notify>>) -> Arc<Control> {
    let control = Arc::new(Control { cancel: AtomicBool::new(false), wake });
    t.lock().entries.push(Entry {
        id,
        control: control.clone(),
        outbox: Vec::new(),
        prompt: None,
        done: None,
        join: None,
        abandoned: false,
    });
    control
}

pub fn set_join(t: &Shared, id: u64, j: Box<dyn Joinable>) {
    if let Some(e) = t.lock().entries.iter_mut().find(|e| e.id == id) {
        e.join = Some(j);
    }
}

pub fn remove(t: &Shared, id: u64) {
    t.lock().entries.retain(|e| e.id != id);
}

/// Hlavni vlakno: zpravy pro okno a nove vyzvy (`next_prompt` = cislo
/// pro protokol). Vrati zpravy k odeslani a ulozeni do bloku.
pub fn drain(t: &Shared, next_prompt: &mut u64) -> Vec<ServerMsg> {
    let mut out = Vec::new();
    let mut tab = t.lock();
    for m in core::mem::take(&mut tab.orphans) {
        if let Ok(sm) = ServerMsg::decode(&m) {
            out.push(sm);
        }
    }
    for e in &mut tab.entries {
        for m in core::mem::take(&mut e.outbox) {
            if let Ok(sm) = ServerMsg::decode(&m) {
                out.push(sm);
            }
        }
        if let Some(p) = e.prompt.as_mut()
            && p.sent.is_none()
        {
            let id = *next_prompt;
            *next_prompt += 1;
            p.sent = Some(id);
            let detail = Item::Value(decode_value(&p.detail));
            out.push(ServerMsg::Prompt { prompt: id, question: p.question.clone(), detail, choices: Vec::new(), label: String::new() });
        }
    }
    // Zrusene a dobehle bez zmen souboru: uklidit. Se zmenami (dobehla
    // tesne pred zrusenim) zustanou pro `take_late` - undo se nesmi ztratit.
    let mut joins = Vec::new();
    tab.entries.retain_mut(|e| {
        if e.abandoned && e.done.as_ref().is_some_and(|d| d.undo.is_empty()) {
            if let Some(j) = e.join.take() {
                joins.push(j);
            }
            false
        } else {
            true
        }
    });
    drop(tab);
    for j in joins {
        j.join();
    }
    out
}

/// Odpoved okna na vyzvu ulohy; `false` = vyzva neni od uloh.
pub fn answer(t: &Shared, prompt: u64, yes: bool) -> bool {
    let mut tab = t.lock();
    for e in &mut tab.entries {
        if let Some(p) = e.prompt.as_mut()
            && p.sent == Some(prompt)
        {
            p.answer = Some(yes);
            e.control.wake();
            return true;
        }
    }
    false
}

pub fn is_done(t: &Shared, id: u64) -> bool {
    t.lock().entries.iter().any(|e| e.id == id && e.done.is_some())
}

/// Dobehla s chybou (`await all` pak neceka na ostatni).
pub fn failed(t: &Shared, id: u64) -> bool {
    t.lock().entries.iter().any(|e| e.id == id && matches!(&e.done, Some(d) if d.result.is_err()))
}

pub fn exists(t: &Shared, id: u64) -> bool {
    t.lock().entries.iter().any(|e| e.id == id)
}

/// Vyzvedne vysledek dobehle ulohy (a pocka na konec vlakna).
pub fn take(t: &Shared, id: u64) -> Option<TaskDone> {
    let (done, join) = {
        let mut tab = t.lock();
        let i = tab.entries.iter().position(|e| e.id == id && e.done.is_some())?;
        let mut e = tab.entries.remove(i);
        let rest = core::mem::take(&mut e.outbox);
        tab.orphans.extend(rest);
        (e.done.take()?, e.join.take())
    };
    if let Some(j) = join {
        j.join();
    }
    Some(TaskDone { id, result: done.result.map(|b| decode_value(&b)), undo: done.undo })
}

/// Zrusit: priznak pro interpret ulohy; hotova se vyzvedne. Kontrola
/// i zruseni pod jednim zamkem (uloha nesmi dobehnout mezi nimi).
pub fn cancel(t: &Shared, id: u64) -> Option<TaskDone> {
    let (done, join) = {
        let mut tab = t.lock();
        let i = tab.entries.iter().position(|e| e.id == id)?;
        if tab.entries[i].done.is_none() {
            let e = &mut tab.entries[i];
            e.control.cancel();
            e.abandoned = true;
            return None;
        }
        let mut e = tab.entries.remove(i);
        let rest = core::mem::take(&mut e.outbox);
        tab.orphans.extend(rest);
        (e.done.take()?, e.join.take())
    };
    if let Some(j) = join {
        j.join();
    }
    Some(TaskDone { id, result: done.result.map(|b| decode_value(&b)), undo: done.undo })
}

/// Pozadat ulohu o zruseni (prehled uloh, panel): jen priznak - vysledek
/// (`SH_INTERRUPTED`) si vyzvedne interpret jako u kazde jine ulohy.
pub fn request_cancel(t: &Shared, id: u64) {
    if let Some(e) = t.lock().entries.iter().find(|e| e.id == id && e.done.is_none()) {
        e.control.cancel();
    }
}

/// Zrusene ulohy, ktere presto stihly prevzit zmeny souboru (dobehly tesne
/// pred zrusenim): jejich undo prevezme relace.
pub fn take_late(t: &Shared) -> Vec<Vec<UndoEntry>> {
    let mut late = Vec::new();
    let mut joins = Vec::new();
    let mut orphans = Vec::new();
    let mut tab = t.lock();
    tab.entries.retain_mut(|e| {
        if e.abandoned && let Some(d) = e.done.as_mut() {
            late.push(core::mem::take(&mut d.undo));
            orphans.extend(core::mem::take(&mut e.outbox));
            if let Some(j) = e.join.take() {
                joins.push(j);
            }
            false
        } else {
            true
        }
    });
    tab.orphans.extend(orphans);
    drop(tab);
    for j in joins {
        j.join();
    }
    late
}

/// Zrusit vse (konec relace).
pub fn cancel_all(t: &Shared) {
    for e in &mut t.lock().entries {
        e.control.cancel();
        e.abandoned = true;
    }
}

/// Hostitel interpretu ve vlakne ulohy.
struct TaskHost {
    fs: Box<dyn Fs + Send>,
    table: Shared,
    notify: Arc<dyn Notify>,
    clock: Clock,
    control: Arc<Control>,
    id: u64,
    session: u64,
    block: u64,
    home: String,
    /// Polozky bloku (poradi pro aktualizace pruhu) a pruhy (id, index).
    items: usize,
    bars: Vec<(u32, usize)>,
}

impl TaskHost {
    fn send(&mut self, m: ServerMsg) {
        let bytes = m.encode();
        if let Some(e) = self.table.lock().entries.iter_mut().find(|e| e.id == self.id) {
            e.outbox.push(bytes);
        }
        self.notify.notify();
    }

    fn push(&mut self, item: Item) {
        self.items += 1;
        self.send(ServerMsg::Item { session: self.session, block: self.block, item });
    }
}

impl Host for TaskHost {
    fn fs(&mut self) -> &mut dyn Fs {
        &mut *self.fs
    }
    fn emit(&mut self, value: Value) {
        self.push(Item::Value(value));
    }
    fn note(&mut self, text: &str) {
        self.push(Item::Note(text.into()));
    }
    fn now(&mut self) -> Option<i64> {
        (self.clock.now)()
    }
    fn sleep(&mut self, ns: i64) {
        // Zruseni ulohu probudi hned (bez udalosti po kouscich).
        let Some(start) = (self.clock.now)() else {
            (self.clock.sleep)(ns);
            return;
        };
        let end = start.saturating_add(ns);
        while !self.interrupted() {
            let Some(now) = (self.clock.now)() else { return };
            if now >= end {
                return;
            }
            self.control.wait((end - now).min(500_000_000), &self.clock);
        }
    }
    fn interrupted(&mut self) -> bool {
        self.control.cancelled()
    }
    fn confirm(&mut self, question: &str, detail: &Value) -> bool {
        if let Some(e) = self.table.lock().entries.iter_mut().find(|e| e.id == self.id) {
            e.prompt = Some(PromptWire { question: question.into(), detail: encode_value(detail), sent: None, answer: None });
        }
        self.notify.notify();
        loop {
            if self.interrupted() {
                return false;
            }
            let a = self
                .table
                .lock()
                .entries
                .iter_mut()
                .find(|e| e.id == self.id)
                .and_then(|e| e.prompt.as_ref().and_then(|p| p.answer).map(|a| (a, &mut e.prompt)))
                .map(|(a, slot)| {
                    *slot = None;
                    a
                });
            if let Some(a) = a {
                self.push(Item::Note(alloc::format!("{question} -> {}", if a { "ano" } else { "ne" })));
                return a;
            }
            // Ceka na odpoved nebo zruseni (probudi `answer`/`cancel`).
            self.control.wait(1_000_000_000, &self.clock);
        }
    }
    fn progress(&mut self, id: u32, done: u64, total: u64, label: &str) {
        let item = Item::Progress { done, total, label: label.into() };
        match self.bars.iter().find(|(b, _)| *b == id).map(|(_, i)| *i) {
            Some(index) => {
                self.send(ServerMsg::ItemUpdate { session: self.session, block: self.block, index: index as u32, item })
            }
            None => {
                self.bars.push((id, self.items));
                self.push(item);
            }
        }
    }
    fn home(&self) -> String {
        self.home.clone()
    }
}

/// Telo vlakna ulohy.
#[allow(clippy::too_many_arguments)]
pub fn run(
    table: Shared,
    notify: Arc<dyn Notify>,
    clock: Clock,
    control: Arc<Control>,
    fs: Box<dyn Fs + Send>,
    session: u64,
    id: u64,
    block: u64,
    home: String,
    spec: SpecWire,
    stack_budget: usize,
) {
    let start = (clock.now)();
    let vars: Vec<(String, Value)> = spec.vars.iter().map(|(k, b)| (k.clone(), decode_value(b))).collect();
    let tspec = TaskSpec {
        code: spec.code,
        vars,
        needs: spec.needs,
        cwd: spec.cwd,
        label: spec.label,
        lock_threshold: spec.lock_threshold,
    };
    let mut host = TaskHost {
        fs,
        table: table.clone(),
        notify: notify.clone(),
        clock,
        control,
        id,
        session,
        block,
        home: home.clone(),
        items: 0,
        bars: Vec::new(),
    };
    let mut interp = Interp::new(Box::new(Builtins), &home);
    interp.stack_budget = stack_budget;
    let done = interp.run_task(&mut host, id, &tspec);
    interp.flush(&mut host);
    let ok = done.result.is_ok();
    if let Err(e) = &done.result {
        host.push(Item::Error { code: e.code.clone(), message: e.message.clone(), report: e.report("") });
    } else if let Ok(v) = &done.result
        && !matches!(v, Value::Nothing)
    {
        host.push(Item::Value(v.clone()));
    }
    let duration = match (start, (clock.now)()) {
        (Some(a), Some(b)) => b - a,
        _ => 0,
    };
    host.send(ServerMsg::BlockEnd { session, block, ok, duration });
    let wire = DoneWire { result: done.result.map(|v| encode_value(&v)), undo: done.undo };
    if let Some(e) = table.lock().entries.iter_mut().find(|e| e.id == id) {
        e.done = Some(wire);
    }
    notify.notify();
}
