//! Soubeznost: `async { }`, `await`, `await all/any`, `timeout`, `cancel`,
//! `each --parallel N`.
//!
//! Uloha je kus kodu (blok nebo telo closure) s kopiemi zachycenych
//! promennych a s pravy volajiciho (`needs`). Interpret ji necha spustit
//! hostitele (`Host::task_spawn`): sluzba relaci ji pusti ve vlastnim
//! vlakne s vlastnim interpretem, konceptem souboru a blokem v terminalu.
//! Hostitel bez soubeznosti (testy, jednoduche prostredi) vrati `None`
//! a uloha probehne hned na miste - vysledek je stejny, jen bez
//! soubeznosti.
//!
//! Strukturovana soubeznost: uloha nepreziju skript ani volani funkce, ve
//! kterem vznikla - pri jejich konci se bezici ulohy zrusi. Interaktivni
//! radek patri relaci (`let a = async {...}` a `await a` muzou byt na dvou
//! radcich); takove ulohy konci s relaci nebo `cancel`.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use super::{Flow, Interp, R};
use crate::ast::{AwaitKind, Block, Expr, Script, Stmt};
use crate::error::{ShError, code};
use crate::host::{Host, TaskDone, TaskNeeds, TaskSpec};
use crate::span::Span;
use crate::value::{ClosureVal, Value};

/// Stav ulohy z pohledu interpretu.
#[derive(Debug, Clone)]
pub(crate) enum TaskSlot {
    Running,
    Done(Result<Value, ShError>),
}

/// Popisovac ulohy v jazyce: zaznam `{task: id}`.
pub(crate) fn handle(id: u64) -> Value {
    Value::record(&[("task", Value::Int(id as i64))])
}

fn task_id(v: &Value, span: Span) -> R<u64> {
    match v.field("task") {
        Some(Value::Int(i)) if i > 0 => Ok(i as u64),
        _ => Err(Flow::Err(
            ShError::new(code::TYPE, alloc::format!("čekám úlohu (async), dostal jsem {}", v.type_name()))
                .at(span)
                .hint("úlohu vytvoří: let u = async { ... }"),
        )),
    }
}

/// Vysledek bloku ulohy jako hodnota nebo chyba.
fn flow_result(r: R<Value>) -> Result<Value, ShError> {
    match r {
        Ok(v) | Err(Flow::Return(v)) => Ok(v),
        Err(Flow::Err(e)) => Err(e),
        Err(Flow::Break | Flow::Continue) => Err(ShError::new(code::PARSE, "break/continue mimo cyklus")),
    }
}

/// Kolik hotovych uloh si relace pamatuje (vysledek pro pozdejsi
/// `await`); starsi se zahodi (`await` na ne = `uloha N neexistuje`).
pub(crate) const KEEP_DONE: usize = 256;

impl Interp {
    /// Hotove ulohy nedrzet donekonecna: rozsahy potrebuji jen bezici
    /// (ruseni na konci bloku), z hotovych zustane poslednich `KEEP_DONE`.
    fn prune_tasks(&mut self) {
        let tasks = &self.tasks;
        for scope in &mut self.task_scopes {
            scope.retain(|id| matches!(tasks.get(id), Some(TaskSlot::Running)));
        }
        let done = self.tasks.values().filter(|s| matches!(s, TaskSlot::Done(_))).count();
        if done > KEEP_DONE {
            let old: Vec<u64> = self
                .tasks
                .iter()
                .filter(|(_, s)| matches!(s, TaskSlot::Done(_)))
                .map(|(id, _)| *id)
                .take(done - KEEP_DONE)
                .collect();
            for id in old {
                self.tasks.remove(&id);
            }
        }
    }

    /// Kod ulohy (zapis B) pro spusteni v jinem interpretu.
    fn task_code(&self, body: &Block, vars: &[(String, Value)]) -> String {
        let script = Script { lang: None, needs: None, body: body.clone() };
        let opts = crate::fmt::FmtOptions {
            style: crate::parse::Style::B,
            sigs: &*self.sigs,
            vars: vars.iter().map(|(n, _)| n.clone()).collect(),
        };
        crate::fmt::format_line(&script, &opts)
    }

    fn task_needs(&self) -> Option<TaskNeeds> {
        self.needs.as_ref().map(|n| TaskNeeds { read: n.read.clone(), write: n.write.clone(), net: n.net })
    }

