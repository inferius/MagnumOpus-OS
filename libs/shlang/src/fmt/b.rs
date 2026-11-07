//! Tisk prikazoveho tvaru zapisu B: `prikaz arg -p`, roura `|`, `&&`/`||`,
//! presmerovani `>`, `&` a podminka `where` bez `it`.
//!
//! `tail` = za prikazem uz v prikazu nic nenasleduje (tak smi byt posledni
//! argument closure bez zavorek - jeji telo by jinak pohltilo `&&` nebo `>`).

use super::Printer;
use super::expr::{Ctx, quote};
use crate::ast::{Arg, BinOp, Closure, Expr, Lit, Part, ROW, Stmt, walk};
use crate::lex::{is_ident_char, is_ident_start, word_number};
use crate::parse::is_keyword;
use crate::sig::{Io, Kind};

fn valid_flag_name(n: &str) -> bool {
    n.chars().next().is_some_and(|c| c.is_alphanumeric())
        && n.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

fn valid_command_name(n: &str) -> bool {
    n.chars().next().is_some_and(|c| is_ident_start(c) || matches!(c, '.' | '/' | '~'))
        && n.chars().all(|c| is_ident_char(c) || matches!(c, '-' | '.' | '/' | '~'))
        && !is_keyword(n)
}

const SPECIAL: &[char] = &['|', ';', '&', '(', ')', '"', '\'', '<', '>', '{', '}', '$', '\\', '`'];

fn safe_glob(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with(['-', '#', ']'])
        && !p.chars().any(|c| c.is_whitespace() || SPECIAL.contains(&c))
}

/// Je `e` na urovni `&&`/`||` (a obsahuje prikaz)?
fn is_logic(e: &Expr) -> bool {
    matches!(e, Expr::Binary(BinOp::And | BinOp::Or, ..))
}

impl<'a> Printer<'a> {
    fn safe_bare(&self, s: &str) -> bool {
        !s.is_empty()
            && !s.starts_with(['-', '#', '[', ']', '='])
            && !s.chars().any(|c| c.is_whitespace() || SPECIAL.contains(&c) || matches!(c, '*' | '?' | '['))
            && !matches!(s, "true" | "false" | "null" | "else")
            && word_number(s).is_none()
            && !self.is_var(s)
    }

    /// Jde slovo s vlozenymi vyrazy zapsat bez uvozovek (`~/a/{x}`)?
    fn bare_interp(&self, parts: &[Part]) -> bool {
        matches!(parts.first(), Some(Part::Text(t)) if !t.starts_with(['-', '#', '[', ']', '~']) || t.starts_with("~/"))
            && parts.iter().all(|p| match p {
                Part::Text(t) => !t.chars().any(|c| c.is_whitespace() || SPECIAL.contains(&c) || matches!(c, '*' | '?' | '[')),
                Part::Expr(_) => true,
            })
    }

    fn output_fields(&self, e: &Expr) -> Option<&'static [&'static str]> {
        let Expr::Call { name, .. } = e else { return None };
        match self.sigs.get(name)?.output {
            Io::Records(f) if !f.is_empty() => Some(f),
            _ => None,
        }
    }

    /// Prikaz na pozici celeho prikazu.
    pub(crate) fn b_chain(&mut self, e: &Expr) {
        self.b_chain_t(e, true, None);
    }

    fn paren_chain(&mut self, e: &Expr) {
        self.out.push('(');
        self.b_chain_t(e, true, None);
        self.out.push(')');
    }

    fn b_chain_t(&mut self, e: &Expr, tail: bool, fields: Option<&'static [&'static str]>) {
        match e {
            Expr::Binary(BinOp::Or, l, r, _) => {
                match l.as_ref() {
                    Expr::Binary(BinOp::Or, ..) => self.b_chain_t(l, false, None),
                    other => self.b_and_operand(other, false),
                }
                self.out.push_str(" || ");
                self.b_and_operand(r, tail);
            }
            Expr::Binary(BinOp::And, ..) => self.b_and_operand(e, tail),
            Expr::Bg(b, _) => match b.stmts.as_slice() {
                [Stmt::Expr(x)] if !matches!(x, Expr::Bg(..)) => {
                    self.b_chain_t(x, false, None);
                    self.out.push_str(" &");
                }
                _ => {
                    self.out.push_str("bg ");
                    self.block(b, &[]);
                }
            },
            _ => self.b_pipeline_f(e, tail, fields),
        }
    }

    fn b_and_operand(&mut self, e: &Expr, tail: bool) {
        match e {
            Expr::Binary(BinOp::And, l, r, _) => {
                self.b_and_operand(l, false);
                self.out.push_str(" && ");
                match r.as_ref() {
                    x if is_logic(x) || matches!(x, Expr::Bg(..)) => self.paren_chain(x),
                    x => self.b_pipeline(x, tail),
                }
            }
            Expr::Binary(BinOp::Or, ..) | Expr::Bg(..) => self.paren_chain(e),
            _ => self.b_pipeline(e, tail),
        }
    }

    pub(crate) fn b_pipeline(&mut self, e: &Expr, tail: bool) {
        self.b_pipeline_f(e, tail, None);
    }

    fn b_pipeline_f(&mut self, e: &Expr, tail: bool, fields: Option<&'static [&'static str]>) {
        let Expr::Pipe(stages, _) = e else {
            self.b_first_stage(e, tail, fields);
            return;
        };
        // Cast, za kterou nasleduje `|`, je "na konci" (vyraz `|` nepohlti);
        // pred presmerovanim `>` ne (`>` by bylo porovnani).
        let n = stages.len();
        let redirect = save_redirect(&stages[n - 1]).is_some();
        let stage_tail = |i: usize| {
            if i + 1 == n {
                tail
            } else if i + 2 == n && redirect {
                false
            } else {
                true
            }
        };
        self.b_first_stage(&stages[0], stage_tail(0), None);
        let mut fields = self.output_fields(&stages[0]);
        for (i, s) in stages.iter().enumerate().skip(1) {
            let last = i + 1 == n;
            if last && let Some((path, append)) = save_redirect(s) {
                self.out.push_str(if append { " >> " } else { " > " });
                self.b_arg(path, true, Some(Kind::Path), None);
                return;
            }
            self.out.push_str(" | ");
            match s {
                Expr::Call { .. } => self.b_command_f(s, stage_tail(i), fields),
                other => self.expr(other, 7, Ctx::B),
            }
            fields = self.output_fields(s);
        }
    }

    fn b_first_stage(&mut self, e: &Expr, tail: bool, fields: Option<&'static [&'static str]>) {
        match e {
            Expr::Call { .. } => self.b_command_f(e, tail, fields),
            Expr::If { .. } | Expr::For { .. } | Expr::While { .. } | Expr::Bg(..) => self.expr(e, 0, Ctx::B),
            Expr::Try { .. } if tail => self.expr(e, 0, Ctx::B),
            Expr::Closure(c, _) if tail => self.closure(c, Ctx::B),
            _ => self.expr(e, 3, Ctx::B),
        }
    }

    /// Telo closure nebo `try cmd` v zapisu B (jedna cast roury).
    pub(crate) fn b_stage_body(&mut self, body: &Expr) {
        match body {
            Expr::Call { .. } => self.b_command(body, true),
            Expr::Pipe(..) => {
                self.out.push('(');
                self.b_pipeline(body, true);
                self.out.push(')');
            }
            x if is_logic(x) && has_command(x) => self.paren_chain(x),
            Expr::If { .. } | Expr::For { .. } | Expr::While { .. } | Expr::Bg(..) => self.expr(body, 0, Ctx::B),
            _ => self.expr(body, 0, Ctx::B),
        }
    }

    pub(crate) fn b_command(&mut self, e: &Expr, tail: bool) {
        self.b_command_f(e, tail, None);
    }

    fn b_command_f(&mut self, e: &Expr, tail: bool, fields: Option<&'static [&'static str]>) {
        let Expr::Call { name, args, .. } = e else {
            self.expr(e, 0, Ctx::B);
            return;
        };
        let sig = self.sigs.get(name).copied();
        // Jde to zapsat jako prikaz? Jinak tvar volani `jmeno(...)`.
        let mut ok = valid_command_name(name);
        let mut npos = 0;
        for a in args {
            match a {
                Arg::Pos(x) => {
                    let kind = sig.as_ref().and_then(|s| s.pos_kind(npos));
                    if kind == Some(Kind::Cond) && !matches!(x, Expr::Closure(..)) {
                        ok = false;
                    }
                    npos += 1;
                }
                Arg::Named(n, _) => {
                    if !valid_flag_name(n) || sig.as_ref().is_some_and(|s| s.flag_long(n).is_none()) {
                        ok = false;
                    }
                }
            }
        }
        if !ok {
            self.call_form(name, args, Ctx::B);
            return;
        }
        self.out.push_str(name);
        let mut npos = 0;
        let n = args.len();
        for (i, a) in args.iter().enumerate() {
            let arg_tail = tail && i + 1 == n;
            self.out.push(' ');
            match a {
                Arg::Pos(x) => {
                    let kind = sig.as_ref().and_then(|s| s.pos_kind(npos));
                    self.b_arg(x, arg_tail, kind, fields);
                    npos += 1;
                }
                Arg::Named(fname, v) => {
                    let flag = sig.as_ref().and_then(|s| s.flag_long(fname));
                    let is_true = matches!(v, Expr::Lit(Lit::Bool(true), _));
                    match flag {
                        Some(f) if f.value.is_some() => {
                            self.out.push_str(&alloc::format!("--{fname} "));
                            self.b_arg(v, arg_tail, f.value, None);
                        }
                        Some(f) if is_true => match f.short {
                            Some(c) => self.out.push_str(&alloc::format!("-{c}")),
                            None => self.out.push_str(&alloc::format!("--{fname}")),
                        },
                        None if is_true && sig.is_none() && fname.chars().count() == 1 => {
                            self.out.push_str(&alloc::format!("-{fname}"));
                        }
                        _ if is_true => self.out.push_str(&alloc::format!("--{fname}")),
                        _ => {
                            self.out.push_str(&alloc::format!("--{fname}="));
                            self.b_arg(v, arg_tail, None, None);
                        }
                    }
                }
            }
        }
    }

    /// Jeden argument prikazu.
    fn b_arg(&mut self, x: &Expr, tail: bool, kind: Option<Kind>, fields: Option<&'static [&'static str]>) {
        match x {
            Expr::Lit(Lit::Str(s), _) => {
                if self.safe_bare(s) {
                    self.out.push_str(s);
                } else {
                    self.out.push_str(&quote(s));
                }
            }
            Expr::Interp(parts, _) if self.bare_interp(parts) => {
                for p in parts {
                    match p {
                        Part::Text(t) => self.out.push_str(t),
                        Part::Expr(e) => {
                            self.out.push('{');
                            self.expr(e, 0, Ctx::B);
                            self.out.push('}');
                        }
                    }
                }
            }
            Expr::Lit(..) | Expr::Interp(..) | Expr::List(..) | Expr::Record(..) => self.expr(x, 0, Ctx::B),
            Expr::Glob(p, _) => {
                if safe_glob(p) {
                    self.out.push_str(p);
                } else {
                    self.out.push('(');
                    self.expr(x, 0, Ctx::B);
                    self.out.push(')');
                }
            }
            Expr::Var(v, _) if self.is_var(v) => {
                self.out.push('$');
                self.out.push_str(v);
            }
            Expr::Field(..) if var_path(x).is_some_and(|(v, _)| self.is_var(v)) => {
                let (v, path) = var_path(x).unwrap();
                self.out.push('$');
                self.out.push_str(v);
                for p in path.iter().rev() {
                    self.out.push('.');
                    self.out.push_str(p);
                }
            }
            Expr::Block(b, _) if !b.stmts.is_empty() => self.block(b, &[]),
            Expr::Closure(c, _) => {
                if !tail {
                    self.out.push('(');
                }
                if kind == Some(Kind::Cond) && c.params.len() == 1 && c.params[0] == ROW && row_printable(c, fields) {
                    self.push_scope();
                    self.declare(ROW);
                    let d = self.scope_depth() - 1;
                    self.row_depth.push(d);
                    self.expr(&c.body, 0, Ctx::Row);
                    self.row_depth.pop();
                    self.pop_scope();
                } else {
                    self.closure(c, Ctx::B);
                }
                if !tail {
                    self.out.push(')');
                }
            }
            _ => self.paren_chain(x),
        }
    }
}

