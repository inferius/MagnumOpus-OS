//! Interpret nad spolecnym stromem.
//!
//! - Neosetrena chyba se siri sama a zastavi skript (`Flow::Err`).
//!   `try`, `||` a `try cmd` (vysledek jako hodnota) ji zachyti.
//!   Preruseni (Ctrl-C, `SH_INTERRUPTED`) zachytit nejde.
//! - Prikaz na nejvyssi urovni (radek, telo cyklu) vypise svou hodnotu
//!   (`Host::emit`); v tele closure a funkce je hodnotou posledni prikaz.
//! - Zmeny souboru jdou nejdriv do prekryvu ([`crate::vfs::overlay`]);
//!   na konci radku se prevezmou s linym undo, nad limit (zamek
//!   nebezpecnych operaci) az po potvrzeni.

mod builtins;
mod call;
mod fsctx;
pub mod ops;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::ast::{BinOp, Block, Expr, Part, Stmt, walk};
use crate::error::{ShError, Warning, code};
use crate::history::{Entry, History};
use crate::host::Host;
use crate::parse::{Options, parse};
use crate::sig::Sigs;
use crate::span::Span;
use crate::value::{ClosureVal, Value};
use crate::vfs::overlay::{DiffEntry, Overlay};
use crate::vfs::undo::UndoLog;

#[cfg(test)]
std::thread_local! {
    /// Nejnizsi adresa zasobniku pri vyhodnoceni (test velikosti ramce).
    pub static STACK_MIN: core::cell::Cell<usize> = const { core::cell::Cell::new(usize::MAX) };
}

/// Rizeni toku: chyba nebo `break`/`continue`/`return`.
#[derive(Debug)]
pub enum Flow {
    Err(ShError),
    Break,
    Continue,
    Return(Value),
}

impl From<ShError> for Flow {
    fn from(e: ShError) -> Self {
        Flow::Err(e)
    }
}

pub type R<T> = Result<T, Flow>;

/// Uzivatelska funkce `fn`.
#[derive(Debug)]
pub struct FnDef {
    pub params: Vec<String>,
    pub body: Block,
}

/// Uloha na pozadi (`cmd &`, `bg { }`). Bezi kooperativne: kdyz je shell
/// volny (`run_pending_jobs`) nebo pri `wait`.
#[derive(Debug)]
pub struct Job {
    pub id: u64,
    pub line: String,
    block: Block,
    vars: Vec<(String, Value)>,
    pub state: JobState,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JobState {
    Pending,
    Done(Value),
    Failed(ShError),
    Killed,
}

/// Omezeni z manifestu `needs` (absolutni cesty).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NeedsRt {
    pub read: Vec<String>,
    pub write: Vec<String>,
    pub net: bool,
}

/// Vysledek jednoho radku.
#[derive(Debug)]
pub struct Outcome {
    pub result: Result<(), ShError>,
    pub warnings: Vec<Warning>,
    /// Prevzate zmeny souboru (pro blok vystupu a undo).
    pub diff: Vec<DiffEntry>,
}

pub struct Interp {
    pub sigs: Box<dyn Sigs>,
    frames: Vec<BTreeMap<String, Value>>,
    fns: BTreeMap<String, Rc<FnDef>>,
    pub cwd: String,
    pub home: String,
    pub undo: UndoLog,
    overlay: Overlay,
    /// Prikaz, ktery zmeni vic souboru (mimo /tmp), se prevezme az po
    /// potvrzeni.
    pub lock_threshold: usize,
    /// Zmeny uz potvrdil `preview`.
    confirmed: bool,
    preview_depth: usize,
    pub history: History,
    needs: Option<NeedsRt>,
    pub jobs: Vec<Job>,
    next_job: u64,
    depth: usize,
    pub max_depth: usize,
    /// Kolik bajtu zasobniku smi interpret spotrebovat (od prvniho vyrazu
    /// radku); hostitel ho nastavi podle sveho zasobniku.
    pub stack_budget: usize,
    stack_base: usize,
    /// Text aktualniho radku (undo, historie, ulohy).
    line: String,
    /// `exit` - relace skonci.
    pub exit: bool,
}

