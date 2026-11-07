//! Vyrazy (spolecne pro oba zapisy): literaly, operatory, closure, `if`,
//! `for`, `while`, `try`, `bg`, volani funkci a retez `.prikaz(...)`.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use super::{Parser, Style, describe, is_keyword};
use crate::ast::{Arg, BinOp, Closure, Expr, Lit, Num, Part, ROW, UnOp};
use crate::error::{ShError, code};
use crate::lex::{RawPart, Tok, is_ident_start};
use crate::span::Span;

fn binop(t: &Tok) -> Option<BinOp> {
    let Tok::Punct(p) = t else { return None };
    Some(match *p {
        "+" => BinOp::Add,
        "-" => BinOp::Sub,
        "*" => BinOp::Mul,
        "/" => BinOp::Div,
        "%" => BinOp::Rem,
        "==" => BinOp::Eq,
        "!=" => BinOp::Ne,
        "<" => BinOp::Lt,
        "<=" => BinOp::Le,
        ">" => BinOp::Gt,
        ">=" => BinOp::Ge,
        "&&" => BinOp::And,
        "||" => BinOp::Or,
        _ => return None,
    })
}

/// Prevod `jmeno_s_podtrzitkem` (C) na jmeno prikazu `jmeno-s-podtrzitkem`,
/// pokud takovy prikaz existuje.
pub(crate) fn demangle(p: &Parser, name: &str) -> String {
    if name.contains('_') {
        let dashed = name.replace('_', "-");
        if p.sigs.get(&dashed).is_some() {
            return dashed;
        }
    }
    name.into()
}

impl<'a> Parser<'a> {
    /// Cely vyraz vcetne `&&`, `||` a closure.
    pub(crate) fn expr(&mut self) -> Result<Expr, ShError> {
        if let Some(c) = self.try_closure()? {
            return Ok(c);
        }
        self.expr_bp(0)
    }

    /// Vyraz s operatory priority alespon `min` (3 = bez `&&`/`||`).
    pub(crate) fn expr_bp(&mut self, min: u8) -> Result<Expr, ShError> {
        self.enter()?;
        let r = self.expr_bp_inner(min);
        self.leave();
        r
    }

