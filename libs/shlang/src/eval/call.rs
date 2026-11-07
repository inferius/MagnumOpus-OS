//! Volani: vestavene prikazy, funkce `fn`, closure v promenne, programy.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use super::{Flow, Interp, R};
use crate::ast::{Arg, Expr};
use crate::error::{ShError, code};
use crate::host::{ExecNeeds, Host};
use crate::sig::{Kind, Sig, builtin};
use crate::span::Span;
use crate::value::{ClosureVal, Value};

/// Vyhodnocene argumenty prikazu.
#[derive(Debug, Default)]
pub struct Args {
    pub pos: Vec<Value>,
    pub named: Vec<(String, Value)>,
    pub span: Span,
}

impl Args {
    pub fn pos(&self, i: usize) -> Option<&Value> {
        self.pos.get(i)
    }

    pub fn flag(&self, name: &str) -> bool {
        self.named.iter().any(|(n, v)| n == name && *v == Value::Bool(true))
    }

    pub fn named(&self, name: &str) -> Option<&Value> {
        self.named.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    pub fn err(&self, code: &str, msg: impl Into<String>) -> Flow {
        Flow::Err(ShError::new(code, msg).at(self.span))
    }
}

/// Nejpodobnejsi jmeno (pro radu "myslel jste ...").
fn nearest<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    fn dist(a: &str, b: &str) -> usize {
        let a: Vec<char> = a.chars().collect();
        let b: Vec<char> = b.chars().collect();
        let mut prev: Vec<usize> = (0..=b.len()).collect();
        for i in 1..=a.len() {
            let mut cur = alloc::vec![i; b.len() + 1];
            for j in 1..=b.len() {
                let c = if a[i - 1] == b[j - 1] { 0 } else { 1 };
                cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + c);
            }
            prev = cur;
        }
        prev[b.len()]
    }
    candidates.map(|c| (dist(name, c), c)).filter(|(d, _)| *d <= 2).min_by_key(|(d, _)| *d).map(|(_, c)| c)
}

impl Interp {
    pub(crate) fn call(&mut self, host: &mut dyn Host, name: &str, args: &[Arg], input: Value, span: Span) -> R<Value> {
        if let Some(sig) = builtin(name) {
            let a = self.eval_args(host, Some(sig), args, span, false)?;
            return super::builtins::call(self, host, sig.name, a, input);
        }
        if let Some(f) = self.fns.get(name).cloned() {
            let a = self.eval_args(host, None, args, span, false)?;
            return self.call_fn(host, name, &f, a, input);
        }
        if let Some(Value::Closure(c)) = self.var(name).cloned() {
            let a = self.eval_args(host, None, args, span, false)?;
            if !a.named.is_empty() {
                return Err(a.err(code::ARGS, "closure nema pojmenovane argumenty"));
            }
            let mut pos = a.pos;
            if !matches!(input, Value::Nothing) {
                pos.insert(0, input);
            }
            return self.call_closure(host, &c, pos);
        }
        // Program.
        let a = self.eval_args(host, self.sigs.get(name).copied().as_ref(), args, span, true)?;
        let mut flat: Vec<(Option<String>, Value)> = a.pos.into_iter().map(|v| (None, v)).collect();
        flat.extend(a.named.into_iter().map(|(n, v)| (Some(n), v)));
        if self.preview_depth > 0 {
            return Err(Flow::Err(
                ShError::new(code::NEEDS, alloc::format!("program '{name}' nejde spustit nanecisto"))
                    .at(span)
                    .hint("v preview { } bezi jen vestavene prikazy; program meni soubory primo"),
            ));
        }
        let needs = self.exec_needs();
        // Program vidi skutecny FS: dosavadni zmeny radku prevzit (se
        // zamkem nebezpecnych operaci; odmitnuti = program se nespusti).
        self.commit_overlay(host).map_err(|e| Flow::Err(e.at(span)))?;
        // Kam program smi zapisovat; s plnymi pravy cely FS.
        let trees = if needs.all { alloc::vec![String::from("/")] } else { needs.write.clone() };
        let guarded = !trees.is_empty();
        if guarded && !self.undo.before_external(host.fs(), &self.line, &trees) {
            host.note("varovani: FS nema snapshoty, zmeny programu nepujde vratit");
        }
        let r = host.exec(name, &flat, input, &needs);
        if guarded {
            // Program nic nezmenil: zadny zaznam undo (jinak by `undo`
            // vratilo jeho prazdny zaznam misto predchoziho prikazu).
            self.undo.after_external(host.fs());
        }
        match r {
            Some(r) => r.map_err(|e| Flow::Err(e.at(span))),
            None => {
                let mut e = ShError::new(code::UNKNOWN, alloc::format!("neznamy prikaz '{name}'")).at(span);
                let mut names: Vec<&str> = self.sigs.names();
                let fns: Vec<String> = self.fns.keys().cloned().collect();
                names.extend(fns.iter().map(|s| s.as_str()));
                if let Some(n) = nearest(name, names.into_iter()) {
                    e = e.hint(alloc::format!("myslel jste '{n}'?"));
                } else {
                    e = e.hint("seznam prikazu: help");
                }
                Err(Flow::Err(e))
            }
        }
    }