impl Interp {
    pub fn new(sigs: Box<dyn Sigs>, home: &str) -> Self {
        Self {
            sigs,
            frames: alloc::vec![BTreeMap::new()],
            fns: BTreeMap::new(),
            cwd: home.into(),
            home: home.into(),
            undo: UndoLog::new(),
            overlay: Overlay::new(),
            lock_threshold: 20,
            confirmed: false,
            preview_depth: 0,
            history: History::default(),
            needs: None,
            jobs: Vec::new(),
            next_job: 1,
            depth: 0,
            max_depth: 256,
            stack_budget: 512 * 1024,
            stack_base: 0,
            line: String::new(),
            exit: false,
        }
    }

    /// Jmena promennych (pro parser: rozsah, `$x`, pole `where`).
    pub fn var_names(&self) -> Vec<String> {
        let mut v = Vec::new();
        for f in &self.frames {
            for k in f.keys() {
                if !v.contains(k) {
                    v.push(k.clone());
                }
            }
        }
        v
    }

    pub fn var(&self, name: &str) -> Option<&Value> {
        self.frames.iter().rev().find_map(|f| f.get(name))
    }

    pub fn set_var(&mut self, name: &str, v: Value) {
        self.frames[0].insert(name.into(), v);
    }

    pub fn fn_names(&self) -> Vec<String> {
        self.fns.keys().cloned().collect()
    }

    /// Spusti jeden interaktivni radek (nebo cely skript bez hlavicky).
    pub fn run_line(&mut self, host: &mut dyn Host, src: &str) -> Outcome {
        let start = host.now();
        let parsed = match parse(src, &Options { style: None, sigs: &*self.sigs, vars: self.var_names(), script: false }) {
            Ok(p) => p,
            Err(e) => {
                self.record(host, src, false, start);
                return Outcome { result: Err(e), warnings: Vec::new(), diff: Vec::new() };
            }
        };
        if parsed.script.lang.is_some() || parsed.script.needs.is_some() {
            let e = ShError::new(code::PARSE, "hlavicka 'lang'/'needs' patri do souboru skriptu (run skript)");
            return Outcome { result: Err(e), warnings: parsed.warnings, diff: Vec::new() };
        }
        self.line = src.trim().into();
        self.stack_base = 0;
        let r = self.eval_block_top(host, &parsed.script.body);
        let (r, diff) = self.finish(host, r);
        self.record(host, src, r.is_ok(), start);
        Outcome { result: r, warnings: parsed.warnings, diff }
    }

    fn record(&mut self, host: &mut dyn Host, src: &str, ok: bool, start: Option<i64>) {
        let now = host.now();
        let duration = match (start, now) {
            (Some(a), Some(b)) => Some(b - a),
            _ => None,
        };
        self.history.push(Entry { line: src.trim().into(), cwd: self.cwd.clone(), ok, duration, time: start });
    }

    /// Telo radku: na nejvyssi urovni se `return` chova jako konec.
    fn eval_block_top(&mut self, host: &mut dyn Host, body: &Block) -> Result<(), ShError> {
        match self.stmts(host, body, true) {
            Ok(_) | Err(Flow::Return(_)) => Ok(()),
            Err(Flow::Err(e)) => Err(e),
            Err(Flow::Break | Flow::Continue) => Err(ShError::new(code::PARSE, "break/continue mimo cyklus")),
        }
    }

    /// Konec radku: prevzeti zmen (zamek nebezpecnych operaci, undo).
    fn finish(&mut self, host: &mut dyn Host, r: Result<(), ShError>) -> (Result<(), ShError>, Vec<DiffEntry>) {
        let overlay = core::mem::take(&mut self.overlay);
        let confirmed = core::mem::replace(&mut self.confirmed, false);
        if overlay.is_empty() {
            return (r, Vec::new());
        }
        let diff = match overlay.changes(host.fs()) {
            Ok(d) => d,
            Err(e) => return (r.and(Err(ShError::sys(e, "zmeny nejde spocitat"))), Vec::new()),
        };
        let risky = diff.iter().filter(|d| !crate::vfs::path::within(&d.path, "/tmp")).count();
        if risky > self.lock_threshold && !confirmed {
            let q = alloc::format!(
                "Prikaz zmeni {risky} souboru (vic nez {}). Prevzit zmeny?",
                self.lock_threshold
            );
            if !host.confirm(&q, &crate::vfs::overlay::diff_value(&diff)) {
                let e = ShError::new(code::REJECTED, "zmeny nebyly prevzaty (nic se nezmenilo)");
                return (r.and(Err(e)), Vec::new());
            }
        }
        match self.undo.commit(host.fs(), &overlay, &self.line) {
            Ok(d) => (r, d),
            Err(e) => (r.and(Err(ShError::sys(e, "zmeny nejde zapsat"))), diff),
        }
    }

