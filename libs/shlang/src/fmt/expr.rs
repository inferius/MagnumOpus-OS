//! Tisk vyrazu: zapis C a vyrazovy rezim zapisu B (vcetne podminky
//! `where`, kde jsou hola jmena pole radku).

use alloc::string::String;

use super::Printer;
use crate::ast::{Arg, Closure, Expr, Lit, Num, Part, ROW};
use crate::lex::{is_ident_char, is_ident_start};
use crate::parse::{Style, is_keyword};

/// Kontext tisku vyrazu.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ctx {
    /// Zapis C.
    C,
    /// Vyraz v zapisu B (zavorka obsahuje rouru).
    B,
    /// Podminka `where` v zapisu B (hola jmena = pole radku `it`).
    Row,
}

/// Retezec v uvozovkach s escape sekvencemi.
pub fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    escape_into(&mut out, s);
    out.push('"');
    out
}

fn escape_into(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            _ => out.push(c),
        }
    }
}

pub(crate) fn is_ident(s: &str) -> bool {
    let mut it = s.chars();
    it.next().is_some_and(is_ident_start) && it.all(is_ident_char) && !is_keyword(s)
}

pub(crate) fn num(n: Num) -> String {
    match n {
        Num::Int(i) => alloc::format!("{i}"),
        Num::Float(f) => {
            let mut s = alloc::format!("{f}");
            if !s.contains(['.', 'e', 'N', 'i']) {
                s.push_str(".0");
            }
            s
        }
    }
}

pub(crate) fn lit(l: &Lit) -> String {
    match l {
        Lit::Null => "null".into(),
        Lit::Bool(b) => alloc::format!("{b}"),
        Lit::Int(i) => alloc::format!("{i}"),
        Lit::Float(f) => num(Num::Float(*f)),
        Lit::Unit { value, unit } => alloc::format!("{}{}", num(*value), unit.name()),
        Lit::Str(s) => quote(s),
    }
}

/// Jmeno prikazu ve tvaru volani (`file-list` -> `file_list`).
pub(crate) fn call_name(name: &str) -> String {
    name.replace('-', "_")
}

/// Je literal zaporne cislo (jako zaklad `.pole` potrebuje zavorku)?
fn negative(l: &Lit) -> bool {
    match l {
        Lit::Int(i) => *i < 0,
        Lit::Float(f) => *f < 0.0 || f.is_sign_negative(),
        Lit::Unit { value: Num::Int(i), .. } => *i < 0,
        Lit::Unit { value: Num::Float(f), .. } => f.is_sign_negative(),
        _ => false,
    }
}

impl<'a> Printer<'a> {
    pub(crate) fn expr_c(&mut self, e: &Expr, min: u8) {
        self.expr(e, min, Ctx::C);
    }

    /// Muze byt `e` zakladem `.pole` / `[i]` / `?` bez zavorek?
    fn atomic(&self, e: &Expr) -> bool {
        match e {
            Expr::Lit(l, _) => !negative(l),
            Expr::Interp(..)
            | Expr::Glob(..)
            | Expr::Var(..)
            | Expr::Field(..)
            | Expr::Index(..)
            | Expr::List(..)
            | Expr::Record(..)
            | Expr::Unwrap(..)
            | Expr::Call { .. }
            // V B se roura tiskne v zavorce, v C jako retez - obojí je atom.
            | Expr::Pipe(..) => true,
            _ => false,
        }
    }

    /// `await`: v C jedna uloha jako `x.await`, jinak klicova slova (stejne
    /// v B i C).
    fn await_expr(&mut self, kind: crate::ast::AwaitKind, target: &Expr, timeout: Option<&Expr>, ctx: Ctx) {
        use crate::ast::AwaitKind;
        if kind == AwaitKind::One && timeout.is_none() && self.style == crate::parse::Style::C && self.atomic(target) {
            self.base(target, ctx);
            self.out.push_str(".await");
            return;
        }
        self.out.push_str(match kind {
            AwaitKind::One => "await ",
            AwaitKind::All => "await all ",
            AwaitKind::Any => "await any ",
        });
        self.base(target, ctx);
        if let Some(t) = timeout {
            self.out.push_str(" timeout ");
            self.base(t, ctx);
        }
    }

    fn base(&mut self, e: &Expr, ctx: Ctx) {
        if self.atomic(e) {
            self.expr(e, 7, ctx);
        } else {
            self.out.push('(');
            self.expr(e, 0, ctx);
            self.out.push(')');
        }
    }