    fn exec_needs(&self) -> ExecNeeds {
        match &self.needs {
            None => ExecNeeds { all: true, net: true, ..Default::default() },
            Some(n) => ExecNeeds { read: n.read.clone(), write: n.write.clone(), net: n.net, all: false },
        }
    }

    /// Argumenty: vzor souboru je jeden seznam cest (u programu se rozvine
    /// do vic argumentu), blok u parametru druhu `Block` se nevyhodnoti.
    fn eval_args(&mut self, host: &mut dyn Host, sig: Option<&Sig>, args: &[Arg], span: Span, splat: bool) -> R<Args> {
        let mut out = Args { span, ..Default::default() };
        let mut npos = 0;
        for a in args {
            match a {
                Arg::Pos(x) => {
                    let kind = sig.and_then(|s| s.pos_kind(npos));
                    npos += 1;
                    if kind == Some(Kind::Block)
                        && let Expr::Block(..) = x
                    {
                        let c = ClosureVal { params: Vec::new(), body: x.clone(), captured: Vec::new() };
                        out.pos.push(Value::Closure(alloc::rc::Rc::new(c)));
                        continue;
                    }
                    let v = self.eval(host, x)?;
                    match (splat, x, v) {
                        (true, Expr::Glob(..), Value::List(items)) => out.pos.extend(items),
                        (_, _, v) => out.pos.push(v),
                    }
                }
                Arg::Named(n, x) => {
                    let v = self.eval(host, x)?;
                    out.named.push((n.clone(), v));
                }
            }
        }
        Ok(out)
    }

    /// Zavola closure s argumenty (pocet musi sedet s parametry).
    pub(crate) fn call_closure(&mut self, host: &mut dyn Host, c: &ClosureVal, args: Vec<Value>) -> R<Value> {
        if args.len() != c.params.len() {
            return Err(Flow::Err(ShError::new(
                code::ARGS,
                alloc::format!("closure ceka {} argumentu, dostala {}", c.params.len(), args.len()),
            )));
        }
        self.enter_call()?;
        let mut frame = BTreeMap::new();
        for (k, v) in &c.captured {
            frame.insert(k.clone(), v.clone());
        }
        for (p, v) in c.params.iter().zip(args) {
            frame.insert(p.clone(), v);
        }
        self.frames.push(frame);
        let r = match &c.body {
            Expr::Block(b, _) => self.stmts(host, b, false),
            other => self.eval(host, other),
        };
        self.frames.pop();
        match r {
            Err(Flow::Return(v)) => Ok(v),
            Err(Flow::Break | Flow::Continue) => Err(Flow::Err(ShError::new(code::PARSE, "break/continue mimo cyklus"))),
            other => other,
        }
    }

    pub(crate) fn call_value(&mut self, host: &mut dyn Host, f: &Value, args: Vec<Value>) -> R<Value> {
        match f {
            Value::Closure(c) => self.call_closure(host, c, args),
            other => Err(Flow::Err(ShError::new(
                code::TYPE,
                alloc::format!("cekam closure (napr. x => x * 2), dostal jsem {}", other.type_name()),
            ))),
        }
    }

    fn call_fn(&mut self, host: &mut dyn Host, name: &str, f: &super::FnDef, a: Args, input: Value) -> R<Value> {
        let mut frame = BTreeMap::new();
        let mut pos = a.pos.into_iter();
        for p in &f.params {
            if let Some((_, v)) = a.named.iter().find(|(n, _)| n == p) {
                frame.insert(p.clone(), v.clone());
            } else if let Some(v) = pos.next() {
                frame.insert(p.clone(), v);
            } else {
                return Err(Flow::Err(
                    ShError::new(code::ARGS, alloc::format!("funkce {name}: chybi argument '{p}'")).at(a.span),
                ));
            }
        }
        if pos.next().is_some() {
            return Err(Flow::Err(ShError::new(code::ARGS, alloc::format!("funkce {name}: prilis mnoho argumentu")).at(a.span)));
        }
        for (n, _) in &a.named {
            if !f.params.contains(n) {
                return Err(Flow::Err(
                    ShError::new(code::ARGS, alloc::format!("funkce {name} nema parametr '{n}'")).at(a.span),
                ));
            }
        }
        frame.insert("input".into(), input);
        self.enter_call()?;
        // Funkce vidi jen globalni promenne a sve parametry.
        let saved: Vec<_> = self.frames.drain(1..).collect();
        self.frames.push(frame);
        let r = self.stmts(host, &f.body, false);
        self.frames.truncate(1);
        self.frames.extend(saved);
        match r {
            Err(Flow::Return(v)) => Ok(v),
            Err(Flow::Break | Flow::Continue) => Err(Flow::Err(ShError::new(code::PARSE, "break/continue mimo cyklus"))),
            other => other,
        }
    }

    fn enter_call(&mut self) -> R<()> {
        if self.frames.len() > self.max_depth {
            return Err(Flow::Err(
                ShError::new(code::DEPTH, alloc::format!("prilis hluboka rekurze (vic nez {})", self.max_depth)),
            ));
        }
        Ok(())
    }

}