    /// Spusti skript ze souboru (s hlavickou `lang` a manifestem `needs`).
    pub fn run_script(&mut self, host: &mut dyn Host, src: &str) -> Result<Vec<Warning>, ShError> {
        self.line = String::from("run");
        self.stack_base = 0;
        let r = self.script_inner(host, src, None);
        let (r, _) = self.finish(host, r.map(|_| ()));
        r.map(|_| Vec::new())
    }

    fn script_inner(&mut self, host: &mut dyn Host, src: &str, path: Option<&str>) -> Result<Vec<Warning>, ShError> {
        let parsed = parse(src, &Options { style: None, sigs: &*self.sigs, vars: Vec::new(), script: true })?;
        for w in &parsed.warnings {
            host.note(&alloc::format!("varovani: {}", w.message));
        }
        let saved_needs = self.needs.clone();
        if let Some(n) = &parsed.script.needs {
            let rt = self.needs_rt(n);
            // Skript nesmi dostat vic nez ten, kdo ho spousti.
            if let Some(outer) = &self.needs {
                for p in rt.read.iter().chain(&rt.write) {
                    if !outer.read.iter().chain(&outer.write).any(|o| crate::vfs::path::within(p, o)) {
                        return Err(ShError::new(code::NEEDS, alloc::format!("skript chce {p}, to volajici nema")));
                    }
                }
            }
            let detail = Value::record(&[
                ("read", Value::List(rt.read.iter().cloned().map(Value::Str).collect())),
                ("write", Value::List(rt.write.iter().cloned().map(Value::Str).collect())),
                ("net", Value::Bool(rt.net)),
            ]);
            let q = alloc::format!("Skript {} chce tato opravneni. Povolit?", path.unwrap_or(""));
            if !host.confirm(&q, &detail) {
                return Err(ShError::new(code::REJECTED, "opravneni skriptu nebyla povolena"));
            }
            self.needs = Some(rt);
        } else if self.needs.is_none() {
            host.note("varovani: skript nema manifest 'needs', bezi s pravy relace");
        }
        self.frames.push(BTreeMap::new());
        let r = self.stmts(host, &parsed.script.body, true);
        self.frames.pop();
        self.needs = saved_needs;
        match r {
            Ok(_) | Err(Flow::Return(_)) => Ok(parsed.warnings),
            Err(Flow::Err(e)) => Err(e),
            Err(_) => Err(ShError::new(code::PARSE, "break/continue mimo cyklus")),
        }
    }

    fn needs_rt(&self, n: &crate::ast::Needs) -> NeedsRt {
        let abs = |p: &String| crate::vfs::path::resolve(&self.cwd, &self.home, p);
        NeedsRt { read: n.read.iter().map(abs).collect(), write: n.write.iter().map(abs).collect(), net: n.net.is_some() }
    }

    /// Spusti ulohy na pozadi, ktere cekaji (shell je volny).
    pub fn run_pending_jobs(&mut self, host: &mut dyn Host) -> Vec<(u64, Result<Value, ShError>)> {
        let ids: Vec<u64> = self.jobs.iter().filter(|j| j.state == JobState::Pending).map(|j| j.id).collect();
        let mut out = Vec::new();
        for id in ids {
            self.line = alloc::format!("uloha {id}");
            self.stack_base = 0;
            let r = self.run_job(host, id);
            let (r2, _) = self.finish(host, r.as_ref().map(|_| ()).map_err(|e| e.clone()));
            out.push((id, r2.and(r)));
        }
        out
    }