    pub(crate) fn expr(&mut self, e: &Expr, min: u8, ctx: Ctx) {
        match e {
            Expr::Lit(l, _) => self.out.push_str(&lit(l)),
            Expr::Interp(parts, _) => {
                self.out.push('"');
                for p in parts {
                    match p {
                        Part::Text(t) => escape_into(&mut self.out, t),
                        Part::Expr(x) => {
                            self.out.push('{');
                            self.expr(x, 0, ctx);
                            self.out.push('}');
                        }
                    }
                }
                self.out.push('"');
            }
            Expr::Glob(p, _) => {
                self.out.push_str("glob(");
                self.out.push_str(&quote(p));
                self.out.push(')');
            }
            Expr::Var(v, _) => self.out.push_str(v),
            Expr::Field(base, name, _) => {
                if ctx == Ctx::Row
                    && let Expr::Var(v, _) = base.as_ref()
                    && v == ROW
                    && self.row_field_bare(name)
                {
                    self.out.push_str(name);
                    return;
                }
                self.base(base, ctx);
                self.out.push('.');
                self.out.push_str(name);
            }
            Expr::Index(base, idx, _) => {
                self.base(base, ctx);
                self.out.push('[');
                self.expr(idx, 0, ctx);
                self.out.push(']');
            }
            Expr::Unwrap(base, _) => {
                self.base(base, ctx);
                self.out.push('?');
            }
            Expr::Await { kind, target, timeout, .. } => self.await_expr(*kind, target, timeout.as_deref(), ctx),
            Expr::Async(b, _) => {
                self.out.push_str("async ");
                self.block(b, &[]);
            }
            Expr::List(items, _) => {
                self.out.push('[');
                for (i, x) in items.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.expr(x, 0, ctx);
                }
                self.out.push(']');
            }
            Expr::Record(fields, _) => {
                self.out.push('{');
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    if is_ident(k) {
                        self.out.push_str(k);
                    } else {
                        self.out.push_str(&quote(k));
                    }
                    self.out.push_str(": ");
                    self.expr(v, 0, ctx);
                }
                self.out.push('}');
            }
            Expr::Unary(op, x, _) => {
                self.out.push_str(match op {
                    crate::ast::UnOp::Not => "!",
                    crate::ast::UnOp::Neg => "-",
                });
                // `- -5` by se spojilo v literal; zavorka to zachova.
                if matches!(x.as_ref(), Expr::Lit(l, _) if negative(l) || matches!(l, Lit::Int(_) | Lit::Float(_) | Lit::Unit { .. }))
                {
                    self.out.push('(');
                    self.expr(x, 0, ctx);
                    self.out.push(')');
                } else {
                    self.expr(x, 6, ctx);
                }
            }
            Expr::Binary(op, l, r, _) => {
                let p = op.prec();
                let paren = p < min;
                if paren {
                    self.out.push('(');
                }
                self.expr(l, p, ctx);
                self.out.push(' ');
                self.out.push_str(op.symbol());
                self.out.push(' ');
                self.expr(r, p + 1, ctx);
                if paren {
                    self.out.push(')');
                }
            }
            Expr::Closure(c, _) => {
                let paren = min > 0 || (ctx != Ctx::C && self.closure_needs_paren(c));
                if paren {
                    self.out.push('(');
                }
                self.closure(c, ctx);
                if paren {
                    self.out.push(')');
                }
            }
            Expr::Block(b, _) => self.block(b, &[]),
            Expr::If { .. } | Expr::For { .. } | Expr::While { .. } | Expr::Try { .. } | Expr::Bg(..) => {
                let paren = min > 0;
                if paren {
                    self.out.push('(');
                }
                self.control(e, ctx);
                if paren {
                    self.out.push(')');
                }
            }
            Expr::Call { name, args, .. } => match ctx {
                Ctx::C | Ctx::Row => self.call_form(name, args, ctx),
                Ctx::B => {
                    if args.is_empty() {
                        self.call_form(name, args, ctx);
                    } else {
                        self.out.push('(');
                        self.b_command(e, true);
                        self.out.push(')');
                    }
                }
            },
            Expr::Pipe(stages, _) => match ctx {
                Ctx::B => {
                    self.out.push('(');
                    self.b_pipeline(e, true);
                    self.out.push(')');
                }
                Ctx::C | Ctx::Row => {
                    self.base(&stages[0], ctx);
                    for s in &stages[1..] {
                        self.out.push('.');
                        match s {
                            Expr::Call { name, args, .. } => self.call_form(name, args, ctx),
                            other => self.expr(other, 7, ctx),
                        }
                    }
                }
            },
        }
    }

    /// Smi byt pole radku v podmince zapsane holym jmenem?
    fn row_field_bare(&self, name: &str) -> bool {
        is_ident(name) && !self.is_var(name)
    }

    /// Volani ve tvaru `jmeno(a, b, klic: c)`.
    pub(crate) fn call_form(&mut self, name: &str, args: &[Arg], ctx: Ctx) {
        // Jmeno, ktere jako identifikator nejde (cesta, `-` bez podpisu,
        // `_` u prikazu s pomlckou), se zapise pres exec("jmeno", ...).
        let cname = call_name(name);
        let direct = is_ident(&cname)
            && cname != "exec"
            && cname != "glob"
            && (!name.contains('-') || self.sigs.get(name).is_some())
            && (!name.contains('_') || self.sigs.get(&name.replace('_', "-")).is_none());
        let mut first = true;
        if direct {
            self.out.push_str(&cname);
            self.out.push('(');
        } else {
            self.out.push_str("exec(");
            self.out.push_str(&quote(name));
            first = false;
        }
        for (i, a) in args.iter().enumerate() {
            if i > 0 || !first {
                self.out.push_str(", ");
            }
            match a {
                Arg::Pos(x) => self.expr(x, 0, ctx),
                Arg::Named(n, x) => {
                    self.out.push_str(n);
                    self.out.push_str(": ");
                    self.expr(x, 0, ctx);
                }
            }
        }
        self.out.push(')');
    }

    /// Potrebuje closure ve vyrazu B zavorku (telo je prikaz)?
    fn closure_needs_paren(&self, c: &Closure) -> bool {
        matches!(c.body.as_ref(), Expr::Call { .. } | Expr::Pipe(..))
    }

    pub(crate) fn closure(&mut self, c: &Closure, ctx: Ctx) {
        match c.params.len() {
            1 => self.out.push_str(&c.params[0]),
            _ => {
                self.out.push('(');
                self.out.push_str(&c.params.join(", "));
                self.out.push(')');
            }
        }
        self.out.push_str(" => ");
        self.closure_body(&c.body, &c.params, ctx);
    }

    /// Telo closure (v B jako cast roury, v C jako vyraz).
    pub(crate) fn closure_body(&mut self, body: &Expr, params: &[String], ctx: Ctx) {
        if let Expr::Block(b, _) = body {
            self.block(b, params);
            return;
        }
        self.push_scope();
        for p in params {
            self.declare(p);
        }
        // Telo uz neni podminka `where`.
        let rows = core::mem::take(&mut self.row_depth);
        match ctx {
            Ctx::C => self.expr(body, 0, Ctx::C),
            Ctx::B | Ctx::Row => self.b_stage_body(body),
        }
        self.row_depth = rows;
        self.pop_scope();
    }

    /// `if`, `for`, `while`, `try`, `bg`.
    fn control(&mut self, e: &Expr, ctx: Ctx) {
        // Vnitrek bloku se tiskne v zapisu souboru; podminky jako vyraz.
        let cctx = if self.style == Style::C { Ctx::C } else { Ctx::B };
        match e {
            Expr::If { cond, then, els, .. } => {
                self.out.push_str("if ");
                self.expr(cond, 0, cctx);
                self.out.push(' ');
                self.block(then, &[]);
                if let Some(e) = els {
                    self.out.push_str(" else ");
                    match e.as_ref() {
                        Expr::Block(b, _) => self.block(b, &[]),
                        other => self.control(other, ctx),
                    }
                }
            }
            Expr::For { var, iter, body, .. } => {
                self.out.push_str(&alloc::format!("for {var} in "));
                self.expr(iter, 0, cctx);
                self.out.push(' ');
                self.block(body, core::slice::from_ref(var));
            }
            Expr::While { cond, body, .. } => {
                self.out.push_str("while ");
                self.expr(cond, 0, cctx);
                self.out.push(' ');
                self.block(body, &[]);
            }
            Expr::Try { body, handler, .. } => {
                self.out.push_str("try ");
                match body.stmts.as_slice() {
                    [crate::ast::Stmt::Expr(x)] if !matches!(x, Expr::Block(..)) => match cctx {
                        Ctx::C => self.expr(x, 0, Ctx::C),
                        _ => self.b_stage_body(x),
                    },
                    _ => self.block(body, &[]),
                }
                if let Some(h) = handler {
                    self.out.push_str(" else ");
                    if h.params.is_empty()
                        && let Expr::Block(b, _) = h.body.as_ref()
                    {
                        self.block(b, &[]);
                    } else {
                        self.closure(h, cctx);
                    }
                }
            }
            Expr::Bg(b, _) => {
                self.out.push_str("bg ");
                self.block(b, &[]);
            }
            Expr::Async(b, _) => {
                self.out.push_str("async ");
                self.block(b, &[]);
            }
            other => self.expr(other, 0, cctx),
        }
    }
}