    /// Spusti ulohu; bez soubeznosti hostitele probehne hned.
    fn spawn_task(&mut self, host: &mut dyn Host, body: &Block, vars: Vec<(String, Value)>, label: String) -> R<u64> {
        self.prune_tasks();
        let spec = TaskSpec {
            code: self.task_code(body, &vars),
            vars: vars.clone(),
            needs: self.task_needs(),
            cwd: self.cwd.clone(),
            label,
            lock_threshold: self.lock_threshold,
        };
        let id = match host.task_spawn(spec) {
            Some(r) => {
                let id = r.map_err(Flow::Err)?;
                self.tasks.insert(id, TaskSlot::Running);
                id
            }
            None => {
                // Na miste: kopie promennych v novem ramci.
                let id = self.next_task;
                self.next_task += 1;
                let mut frame = BTreeMap::new();
                for (k, v) in vars {
                    frame.insert(k, v);
                }
                self.frames.push(frame);
                let r = self.stmts(host, body, false);
                self.frames.pop();
                // Preruseni (Ctrl-C) se z ulohy nezachyti.
                if let Err(Flow::Err(e)) = &r
                    && e.code == code::INTERRUPTED
                {
                    return Err(Flow::Err(e.clone()));
                }
                self.tasks.insert(id, TaskSlot::Done(flow_result(r)));
                id
            }
        };
        if let Some(scope) = self.task_scopes.last_mut() {
            scope.push(id);
        }
        Ok(id)
    }

    /// `async { ... }`.
    pub(crate) fn eval_async(&mut self, host: &mut dyn Host, body: &Block, span: Span) -> R<Value> {
        let vars = self.make_closure(&[], &Expr::Block(body.clone(), span)).captured;
        let label = self.task_code(body, &vars);
        let id = self.spawn_task(host, body, vars, label)?;
        Ok(handle(id))
    }

    fn adopt(&mut self, host: &mut dyn Host, done: Vec<TaskDone>) {
        for d in done {
            self.undo.adopt(host.fs(), d.undo);
            self.tasks.insert(d.id, TaskSlot::Done(d.result));
        }
    }

    /// Pocka na ulohy `ids`: `any` = staci prvni hotova. Vrati poradi
    /// hotovych (prvni = prvni dokoncena).
    fn wait_tasks(&mut self, host: &mut dyn Host, ids: &[u64], any: bool, deadline: Option<i64>, span: Span) -> R<Vec<u64>> {
        let mut done: Vec<u64> =
            ids.iter().copied().filter(|i| matches!(self.tasks.get(i), Some(TaskSlot::Done(_)))).collect();
        let running: Vec<u64> =
            ids.iter().copied().filter(|i| matches!(self.tasks.get(i), Some(TaskSlot::Running))).collect();
        if let Some(bad) = ids.iter().find(|i| !self.tasks.contains_key(i)) {
            return Err(Flow::Err(ShError::new(code::ARGS, alloc::format!("úloha {bad} neexistuje")).at(span)));
        }
        if running.is_empty() || (any && !done.is_empty()) {
            return Ok(done);
        }
        let r = host.task_wait(&running, any, deadline);
        match r {
            Ok(finished) => {
                done.extend(finished.iter().map(|d| d.id));
                self.adopt(host, finished);
                Ok(done)
            }
            Err(e) => Err(Flow::Err(e.at(span))),
        }
    }