    fn run_job(&mut self, host: &mut dyn Host, id: u64) -> Result<Value, ShError> {
        let Some(i) = self.jobs.iter().position(|j| j.id == id) else {
            return Err(ShError::new(code::UNKNOWN, alloc::format!("uloha {id} neexistuje")));
        };
        if self.jobs[i].state != JobState::Pending {
            return match &self.jobs[i].state {
                JobState::Done(v) => Ok(v.clone()),
                JobState::Failed(e) => Err(e.clone()),
                _ => Err(ShError::new(code::INTERRUPTED, alloc::format!("uloha {id} byla ukoncena"))),
            };
        }
        let block = self.jobs[i].block.clone();
        let vars = self.jobs[i].vars.clone();
        let mut frame = BTreeMap::new();
        for (k, v) in vars {
            frame.insert(k, v);
        }
        self.frames.push(frame);
        let r = self.stmts(host, &block, false);
        self.frames.pop();
        let r = match r {
            Ok(v) | Err(Flow::Return(v)) => Ok(v),
            Err(Flow::Err(e)) => Err(e),
            Err(_) => Err(ShError::new(code::PARSE, "break/continue mimo cyklus")),
        };
        let i = self.jobs.iter().position(|j| j.id == id).unwrap();
        self.jobs[i].state = match &r {
            Ok(v) => JobState::Done(v.clone()),
            Err(e) => JobState::Failed(e.clone()),
        };
        r
    }

    // ---- vyhodnoceni ----

    fn check_interrupt(&mut self, host: &mut dyn Host) -> R<()> {
        if host.interrupted() {
            return Err(Flow::Err(ShError::new(code::INTERRUPTED, "preruseno (Ctrl-C)")));
        }
        Ok(())
    }

    /// Prikazy bloku. `emit` = vypsat hodnotu kazdeho prikazu (radek, telo
    /// cyklu); jinak je vysledkem hodnota posledniho prikazu.
    fn stmts(&mut self, host: &mut dyn Host, b: &Block, emit: bool) -> R<Value> {
        let mut last = Value::Nothing;
        for s in &b.stmts {
            self.check_interrupt(host)?;
            last = self.stmt(host, s, emit)?;
        }
        Ok(last)
    }

    pub(crate) fn block(&mut self, host: &mut dyn Host, b: &Block, emit: bool) -> R<Value> {
        self.frames.push(BTreeMap::new());
        let r = self.stmts(host, b, emit);
        self.frames.pop();
        r
    }

    fn stmt(&mut self, host: &mut dyn Host, s: &Stmt, emit: bool) -> R<Value> {
        match s {
            Stmt::Let { name, value, span } => {
                let v = self.eval(host, value).map_err(|f| at(f, *span))?;
                self.frames.last_mut().unwrap().insert(name.clone(), v);
                Ok(Value::Nothing)
            }
            Stmt::Assign { name, value, span } => {
                let v = self.eval(host, value).map_err(|f| at(f, *span))?;
                match self.frames.iter_mut().rev().find(|f| f.contains_key(name)) {
                    Some(f) => {
                        f.insert(name.clone(), v);
                        Ok(Value::Nothing)
                    }
                    None => Err(Flow::Err(
                        ShError::new(code::UNKNOWN, alloc::format!("promenna '{name}' neexistuje")).at(*span),
                    )),
                }
            }
            Stmt::Fn { name, params, body, .. } => {
                self.fns.insert(name.clone(), Rc::new(FnDef { params: params.clone(), body: body.clone() }));
                Ok(Value::Nothing)
            }
            Stmt::Return { value, .. } => {
                let v = match value {
                    Some(e) => self.eval(host, e)?,
                    None => Value::Nothing,
                };
                Err(Flow::Return(v))
            }
            Stmt::Break { .. } => Err(Flow::Break),
            Stmt::Continue { .. } => Err(Flow::Continue),
            Stmt::Comment(_) => Ok(Value::Nothing),
            Stmt::Expr(e) => {
                let v = self.eval_ctx(host, e, emit)?;
                if emit {
                    if !matches!(v, Value::Nothing) {
                        host.emit(v);
                    }
                    Ok(Value::Nothing)
                } else {
                    Ok(v)
                }
            }
        }
    }