    fn expr_bp_inner(&mut self, min: u8) -> Result<Expr, ShError> {
        let mut left = self.unary()?;
        loop {
            let t = self.peek()?;
            let Some(op) = binop(&t) else { break };
            if op.prec() < min {
                break;
            }
            self.next()?;
            self.lx.skip_space_nl();
            let right = self.expr_bp(op.prec() + 1)?;
            let span = left.span().to(right.span());
            left = Expr::Binary(op, Box::new(left), Box::new(right), span);
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, ShError> {
        self.enter()?;
        let r = self.unary_inner();
        self.leave();
        r
    }

    fn unary_inner(&mut self) -> Result<Expr, ShError> {
        let t = self.peek()?;
        let op = match t {
            Tok::Punct("!") => Some(UnOp::Not),
            Tok::Punct("-") => Some(UnOp::Neg),
            _ => None,
        };
        if let Some(op) = op {
            let (_, span) = self.next()?;
            let e = self.unary()?;
            // `-5` je literal, ne operace (aby B `-5` a C `-5` daly totez).
            if op == UnOp::Neg
                && let Expr::Lit(lit, s) = &e
            {
                let neg = match lit {
                    Lit::Int(i) => Some(Lit::Int(-i)),
                    Lit::Float(f) => Some(Lit::Float(-f)),
                    Lit::Unit { value: Num::Int(i), unit } => Some(Lit::Unit { value: Num::Int(-i), unit: *unit }),
                    Lit::Unit { value: Num::Float(f), unit } => {
                        Some(Lit::Unit { value: Num::Float(-f), unit: *unit })
                    }
                    _ => None,
                };
                if let Some(n) = neg {
                    return Ok(Expr::Lit(n, span.to(*s)));
                }
            }
            let s = span.to(e.span());
            return Ok(Expr::Unary(op, Box::new(e), s));
        }
        let e = self.primary()?;
        self.postfix(e)
    }

    /// `.pole`, `.prikaz(...)`, `[i]`, `?`. Tecka smi byt i na dalsim radku
    /// (retez v C pres vic radku).
    pub(crate) fn postfix(&mut self, mut e: Expr) -> Result<Expr, ShError> {
        loop {
            let save = self.lx.pos;
            // Bez mezery: `[`, `?`, `.`; po novem radku jen `.jmeno`.
            match self.lx.peek_char() {
                Some('[') => {
                    self.lx.bump();
                    self.lx.skip_space_nl();
                    let idx = self.expr()?;
                    self.lx.skip_space_nl();
                    let end = self.expect_punct("]")?;
                    let span = e.span().to(end);
                    e = Expr::Index(Box::new(e), Box::new(idx), span);
                    continue;
                }
                Some('?') => {
                    self.lx.bump();
                    let span = e.span().to(self.here());
                    e = Expr::Unwrap(Box::new(e), span);
                    continue;
                }
                _ => {}
            }
            let line = self.lx.pos;
            self.lx.skip_space_nl();
            let next_line = self.lx.src[line..self.lx.pos].contains('\n');
            if next_line && !self.method_follows() {
                self.lx.pos = save;
                return Ok(e);
            }
            if self.lx.peek_char() == Some('.') && self.lx.peek_char2().is_some_and(is_ident_start) {
                self.lx.bump();
                let name_start = self.lx.pos;
                let name: String = self.lx.ident().into();
                let name_span = Span::new(name_start, self.lx.pos);
                if name == "await" && self.lx.peek_char() != Some('(') {
                    // Zapis C `uloha.await`.
                    let span = e.span().to(name_span);
                    e = Expr::Await { kind: crate::ast::AwaitKind::One, target: Box::new(e), timeout: None, span };
                    continue;
                }
                if self.lx.peek_char() == Some('(') {
                    // Cast roury: `x.where(...)`
                    let name = demangle(self, &name);
                    let args = self.call_args()?;
                    let cspan = name_span.to(self.here());
                    self.check_call(&name, &args, cspan)?;
                    let call = Expr::Call { name, args, span: cspan };
                    let span = e.span().to(self.here());
                    e = match e {
                        Expr::Pipe(mut stages, _) => {
                            stages.push(call);
                            Expr::Pipe(stages, span)
                        }
                        other => Expr::Pipe(alloc::vec![other, call], span),
                    };
                } else {
                    let span = e.span().to(name_span);
                    e = Expr::Field(Box::new(e), name, span);
                }
                continue;
            }
            self.lx.pos = save;
            return Ok(e);
        }
    }

    /// Nasleduje `.jmeno(` (pokracovani retezu na dalsim radku)?
    fn method_follows(&self) -> bool {
        let mut l = self.lx.clone();
        if l.peek_char() != Some('.') {
            return false;
        }
        l.bump();
        !l.ident().is_empty() && l.peek_char() == Some('(')
    }

    /// Argumenty volani v zapisu C: `(a, b, jmeno: hodnota)`.
    pub(crate) fn call_args(&mut self) -> Result<Vec<Arg>, ShError> {
        self.expect_punct("(")?;
        let mut args = Vec::new();
        loop {
            self.lx.skip_space_nl();
            if self.eat_punct(")")? {
                break;
            }
            // `jmeno: hodnota`
            let save = self.lx.pos;
            let mut named = None;
            if let (Tok::Ident(n), _) = self.next()?
                && self.lx.peek_char() == Some(':')
            {
                self.lx.bump();
                named = Some(n);
            }
            if named.is_none() {
                self.lx.pos = save;
            }
            self.lx.skip_space_nl();
            let value = self.expr()?;
            args.push(match named {
                Some(n) => Arg::Named(n, value),
                None => Arg::Pos(value),
            });
            self.lx.skip_space_nl();
            if !self.eat_punct(",")? {
                self.lx.skip_space_nl();
                self.expect_punct(")")?;
                break;
            }
        }
        Ok(args)
    }

    /// Closure `x => ...` nebo `(a, b) => ...`, pokud tu zacina.
    pub(crate) fn try_closure(&mut self) -> Result<Option<Expr>, ShError> {
        self.lx.skip_space();
        let start = self.lx.pos;
        let save = self.lx.pos;
        let mut params = Vec::new();
        match match self.lx.token() { Ok(t) => t, Err(_) => { self.lx.pos = save; return Ok(None); } } {
            (Tok::Ident(n), _) if !is_keyword(&n) => params.push(n),
            (Tok::Punct("("), _) => loop {
                self.lx.skip_space_nl();
                match match self.lx.token() { Ok(t) => t, Err(_) => { self.lx.pos = save; return Ok(None); } } {
                    (Tok::Punct(")"), _) => break,
                    (Tok::Ident(n), _) if !is_keyword(&n) => {
                        params.push(n);
                        self.lx.skip_space_nl();
                        match match self.lx.token() { Ok(t) => t, Err(_) => { self.lx.pos = save; return Ok(None); } } {
                            (Tok::Punct(","), _) => continue,
                            (Tok::Punct(")"), _) => break,
                            _ => {
                                self.lx.pos = save;
                                return Ok(None);
                            }
                        }
                    }
                    _ => {
                        self.lx.pos = save;
                        return Ok(None);
                    }
                }
            },
            _ => {
                self.lx.pos = save;
                return Ok(None);
            }
        }
        if !self.eat_punct("=>")? {
            self.lx.pos = save;
            return Ok(None);
        }
        self.lx.skip_space_nl();
        let body = self.closure_body(&params)?;
        let span = Span::new(start, self.lx.pos);
        Ok(Some(Expr::Closure(Closure { params, body: Box::new(body) }, span)))
    }

    /// Telo closure: blok `{ }` nebo jeden vyraz (v B jedna cast roury;
    /// rouru dejte do zavorek nebo do bloku).
    pub(crate) fn closure_body(&mut self, params: &[String]) -> Result<Expr, ShError> {
        self.enter()?;
        let r = self.closure_body_inner(params);
        self.leave();
        r
    }

    fn closure_body_inner(&mut self, params: &[String]) -> Result<Expr, ShError> {
        if self.is_punct("{")? && self.brace_is_block()? {
            let (b, span) = self.block(params)?;
            return Ok(Expr::Block(b, span));
        }
        self.push_scope();
        for p in params {
            self.declare(p);
        }
        // Telo closure uz neni podminka `where` (pole jen pres parametr).
        let row = core::mem::take(&mut self.row);
        let r = match self.style {
            Style::C => self.expr(),
            Style::B => self.b_stage(true),
        };
        self.row = row;
        self.pop_scope();
        r
    }

    /// Je `{` na aktualni pozici blok (a ne zaznam)?
    pub(crate) fn brace_is_block(&mut self) -> Result<bool, ShError> {
        let save = self.lx.pos;
        self.lx.skip_space();
        self.lx.bump(); // {
        self.lx.skip_space_nl();
        let r = match self.lx.token() {
            Ok((Tok::Punct("}"), _)) => false,
            Ok((Tok::Ident(_) | Tok::Str(_), _)) => self.lx.peek_char() != Some(':'),
            _ => true,
        };
        self.lx.pos = save;
        Ok(r)
    }

    fn primary(&mut self) -> Result<Expr, ShError> {
        let (t, span) = self.next()?;
        match t {
            Tok::Num(Num::Int(i)) => Ok(Expr::Lit(Lit::Int(i), span)),
            Tok::Num(Num::Float(f)) => Ok(Expr::Lit(Lit::Float(f), span)),
            Tok::Unit(value, unit) => Ok(Expr::Lit(Lit::Unit { value, unit }, span)),
            Tok::Str(parts) => self.interp(parts, span),
            Tok::Ident(w) => self.ident_expr(w, span),
            Tok::Punct("(") => {
                self.lx.skip_space_nl();
                // V C a v podmince `where` je obsah zavorky vyraz (pole radku
                // zustavaji), jinak v B roura podle rozsahu.
                let inner = if self.style == Style::C || !self.row.is_empty() {
                    self.expr()?
                } else {
                    let row = core::mem::take(&mut self.row);
                    let r = self.or_chain();
                    self.row = row;
                    r?
                };
                self.lx.skip_space_nl();
                self.expect_punct(")")?;
                Ok(inner)
            }
            Tok::Punct("[") => {
                let mut items = Vec::new();
                loop {
                    self.lx.skip_space_nl();
                    if self.eat_punct("]")? {
                        break;
                    }
                    items.push(self.expr()?);
                    self.lx.skip_space_nl();
                    if !self.eat_punct(",")? {
                        self.lx.skip_space_nl();
                        self.expect_punct("]")?;
                        break;
                    }
                }
                Ok(Expr::List(items, span.to(self.here())))
            }
            Tok::Punct("{") => {
                self.lx.pos = span.start as usize;
                if self.brace_is_block()? {
                    let (b, s) = self.block(&[])?;
                    return Ok(Expr::Block(b, s));
                }
                self.lx.pos = span.end as usize;
                self.record(span)
            }
            Tok::Punct("$") => {
                let name: String = self.lx.ident().into();
                Err(self
                    .err("ve vyrazu se promenna pise bez '$'", span)
                    .hint(alloc::format!("napiste jen '{name}'; '$' patri jen do argumentu prikazu")))
            }
            other => Err(self.err(alloc::format!("cekam vyraz, ale je tu {}", describe(&other)), span)),
        }
    }

    fn record(&mut self, start: Span) -> Result<Expr, ShError> {
        let mut fields = Vec::new();
        loop {
            self.lx.skip_space_nl();
            if self.eat_punct("}")? {
                break;
            }
            let (t, kspan) = self.next()?;
            let key = match t {
                Tok::Ident(s) => s,
                Tok::Str(parts) => match parts.as_slice() {
                    [RawPart::Text(s)] => s.clone(),
                    _ => return Err(self.err("klic zaznamu nesmi obsahovat {vyraz}", kspan)),
                },
                other => return Err(self.err(alloc::format!("cekam klic zaznamu, je tu {}", describe(&other)), kspan)),
            };
            self.expect_punct(":")?;
            self.lx.skip_space_nl();
            let v = self.expr()?;
            fields.push((key, v));
            self.lx.skip_space_nl();
            if !self.eat_punct(",")? {
                self.lx.skip_space_nl();
                self.expect_punct("}")?;
                break;
            }
        }
        Ok(Expr::Record(fields, start.to(self.here())))
    }

    /// Retezec: bez vlozenych vyrazu literal, jinak `Interp`.
    pub(crate) fn interp(&mut self, raw: Vec<RawPart>, span: Span) -> Result<Expr, ShError> {
        if let [RawPart::Text(s)] = raw.as_slice() {
            return Ok(Expr::Lit(Lit::Str(s.clone()), span));
        }
        let saved = self.lx.pos;
        let mut parts = Vec::new();
        for p in raw {
            match p {
                RawPart::Text(s) => parts.push(Part::Text(s)),
                RawPart::Code(s, e) => {
                    self.lx.pos = s;
                    self.lx.skip_space();
                    let ex = self.expr()?;
                    self.lx.skip_space();
                    if self.lx.pos != e {
                        return Err(self.err("ve vlozenem {vyrazu} je neco navic", Span::new(self.lx.pos, e)));
                    }
                    parts.push(Part::Expr(ex));
                }
            }
        }
        self.lx.pos = saved;
        Ok(Expr::Interp(parts, span))
    }

    fn ident_expr(&mut self, w: String, span: Span) -> Result<Expr, ShError> {
        match w.as_str() {
            "true" => return Ok(Expr::Lit(Lit::Bool(true), span)),
            "false" => return Ok(Expr::Lit(Lit::Bool(false), span)),
            "null" => return Ok(Expr::Lit(Lit::Null, span)),
            "if" => return self.if_expr(span),
            "for" => return self.for_expr(span),
            "while" => return self.while_expr(span),
            "try" => return self.try_expr(span),
            "bg" => {
                self.lx.skip_space();
                let (b, s) = self.block(&[])?;
                return Ok(Expr::Bg(b, span.to(s)));
            }
            "async" => {
                self.lx.skip_space();
                let (b, s) = self.block(&[])?;
                return Ok(Expr::Async(b, span.to(s)));
            }
            "await" => return self.await_expr(span),
            _ if is_keyword(&w) => {
                return Err(self.err(alloc::format!("'{w}' tu nemuze byt"), span));
            }
            _ => {}
        }
        // Volani funkce `jmeno(...)` (zavorka hned za jmenem).
        if self.lx.peek_char() == Some('(') {
            if w == "glob" {
                return self.glob_call(span);
            }
            if w == "exec" {
                return self.exec_call(span);
            }
            let name = demangle(self, &w);
            let args = self.call_args()?;
            let span = span.to(self.here());
            self.check_call(&name, &args, span)?;
            return Ok(Expr::Call { name, args, span });
        }
        if let Some(row) = self.row.last() {
            let inner = self.is_inner_var(&w, row.scope_depth);
            if !inner && self.is_var(&w) {
                if let Some(fields) = row.fields
                    && fields.contains(&w.as_str())
                {
                    return Err(ShError::new(
                        code::PARSE,
                        alloc::format!("'{w}' je promenna i pole radku - neni jasne, co myslite"),
                    )
                    .at(span)
                    .hint(alloc::format!("pole radku: it.{w}; jinak promennou prejmenujte")));
                }
                return Ok(Expr::Var(w, span));
            }
            if inner {
                return Ok(Expr::Var(w, span));
            }
            // Hole jmeno v podmince = pole radku.
            return Ok(Expr::Field(Box::new(Expr::Var(ROW.into(), span)), w, span));
        }
        if self.is_var(&w) {
            return Ok(Expr::Var(w, span));
        }
        let mut e = ShError::new(code::UNKNOWN, alloc::format!("nezname jmeno '{w}'")).at(span);
        if self.sigs.get(&demangle(self, &w)).is_some() {
            e = e.hint(match self.style {
                Style::C => alloc::format!("prikaz se v zapisu C vola jako funkce: {w}(...)"),
                Style::B => alloc::format!("prikaz se ve vyrazu vola jako funkce {w}(...) nebo v zavorce ({w} ...)"),
            });
        } else {
            e = e.hint("promennou zavedte pres let");
        }
        Err(e)
    }

    /// `exec("./prog", args...)`: program, jehoz jmeno neni identifikator
    /// (cesta, jmeno s `-` bez podpisu). Strom je obycejne volani.
    fn exec_call(&mut self, span: Span) -> Result<Expr, ShError> {
        let mut args = self.call_args()?;
        let name = match args.first() {
            Some(Arg::Pos(Expr::Lit(Lit::Str(n), _))) if !n.is_empty() => n.clone(),
            _ => return Err(self.err("exec() ceka jmeno programu v uvozovkach: exec(\"./prog\", ...)", span)),
        };
        args.remove(0);
        let span = span.to(self.here());
        self.check_call(&name, &args, span)?;
        Ok(Expr::Call { name, args, span })
    }

    fn glob_call(&mut self, span: Span) -> Result<Expr, ShError> {
        self.expect_punct("(")?;
        self.lx.skip_space();
        let (t, s) = self.next()?;
        let pat = match t {
            Tok::Str(parts) => match parts.as_slice() {
                [RawPart::Text(p)] => p.clone(),
                _ => return Err(self.err("vzor v glob() musi byt pevny text", s)),
            },
            _ => return Err(self.err("glob() cekam vzor v uvozovkach: glob(\"*.txt\")", s)),
        };
        self.lx.skip_space();
        let end = self.expect_punct(")")?;
        Ok(Expr::Glob(pat, span.to(end)))
    }

    fn if_expr(&mut self, start: Span) -> Result<Expr, ShError> {
        self.enter()?;
        let r = self.if_inner(start);
        self.leave();
        r
    }

    fn if_inner(&mut self, start: Span) -> Result<Expr, ShError> {
        self.lx.skip_space();
        let cond = self.expr()?;
        self.lx.skip_space();
        let (then, mut end) = self.block(&[])?;
        let save = self.lx.pos;
        self.lx.skip_space_nl();
        let mut els = None;
        if self.is_ident("else")? {
            self.next()?;
            self.lx.skip_space();
            if self.is_ident("if")? {
                let (_, s) = self.next()?;
                let e = self.if_expr(s)?;
                end = e.span();
                els = Some(Box::new(e));
            } else {
                let (b, s) = self.block(&[])?;
                end = s;
                els = Some(Box::new(Expr::Block(b, s)));
            }
        } else {
            self.lx.pos = save;
        }
        Ok(Expr::If { cond: Box::new(cond), then, els, span: start.to(end) })
    }

    fn for_expr(&mut self, start: Span) -> Result<Expr, ShError> {
        let (var, _) = self.expect_ident()?;
        if !self.is_ident("in")? {
            return Err(self.err("za promennou cyklu patri 'in' (for f in seznam { })", self.here()));
        }
        self.next()?;
        self.lx.skip_space();
        let iter = self.expr()?;
        self.lx.skip_space();
        let (body, end) = self.block(core::slice::from_ref(&var))?;
        Ok(Expr::For { var, iter: Box::new(iter), body, span: start.to(end) })
    }

    fn while_expr(&mut self, start: Span) -> Result<Expr, ShError> {
        self.lx.skip_space();
        let cond = self.expr()?;
        self.lx.skip_space();
        let (body, end) = self.block(&[])?;
        Ok(Expr::While { cond: Box::new(cond), body, span: start.to(end) })
    }

    /// `await x`, `await all [..]`, `await any [..]` (`timeout d`).
    fn await_expr(&mut self, start: Span) -> Result<Expr, ShError> {
        use crate::ast::AwaitKind;
        self.lx.skip_space();
        let mut kind = AwaitKind::One;
        {
            let mut l = self.lx.clone();
            let w = l.ident();
            let after = l.peek_char();
            if (w == "all" || w == "any") && matches!(after, Some(' ' | '\t' | '[' | '(')) {
                kind = if w == "all" { AwaitKind::All } else { AwaitKind::Any };
                self.lx = l;
                self.lx.skip_space();
            }
        }
        let target = self.unary()?;
        let save = self.lx.pos;
        self.lx.skip_space();
        let mut timeout = None;
        if self.is_ident("timeout")? {
            self.next()?;
            self.lx.skip_space();
            timeout = Some(Box::new(self.unary()?));
        } else {
            self.lx.pos = save;
        }
        Ok(Expr::Await { kind, target: Box::new(target), timeout, span: start.to(self.here()) })
    }

    fn try_expr(&mut self, start: Span) -> Result<Expr, ShError> {
        self.lx.skip_space();
        let body = if self.is_punct("{")? && self.brace_is_block()? {
            self.block(&[])?.0
        } else {
            let e = match self.style {
                Style::C => self.expr()?,
                Style::B => {
                    let prev = core::mem::replace(&mut self.stop_else, true);
                    let r = self.b_stage(true);
                    self.stop_else = prev;
                    r?
                }
            };
            crate::ast::Block { stmts: alloc::vec![crate::ast::Stmt::Expr(e)] }
        };
        let save = self.lx.pos;
        self.lx.skip_space_nl();
        let mut handler = None;
        if self.is_ident("else")? {
            self.next()?;
            self.lx.skip_space();
            if self.is_punct("{")? && self.brace_is_block()? {
                let (b, s) = self.block(&[])?;
                handler = Some(Closure { params: Vec::new(), body: Box::new(Expr::Block(b, s)) });
            } else {
                match self.try_closure()? {
                    Some(Expr::Closure(c, _)) => handler = Some(c),
                    _ => {
                        return Err(self
                            .err("za 'else' u try patri closure nebo blok", self.here())
                            .hint("try { ... } else e => log \"{e.message}\""));
                    }
                }
            }
        } else {
            self.lx.pos = save;
        }
        Ok(Expr::Try { body, handler, span: start.to(self.here()) })
    }
}