    /// `await`.
    pub(crate) fn eval_await(
        &mut self,
        host: &mut dyn Host,
        kind: AwaitKind,
        target: &Expr,
        timeout: Option<&Expr>,
        span: Span,
    ) -> R<Value> {
        let tv = self.eval(host, target)?;
        let ids: Vec<u64> = match kind {
            AwaitKind::One => alloc::vec![task_id(&tv, span)?],
            _ => match &tv {
                Value::List(items) => items.iter().map(|v| task_id(v, span)).collect::<R<_>>()?,
                other => {
                    return Err(Flow::Err(
                        ShError::new(code::TYPE, alloc::format!("await all/any čeká seznam úloh, ne {}", other.type_name()))
                            .at(span)
                            .hint("await all [a, b]"),
                    ));
                }
            },
        };
        let deadline = match timeout {
            Some(t) => match self.eval(host, t)? {
                Value::Duration(ns) => host.now().map(|n| n.saturating_add(ns)).or(Some(ns)),
                other => {
                    return Err(Flow::Err(
                        ShError::new(code::TYPE, alloc::format!("timeout je doba, ne {}", other.type_name()))
                            .at(span)
                            .hint("timeout 5s"),
                    ));
                }
            },
            None => None,
        };
        let done = self.wait_tasks(host, &ids, kind == AwaitKind::Any, deadline, span)?;
        let result = |s: &Self, id: u64| match s.tasks.get(&id) {
            Some(TaskSlot::Done(r)) => r.clone(),
            _ => Err(ShError::new(code::INTERRUPTED, alloc::format!("úloha {id} nedoběhla"))),
        };
        match kind {
            AwaitKind::One => result(self, ids[0]).map_err(|e| Flow::Err(e.at(span))),
            AwaitKind::Any => {
                let first = *done.first().ok_or_else(|| Flow::Err(ShError::new(code::ARGS, "await any: prázdný seznam").at(span)))?;
                result(self, first).map_err(|e| Flow::Err(e.at(span)))
            }
            AwaitKind::All => {
                // Prvni chyba (hostitel vraci driv, kdyz jedna selze): ostatni
                // jeste nemusi byt hotove.
                if let Some(e) = done.iter().find_map(|id| match self.tasks.get(id) {
                    Some(TaskSlot::Done(Err(e))) => Some(e.clone()),
                    _ => None,
                }) {
                    for other in &ids {
                        if matches!(self.tasks.get(other), Some(TaskSlot::Running)) {
                            self.cancel_task(host, *other);
                        }
                    }
                    return Err(Flow::Err(e.at(span)));
                }
                let mut out = Vec::new();
                for id in &ids {
                    match result(self, *id) {
                        Ok(v) => out.push(v),
                        Err(e) => {
                            // Chyba jedne ulohy: ostatni bezici zrusit (strukturovane).
                            for other in &ids {
                                if matches!(self.tasks.get(other), Some(TaskSlot::Running)) {
                                    self.cancel_task(host, *other);
                                }
                            }
                            return Err(Flow::Err(e.at(span)));
                        }
                    }
                }
                Ok(Value::List(out))
            }
        }
    }

    /// Zrusi ulohu (hotova zustane hotova; jeji undo se prevezme).
    pub(crate) fn cancel_task(&mut self, host: &mut dyn Host, id: u64) {
        if !matches!(self.tasks.get(&id), Some(TaskSlot::Running)) {
            return;
        }
        match host.task_cancel(id) {
            Some(d) => self.adopt(host, alloc::vec![d]),
            None => {
                self.tasks.insert(
                    id,
                    TaskSlot::Done(Err(ShError::new(code::INTERRUPTED, alloc::format!("úloha {id} byla zrušena")))),
                );
            }
        }
    }

    /// `cancel u`.
    pub(crate) fn cancel_value(&mut self, host: &mut dyn Host, v: &Value, span: Span) -> R<Value> {
        let ids: Vec<u64> = match v {
            Value::List(items) => items.iter().map(|x| task_id(x, span)).collect::<R<_>>()?,
            other => alloc::vec![task_id(other, span)?],
        };
        for id in ids {
            self.cancel_task(host, id);
        }
        Ok(Value::Nothing)
    }

    /// Novy rozsah uloh (skript, volani funkce).
    pub(crate) fn push_task_scope(&mut self) {
        self.task_scopes.push(Vec::new());
    }

    /// Konec rozsahu: bezici ulohy zrusit (nezustanou zapomenute).
    pub(crate) fn pop_task_scope(&mut self, host: &mut dyn Host) {
        let Some(ids) = self.task_scopes.pop() else { return };
        let mut cancelled = 0;
        for id in ids {
            if matches!(self.tasks.get(&id), Some(TaskSlot::Running)) {
                self.cancel_task(host, id);
                cancelled += 1;
            }
        }
        if cancelled > 0 {
            host.note(&alloc::format!("{cancelled} nedokončených úloh zrušeno (konec bloku)"));
        }
        if self.task_scopes.is_empty() {
            self.task_scopes.push(Vec::new());
        }
    }

    /// Shell je volny: vyzvednout ulohy, ktere mezitim dobehly (bez cekani),
    /// a prevzit jejich undo.
    pub fn reap_tasks(&mut self, host: &mut dyn Host) {
        let running: Vec<u64> =
            self.tasks.iter().filter(|(_, s)| matches!(s, TaskSlot::Running)).map(|(i, _)| *i).collect();
        if running.is_empty() {
            return;
        }
        let now = host.now().unwrap_or(0);
        if let Ok(done) = host.task_wait(&running, true, Some(now)) {
            self.adopt(host, done);
        }
    }

    /// Bezi nejaka uloha?
    pub fn tasks_running(&self) -> usize {
        self.tasks.values().filter(|s| matches!(s, TaskSlot::Running)).count()
    }