    /// Vyraz na pozici prikazu: bloky `if`/`try` vypisuji jako radky.
    fn eval_ctx(&mut self, host: &mut dyn Host, e: &Expr, emit: bool) -> R<Value> {
        match e {
            Expr::If { cond, then, els, span } if emit => {
                if self.cond(host, cond, *span)? {
                    self.block(host, then, true)
                } else if let Some(x) = els {
                    match x.as_ref() {
                        Expr::Block(b, _) => self.block(host, b, true),
                        other => self.eval_ctx(host, other, true),
                    }
                } else {
                    Ok(Value::Nothing)
                }
            }
            _ => self.eval(host, e),
        }
    }

    fn cond(&mut self, host: &mut dyn Host, e: &Expr, span: Span) -> R<bool> {
        match self.eval(host, e)? {
            Value::Bool(b) => Ok(b),
            other => Err(Flow::Err(
                ShError::new(code::TYPE, alloc::format!("podminka musi byt bool, je to {}", other.type_name()))
                    .at(e.span().to(span))
                    .hint("porovnejte vyslovne, napr. x != null nebo n > 0"),
            )),
        }
    }

    pub(crate) fn eval(&mut self, host: &mut dyn Host, e: &Expr) -> R<Value> {
        let here = stack_addr();
        #[cfg(test)]
        STACK_MIN.with(|m| m.set(m.get().min(here)));
        if self.stack_base == 0 {
            self.stack_base = here;
        }
        if self.stack_base.saturating_sub(here) > self.stack_budget {
            return Err(Flow::Err(
                ShError::new(code::DEPTH, "prilis hluboke vnoreni (dosel zasobnik)")
                    .at(e.span())
                    .hint("nekonecna rekurze? hloubku omezuje velikost zasobniku shellu"),
            ));
        }
        self.depth += 1;
        let r = self.eval_inner(host, e);
        self.depth -= 1;
        r.map_err(|f| at(f, e.span()))
    }

    #[inline(never)]
    fn eval_inner(&mut self, host: &mut dyn Host, e: &Expr) -> R<Value> {
        match e {
            Expr::Lit(l, _) => Ok(ops::lit_value(l)?),
            Expr::Interp(parts, _) => {
                let mut s = String::new();
                for p in parts {
                    match p {
                        Part::Text(t) => s.push_str(t),
                        Part::Expr(x) => s.push_str(&self.eval(host, x)?.to_text()),
                    }
                }
                Ok(Value::Str(s))
            }
            Expr::Glob(p, span) => self.glob(host, p, *span),
            Expr::Var(name, span) => match self.var(name) {
                Some(v) => Ok(v.clone()),
                None => Err(Flow::Err(
                    ShError::new(code::UNKNOWN, alloc::format!("promenna '{name}' neexistuje")).at(*span),
                )),
            },
            Expr::Field(base, name, span) => {
                let b = self.eval(host, base)?;
                field(&b, name).map_err(|e| Flow::Err(e.at(*span)))
            }
            Expr::Index(base, idx, span) => {
                let b = self.eval(host, base)?;
                let i = self.eval(host, idx)?;
                index(&b, &i).map_err(|e| Flow::Err(e.at(*span)))
            }
            Expr::List(items, _) => {
                let mut v = Vec::with_capacity(items.len());
                for x in items {
                    v.push(self.eval(host, x)?);
                }
                Ok(Value::List(v))
            }
            Expr::Record(fields, _) => {
                let mut r = Vec::with_capacity(fields.len());
                for (k, x) in fields {
                    let v = self.eval(host, x)?;
                    r.push((k.clone(), v));
                }
                Ok(Value::Record(r))
            }
            Expr::Unary(op, x, _) => {
                let v = self.eval(host, x)?;
                Ok(ops::unary(*op, v)?)
            }
            Expr::Binary(BinOp::Or, l, r, _) => match self.eval(host, l) {
                Err(Flow::Err(e)) if e.code != code::INTERRUPTED => self.eval(host, r),
                Ok(Value::Bool(false)) => self.eval(host, r),
                other => other,
            },
            Expr::Binary(BinOp::And, l, r, _) => match self.eval(host, l)? {
                Value::Bool(false) => Ok(Value::Bool(false)),
                _ => self.eval(host, r),
            },
            Expr::Binary(op, l, r, _) => {
                let a = self.eval(host, l)?;
                let b = self.eval(host, r)?;
                Ok(ops::binary(*op, a, b)?)
            }
            Expr::Closure(c, _) => Ok(Value::Closure(Rc::new(self.make_closure(&c.params, &c.body)))),
            Expr::Block(b, _) => self.block(host, b, false),
            Expr::If { cond, then, els, span } => {
                if self.cond(host, cond, *span)? {
                    self.block(host, then, false)
                } else if let Some(x) = els {
                    self.eval(host, x)
                } else {
                    Ok(Value::Nothing)
                }
            }
            Expr::Call { name, args, span } => self.call(host, name, args, Value::Nothing, *span),
            Expr::For { .. } | Expr::While { .. } => self.eval_loop(host, e),
            Expr::Try { .. } => self.eval_try(host, e),
            Expr::Pipe(..) | Expr::Bg(..) | Expr::Unwrap(..) => self.eval_misc(host, e),
        }
    }