/// `$x.a.b` -> ("x", ["b", "a"]); jen kdyz jsou vsechna pole jmena.
fn var_path(e: &Expr) -> Option<(&str, alloc::vec::Vec<&str>)> {
    let mut path = alloc::vec::Vec::new();
    let mut cur = e;
    loop {
        match cur {
            Expr::Field(base, name, _) => {
                if !super::expr::is_ident(name) {
                    return None;
                }
                path.push(name.as_str());
                cur = base;
            }
            Expr::Var(v, _) => return Some((v, path)),
            _ => return None,
        }
    }
}

/// Posledni cast roury `save cesta [--append]` -> presmerovani `>`/`>>`.
fn save_redirect(e: &Expr) -> Option<(&Expr, bool)> {
    let Expr::Call { name, args, .. } = e else { return None };
    if name != "save" {
        return None;
    }
    match args.as_slice() {
        [Arg::Pos(p)] => Some((p, false)),
        [Arg::Pos(p), Arg::Named(n, Expr::Lit(Lit::Bool(true), _))] if n == "append" => Some((p, true)),
        _ => None,
    }
}

/// Obsahuje vyraz na urovni `&&`/`||` prikaz (tisk by byl jako prikaz)?
fn has_command(e: &Expr) -> bool {
    match e {
        Expr::Binary(BinOp::And | BinOp::Or, l, r, _) => has_command(l) || has_command(r),
        Expr::Call { .. } | Expr::Pipe(..) | Expr::Bg(..) => true,
        _ => false,
    }
}

/// Jde podminku zapsat bez `it =>`? Ne, kdyz pouziva promennou, ktera se
/// jmenuje jako pole vstupu (parser by to odmitl jako nejednoznacne).
fn row_printable(c: &Closure, fields: Option<&'static [&'static str]>) -> bool {
    // `where (f => ...)` je sama closure; telo-closure jen v plnem tvaru.
    if matches!(c.body.as_ref(), Expr::Closure(..)) {
        return false;
    }
    let Some(fields) = fields else { return true };
    let mut ok = true;
    walk(&c.body, &mut |e| {
        if let Expr::Var(v, _) = e
            && v != ROW
            && fields.contains(&v.as_str())
        {
            ok = false;
        }
    });
    ok
}