    /// `each --parallel N f`: nejvys N uloh najednou, vysledky v poradi.
    pub(crate) fn each_parallel(&mut self, host: &mut dyn Host, f: &ClosureVal, items: Vec<Value>, n: usize, span: Span) -> R<Value> {
        let n = n.max(1);
        let body = Block { stmts: alloc::vec![Stmt::Expr(f.body.clone())] };
        let param = f.params.first().cloned().unwrap_or_else(|| String::from(crate::ast::ROW));
        // Vysledky si vyzvedava prubezne (radku muze byt vic nez
        // `KEEP_DONE`, hotove ulohy se prorezavaji).
        let mut results: Vec<Option<Value>> = Vec::with_capacity(items.len());
        let mut running: Vec<(u64, usize)> = Vec::new();
        for (i, x) in items.into_iter().enumerate() {
            if running.len() >= n {
                let ids: Vec<u64> = running.iter().map(|(id, _)| *id).collect();
                self.wait_tasks(host, &ids, true, None, span)?;
                self.collect_each(host, &mut running, &mut results, span)?;
            }
            let mut vars = f.captured.clone();
            vars.retain(|(k, _)| *k != param);
            vars.push((param.clone(), x));
            let label = alloc::format!("each {}: {}", i + 1, self.task_code(&body, &vars));
            let id = self.spawn_task(host, &body, vars, label)?;
            results.push(None);
            running.push((id, i));
            self.collect_each(host, &mut running, &mut results, span)?;
        }
        while !running.is_empty() {
            let ids: Vec<u64> = running.iter().map(|(id, _)| *id).collect();
            self.wait_tasks(host, &ids, true, None, span)?;
            self.collect_each(host, &mut running, &mut results, span)?;
        }
        Ok(Value::List(results.into_iter().flatten().filter(|v| !matches!(v, Value::Nothing)).collect()))
    }

    /// Hotove ulohy `each --parallel`: vysledek na misto radku, slot pryc.
    /// Chyba jedne = ostatni zrusit a chybu vratit (jako `await all`).
    fn collect_each(
        &mut self,
        host: &mut dyn Host,
        running: &mut Vec<(u64, usize)>,
        results: &mut [Option<Value>],
        span: Span,
    ) -> R<()> {
        let mut err = None;
        running.retain(|(id, idx)| match self.tasks.get(id) {
            Some(TaskSlot::Running) => true,
            _ => {
                match self.tasks.remove(id) {
                    Some(TaskSlot::Done(Ok(v))) => results[*idx] = Some(v),
                    Some(TaskSlot::Done(Err(e))) => {
                        err.get_or_insert(e);
                    }
                    _ => {}
                }
                false
            }
        });
        if let Some(e) = err {
            for (id, _) in core::mem::take(running) {
                self.cancel_task(host, id);
            }
            return Err(Flow::Err(e.at(span)));
        }
        Ok(())
    }

    /// Detsky interpret ulohy (ve vlakne sluzby): spusti kod se
    /// zachycenymi promennymi a pravy volajiciho, prevezme koncept souboru
    /// (vlastni zaznam undo) a vrati vysledek.
    pub fn run_task(&mut self, host: &mut dyn Host, id: u64, spec: &TaskSpec) -> TaskDone {
        self.line = spec.label.clone();
        self.stack_base = 0;
        self.cwd = spec.cwd.clone();
        self.lock_threshold = spec.lock_threshold;
        self.needs = spec.needs.as_ref().map(|n| super::NeedsRt { read: n.read.clone(), write: n.write.clone(), net: n.net });
        let names: Vec<String> = spec.vars.iter().map(|(n, _)| n.clone()).collect();
        let parsed = crate::parse::parse(
            &spec.code,
            &crate::parse::Options { style: None, sigs: &*self.sigs, vars: names, script: false },
        );
        let r = match parsed {
            Ok(p) => {
                for (k, v) in &spec.vars {
                    self.frames[0].insert(k.clone(), v.clone());
                }
                self.push_task_scope();
                let r = flow_result(self.stmts(host, &p.script.body, false));
                self.pop_task_scope(host);
                r
            }
            Err(e) => Err(e),
        };
        // Zrusena (i po dobehnuti tela) nebo chybna uloha: koncept zahodit
        // (nic se nezmeni).
        let r = match r {
            Ok(_) if host.interrupted() => Err(ShError::new(code::INTERRUPTED, "úloha byla zrušena")),
            r => r,
        };
        if r.is_err() {
            self.draft.discard_all(host.fs());
        }
        let (r, _) = self.finish_value(host, r);
        let undo = core::mem::take(&mut self.undo.entries);
        TaskDone { id, result: r, undo }
    }
}