    #[inline(never)]
    fn eval_loop(&mut self, host: &mut dyn Host, e: &Expr) -> R<Value> {
        match e {
            Expr::For { var, iter, body, span } => {
                let items = match self.eval(host, iter)? {
                    Value::List(v) => v,
                    Value::Record(r) => r
                        .into_iter()
                        .map(|(k, v)| Value::record(&[("key", Value::Str(k)), ("value", v)]))
                        .collect(),
                    other => {
                        return Err(Flow::Err(
                            ShError::new(code::TYPE, alloc::format!("for prochazi seznam, ne {}", other.type_name()))
                                .at(*span)
                                .hint("text rozdelte na radky: lines"),
                        ));
                    }
                };
                for item in items {
                    self.check_interrupt(host)?;
                    self.frames.push(BTreeMap::new());
                    self.frames.last_mut().unwrap().insert(var.clone(), item);
                    let r = self.stmts(host, body, true);
                    self.frames.pop();
                    match r {
                        Ok(_) | Err(Flow::Continue) => {}
                        Err(Flow::Break) => break,
                        Err(other) => return Err(other),
                    }
                }
                Ok(Value::Nothing)
            }
            Expr::While { cond, body, span } => {
                while self.cond(host, cond, *span)? {
                    self.check_interrupt(host)?;
                    match self.block(host, body, true) {
                        Ok(_) | Err(Flow::Continue) => {}
                        Err(Flow::Break) => break,
                        Err(other) => return Err(other),
                    }
                }
                Ok(Value::Nothing)
            }
            _ => unreachable!(),
        }
    }

    #[inline(never)]
    fn eval_try(&mut self, host: &mut dyn Host, e: &Expr) -> R<Value> {
        match e {
            Expr::Try { body, handler, .. } => match self.block(host, body, false) {
                Ok(v) => Ok(match handler {
                    Some(_) => v,
                    None => Value::record(&[
                        ("ok", Value::Bool(true)),
                        ("failed", Value::Bool(false)),
                        ("value", v),
                        ("error", Value::Null),
                    ]),
                }),
                Err(Flow::Err(e)) if e.code != code::INTERRUPTED => match handler {
                    Some(h) => {
                        let c = self.make_closure(&h.params, &h.body);
                        let args = if h.params.is_empty() { Vec::new() } else { alloc::vec![Value::Error(Box::new(e))] };
                        self.call_closure(host, &c, args)
                    }
                    None => Ok(Value::record(&[
                        ("ok", Value::Bool(false)),
                        ("failed", Value::Bool(true)),
                        ("value", Value::Null),
                        ("error", Value::Error(Box::new(e))),
                    ])),
                },
                Err(other) => Err(other),
            },
            _ => unreachable!(),
        }
    }

