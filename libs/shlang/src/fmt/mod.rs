//! Formatovac: strom -> text v zapisu B nebo C (`fmt --style b|c`).
//!
//! Vystup musi jit znovu rozparsovat na **stejny strom** (test prevodu
//! `B -> strom -> C -> strom`). Proto formatovac sleduje rozsah promennych
//! stejne jako parser (pole radku v `where`, `$x` jen u promennych).
//! Kdyz konstrukce nejde zapsat v prikazovem tvaru B, pouzije se tvar
//! volani `jmeno(...)`, ktery B prijima take (pravidlo 5: prikaz uvnitr
//! vyrazu se vola jako funkce).

mod b;
mod expr;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::ast::{Block, Needs, Script, Stmt};
use crate::parse::Style;
use crate::sig::Sigs;

/// Nastaveni formatovace.
pub struct FmtOptions<'a> {
    pub style: Style,
    pub sigs: &'a dyn Sigs,
    /// Promenne, ktere uz existuji (interaktivni relace).
    pub vars: Vec<String>,
}

/// Desetinne cislo tak, jak ho prijme parser (vzdy s teckou).
pub fn num_text(f: f64) -> String {
    expr::num(crate::ast::Num::Float(f))
}

/// Naformatuje skript.
pub fn format(script: &Script, opts: &FmtOptions) -> String {
    let mut p = Printer {
        out: String::new(),
        style: opts.style,
        sigs: opts.sigs,
        indent: 0,
        scopes: alloc::vec![opts.vars.clone()],
        row_depth: Vec::new(),
    };
    if let Some(v) = script.lang {
        p.out.push_str(&alloc::format!("lang {v}"));
        if opts.style == Style::C {
            p.out.push_str(" c");
        }
        p.out.push('\n');
    }
    if let Some(n) = &script.needs {
        p.needs(n);
    }
    p.stmts(&script.body);
    p.out
}

/// Naformatuje jeden prikaz/vyraz (bez konce radku) - pro dokumentaci
/// a historii.
pub fn format_line(script: &Script, opts: &FmtOptions) -> String {
    let mut s = format(script, opts);
    while s.ends_with('\n') {
        s.pop();
    }
    s
}

pub(crate) struct Printer<'a> {
    pub(crate) out: String,
    pub(crate) style: Style,
    pub(crate) sigs: &'a dyn Sigs,
    indent: usize,
    scopes: Vec<Vec<String>>,
    /// Hloubky rozsahu, kde zacinaji podminky `where` (pro B).
    pub(crate) row_depth: Vec<usize>,
}

impl<'a> Printer<'a> {
    pub(crate) fn push_scope(&mut self) {
        self.scopes.push(Vec::new());
    }

    pub(crate) fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    pub(crate) fn declare(&mut self, name: &str) {
        if let Some(s) = self.scopes.last_mut() {
            s.push(name.to_string());
        }
    }

    pub(crate) fn is_var(&self, name: &str) -> bool {
        self.scopes.iter().any(|s| s.iter().any(|v| v == name))
    }

    pub(crate) fn scope_depth(&self) -> usize {
        self.scopes.len()
    }

    fn newline(&mut self) {
        self.out.push('\n');
        for _ in 0..self.indent {
            self.out.push_str("    ");
        }
    }

    fn needs(&mut self, n: &Needs) {
        self.out.push_str("needs {\n");
        let list = |v: &[String]| {
            let items: Vec<String> = v.iter().map(|s| expr::quote(s)).collect();
            alloc::format!("[{}]", items.join(", "))
        };
        if !n.read.is_empty() {
            self.out.push_str(&alloc::format!("    read: {}\n", list(&n.read)));
        }
        if !n.write.is_empty() {
            self.out.push_str(&alloc::format!("    write: {}\n", list(&n.write)));
        }
        match &n.net {
            None => self.out.push_str("    net: none\n"),
            Some(v) => self.out.push_str(&alloc::format!("    net: {}\n", list(v))),
        }
        self.out.push_str("}\n");
    }

    /// Prikazy na nejvyssi urovni (kazdy na svem radku).
    fn stmts(&mut self, b: &Block) {
        for s in &b.stmts {
            self.stmt(s);
            self.out.push('\n');
        }
    }

    /// Blok `{ ... }` s vlastnim rozsahem.
    pub(crate) fn block(&mut self, b: &Block, params: &[String]) {
        self.out.push('{');
        self.push_scope();
        for p in params {
            self.declare(p);
        }
        self.indent += 1;
        for s in &b.stmts {
            self.newline();
            self.stmt(s);
        }
        self.indent -= 1;
        self.pop_scope();
        self.newline();
        self.out.push('}');
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { name, value, .. } => {
                self.out.push_str(&alloc::format!("let {name} = "));
                self.chain(value);
                self.declare(name);
            }
            Stmt::Assign { name, value, .. } => {
                self.out.push_str(&alloc::format!("{name} = "));
                self.chain(value);
            }
            Stmt::LetList { names, value, .. } => {
                self.out.push_str(&alloc::format!("let ({}) = ", names.join(", ")));
                self.chain(value);
                for n in names {
                    self.declare(n);
                }
            }
            Stmt::Fn { name, params, body, .. } => {
                self.out.push_str(&alloc::format!("fn {name}({}) ", params.join(", ")));
                let mut inner = params.clone();
                inner.push("input".into());
                self.block(body, &inner);
            }
            Stmt::Return { value, .. } => {
                self.out.push_str("return");
                if let Some(v) = value {
                    self.out.push(' ');
                    self.chain(v);
                }
            }
            Stmt::Break { .. } => self.out.push_str("break"),
            Stmt::Continue { .. } => self.out.push_str("continue"),
            Stmt::Comment(c) => {
                self.out.push('#');
                self.out.push_str(c);
            }
            Stmt::Expr(e) => self.chain(e),
        }
    }

    /// Prikaz na pozici celeho prikazu (`let x = ...`, radek skriptu).
    pub(crate) fn chain(&mut self, e: &crate::ast::Expr) {
        match self.style {
            Style::C => self.expr_c(e, 0),
            Style::B => self.b_chain(e),
        }
    }
}