    #[inline(never)]
    fn eval_misc(&mut self, host: &mut dyn Host, e: &Expr) -> R<Value> {
        match e {
            Expr::Pipe(stages, _) => {
                let mut v = self.eval(host, &stages[0])?;
                for s in &stages[1..] {
                    self.check_interrupt(host)?;
                    v = match s {
                        Expr::Call { name, args, span } => self.call(host, name, args, v, *span)?,
                        other => {
                            return Err(Flow::Err(
                                ShError::new(code::PARSE, "za '|' musi byt prikaz").at(other.span()),
                            ));
                        }
                    };
                }
                Ok(v)
            }
            Expr::Bg(b, _) => {
                let id = self.next_job;
                self.next_job += 1;
                let vars = self.visible_vars();
                self.jobs.push(Job { id, line: self.line.clone(), block: b.clone(), vars, state: JobState::Pending });
                host.note(&alloc::format!("[{id}] uloha na pozadi"));
                Ok(Value::record(&[("job", Value::Int(id as i64))]))
            }
            Expr::Unwrap(x, span) => match self.eval(host, x)? {
                Value::Error(e) => Err(Flow::Err(*e)),
                Value::Record(r)
                    if r.iter().any(|(k, _)| k == "failed") && r.iter().any(|(k, _)| k == "error") =>
                {
                    let get = |n: &str| r.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone()).unwrap();
                    match (get("failed"), get("error")) {
                        (Value::Bool(true), Value::Error(e)) => Err(Flow::Err(*e)),
                        _ => Ok(get("value")),
                    }
                }
                other => {
                    let _ = span;
                    Ok(other)
                }
            },
            _ => unreachable!(),
        }
    }

    fn visible_vars(&self) -> Vec<(String, Value)> {
        let mut out: Vec<(String, Value)> = Vec::new();
        for f in &self.frames {
            for (k, v) in f {
                match out.iter_mut().find(|(n, _)| n == k) {
                    Some(slot) => slot.1 = v.clone(),
                    None => out.push((k.clone(), v.clone())),
                }
            }
        }
        out
    }

    /// Closure se zachycenymi promennymi, ktere telo pouziva.
    fn make_closure(&self, params: &[String], body: &Expr) -> ClosureVal {
        let mut captured: Vec<(String, Value)> = Vec::new();
        walk(body, &mut |e| {
            if let Expr::Var(n, _) = e
                && !params.contains(n)
                && !captured.iter().any(|(c, _)| c == n)
                && let Some(v) = self.var(n)
            {
                captured.push((n.clone(), v.clone()));
            }
        });
        ClosureVal { params: params.into(), body: body.clone(), captured }
    }
}

/// Adresa na zasobniku (pro hlidani hloubky rekurze).
#[inline(never)]
fn stack_addr() -> usize {
    let x = 0u8;
    core::hint::black_box(&x) as *const u8 as usize
}

/// Doplni misto do chyby (jen kdyz ho jeste nema).
fn at(f: Flow, span: Span) -> Flow {
    match f {
        Flow::Err(e) => Flow::Err(e.at(span)),
        other => other,
    }
}

/// Pole hodnoty. Na seznamu zaznamu vrati sloupec.
pub fn field(b: &Value, name: &str) -> Result<Value, ShError> {
    if let Some(v) = b.field(name) {
        return Ok(v);
    }
    match b {
        Value::List(items) => {
            let mut out = Vec::with_capacity(items.len());
            for x in items {
                out.push(field(x, name)?);
            }
            Ok(Value::List(out))
        }
        Value::Record(_) | Value::Error(_) => {
            let names = b.field_names().join(", ");
            Err(ShError::new(code::UNKNOWN, alloc::format!("pole '{name}' neexistuje")).hint(alloc::format!("zaznam ma: {names}")))
        }
        other => Err(ShError::new(code::TYPE, alloc::format!("{} nema pole '{name}'", other.type_name()))),
    }
}

fn index(b: &Value, i: &Value) -> Result<Value, ShError> {
    match (b, i) {
        (Value::List(items), Value::Int(n)) => {
            let idx = if *n < 0 { items.len() as i64 + n } else { *n };
            items.get(idx as usize).cloned().filter(|_| idx >= 0).ok_or_else(|| {
                ShError::new(code::ARGS, alloc::format!("index {n} je mimo seznam (delka {})", items.len()))
            })
        }
        (Value::Record(_), Value::Str(k)) => field(b, k),
        (Value::Str(s), Value::Int(n)) => s
            .chars()
            .nth(*n as usize)
            .filter(|_| *n >= 0)
            .map(|c| Value::Str(c.to_string()))
            .ok_or_else(|| ShError::new(code::ARGS, alloc::format!("index {n} je mimo text"))),
        _ => Err(ShError::new(
            code::TYPE,
            alloc::format!("{} nejde indexovat hodnotou {}", b.type_name(), i.type_name()),
        )),
    }
}
