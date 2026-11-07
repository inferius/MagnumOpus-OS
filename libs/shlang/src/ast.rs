//! Spolecny strom obou zapisu (B i C).
//!
//! Oba parsery vytvari tento strom a vse ostatni (formatovac, interpret,
//! doplnovani, kontrola) pracuje jen s nim. Strom nezna, ze kterého zapisu
//! vznikl: napr. `ls ~/Dok -r` (B) a `ls("~/Dok", recursive: true)` (C) dají
//! stejny strom. Vycet konstrukci a jejich tvar v obou zapisech je
//! v `docs/cs/navrh-shell.md`, kapitola Gramatika.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::span::Span;

/// Cely skript nebo jeden interaktivni radek.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Script {
    /// Verze jazyka z hlavicky `lang N` (`None` = hlavicka chybi).
    pub lang: Option<u32>,
    /// Manifest opravneni `needs { ... }`.
    pub needs: Option<Needs>,
    pub body: Block,
}

/// Manifest opravneni skriptu.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Needs {
    /// Cesty, ktere smi skript cist.
    pub read: Vec<String>,
    /// Cesty, kam smi zapisovat (zapis zahrnuje cteni).
    pub write: Vec<String>,
    /// Sit: `None` = `net: none`, jinak seznam povolenych cilu.
    pub net: Option<Vec<String>>,
}

/// Posloupnost prikazu.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Block {
    pub stmts: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// `let jmeno = hodnota`
    Let { name: String, value: Expr, span: Span },
    /// `jmeno = hodnota` (promenna uz existuje)
    Assign { name: String, value: Expr, span: Span },
    /// `fn jmeno(a, b) { ... }`
    Fn { name: String, params: Vec<String>, body: Block, span: Span },
    /// `return [hodnota]`
    Return { value: Option<Expr>, span: Span },
    Break { span: Span },
    Continue { span: Span },
    /// Komentar `# ...` na samostatnem radku (formatovac ho zachova).
    Comment(String),
    /// Vyraz nebo prikaz (roura, volani, ...).
    Expr(Expr),
}

/// Druh cisla s jednotkou.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    // velikosti (desitkove a dvojkove)
    B,
    KB,
    MB,
    GB,
    TB,
    KiB,
    MiB,
    GiB,
    TiB,
    // doby
    Ns,
    Us,
    Ms,
    S,
    Min,
    H,
    D,
    W,
    Y,
}

impl Unit {
    pub const ALL: &'static [(Unit, &'static str)] = &[
        (Unit::KiB, "KiB"),
        (Unit::MiB, "MiB"),
        (Unit::GiB, "GiB"),
        (Unit::TiB, "TiB"),
        (Unit::KB, "KB"),
        (Unit::MB, "MB"),
        (Unit::GB, "GB"),
        (Unit::TB, "TB"),
        (Unit::B, "B"),
        (Unit::Min, "min"),
        (Unit::Ns, "ns"),
        (Unit::Us, "us"),
        (Unit::Ms, "ms"),
        (Unit::S, "s"),
        (Unit::H, "h"),
        (Unit::D, "d"),
        (Unit::W, "w"),
        (Unit::Y, "y"),
    ];

    pub fn name(self) -> &'static str {
        Self::ALL.iter().find(|(u, _)| *u == self).map(|(_, n)| *n).unwrap_or("?")
    }

    pub fn parse(s: &str) -> Option<Unit> {
        Self::ALL.iter().find(|(_, n)| *n == s).map(|(u, _)| *u)
    }

    /// Je to velikost (bajty)? Jinak doba.
    pub fn is_size(self) -> bool {
        matches!(
            self,
            Unit::B | Unit::KB | Unit::MB | Unit::GB | Unit::TB | Unit::KiB | Unit::MiB | Unit::GiB | Unit::TiB
        )
    }

    /// Nasobek zakladni jednotky (bajt, nanosekunda).
    pub fn factor(self) -> u128 {
        const S: u128 = 1_000_000_000;
        match self {
            Unit::B => 1,
            Unit::KB => 1000,
            Unit::MB => 1000_000,
            Unit::GB => 1000_000_000,
            Unit::TB => 1000_000_000_000,
            Unit::KiB => 1 << 10,
            Unit::MiB => 1 << 20,
            Unit::GiB => 1 << 30,
            Unit::TiB => 1 << 40,
            Unit::Ns => 1,
            Unit::Us => 1000,
            Unit::Ms => 1000_000,
            Unit::S => S,
            Unit::Min => 60 * S,
            Unit::H => 3600 * S,
            Unit::D => 86400 * S,
            Unit::W => 7 * 86400 * S,
            Unit::Y => 365 * 86400 * S,
        }
    }
}

/// Literal.
#[derive(Debug, Clone, PartialEq)]
pub enum Lit {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// Cislo s jednotkou (`100MB`, `1.5h`); `value` je cele nebo desetinne.
    Unit { value: Num, unit: Unit },
    Str(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Num {
    Int(i64),
    Float(f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    /// `&&`: levá strana chyba -> chyba se siri; `false` -> `false`; jinak pravá.
    And,
    /// `||`: levá strana chyba nebo `false` -> prava strana; jinak leva.
    Or,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "&&",
            BinOp::Or => "||",
        }
    }

    /// Priorita (vyssi = vaze tesneji).
    pub fn prec(self) -> u8 {
        match self {
            BinOp::Or => 1,
            BinOp::And => 2,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 3,
            BinOp::Add | BinOp::Sub => 4,
            BinOp::Mul | BinOp::Div | BinOp::Rem => 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Not,
    Neg,
}

/// Cast retezce s vlozenymi vyrazy `"a {x} b"`.
#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Text(String),
    Expr(Expr),
}

/// Argument volani prikazu nebo funkce.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    /// Pozicni (`ls ~/Dok`, `ls("~/Dok")`).
    Pos(Expr),
    /// Pojmenovany (`-r`, `--width 1600`, `recursive: true`). Jmeno je vzdy
    /// plne (dlouhe), kratky prepinac prevede parser podle podpisu.
    Named(String, Expr),
}

/// Telo closure: vyraz nebo blok.
#[derive(Debug, Clone, PartialEq)]
pub struct Closure {
    pub params: Vec<String>,
    pub body: Box<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Lit(Lit, Span),
    /// Retezec s vlozenymi vyrazy.
    Interp(Vec<Part>, Span),
    /// Vzor souboru (`*.txt` v B, `glob("*.txt")` v C).
    Glob(String, Span),
    Var(String, Span),
    /// `a.b`
    Field(Box<Expr>, String, Span),
    /// `a[i]`
    Index(Box<Expr>, Box<Expr>, Span),
    List(Vec<Expr>, Span),
    Record(Vec<(String, Expr)>, Span),
    Unary(UnOp, Box<Expr>, Span),
    Binary(BinOp, Box<Expr>, Box<Expr>, Span),
    Closure(Closure, Span),
    /// Blok jako telo closure, `try` nebo `bg`.
    Block(Block, Span),
    If { cond: Box<Expr>, then: Block, els: Option<Box<Expr>>, span: Span },
    For { var: String, iter: Box<Expr>, body: Block, span: Span },
    While { cond: Box<Expr>, body: Block, span: Span },
    /// `try { } else e => ...`, `try cmd` (bez `else` vrati vysledek jako
    /// zaznam `{ok, failed, value, error}`).
    Try { body: Block, handler: Option<Closure>, span: Span },
    /// Volani prikazu nebo funkce. Jmeno je tak, jak je napsane (alias se
    /// rozlisi az pri vyhodnoceni), vzdy s pomlckou (`file-list`).
    Call { name: String, args: Vec<Arg>, span: Span },
    /// Roura: hodnota prvni casti jde jako vstup do dalsich (ty jsou `Call`).
    Pipe(Vec<Expr>, Span),
    /// Uloha na pozadi (`cmd &` v B, `bg { }` v obou).
    Bg(Block, Span),
    /// `x?` - vysledek z `try` rozbali, chybu vyvola.
    Unwrap(Box<Expr>, Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Lit(_, s)
            | Expr::Interp(_, s)
            | Expr::Glob(_, s)
            | Expr::Var(_, s)
            | Expr::Field(_, _, s)
            | Expr::Index(_, _, s)
            | Expr::List(_, s)
            | Expr::Record(_, s)
            | Expr::Unary(_, _, s)
            | Expr::Binary(_, _, _, s)
            | Expr::Closure(_, s)
            | Expr::Block(_, s)
            | Expr::Pipe(_, s)
            | Expr::Bg(_, s)
            | Expr::Unwrap(_, s) => *s,
            Expr::If { span, .. }
            | Expr::For { span, .. }
            | Expr::While { span, .. }
            | Expr::Try { span, .. }
            | Expr::Call { span, .. } => *span,
        }
    }

    pub fn str(s: &str) -> Expr {
        Expr::Lit(Lit::Str(s.into()), Span::default())
    }
}

/// Jmeno implicitniho parametru podminky `where` v zapisu B.
pub const ROW: &str = "it";

/// Projde vyraz a vsechny vnorene vyrazy (vcetne bloku), predek pred
/// potomky.
pub fn walk(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    f(e);
    match e {
        Expr::Lit(..) | Expr::Glob(..) | Expr::Var(..) => {}
        Expr::Interp(parts, _) => {
            for p in parts {
                if let Part::Expr(x) = p {
                    walk(x, f);
                }
            }
        }
        Expr::Field(b, _, _) | Expr::Unary(_, b, _) | Expr::Unwrap(b, _) => walk(b, f),
        Expr::Index(a, b, _) | Expr::Binary(_, a, b, _) => {
            walk(a, f);
            walk(b, f);
        }
        Expr::List(items, _) | Expr::Pipe(items, _) => {
            for x in items {
                walk(x, f);
            }
        }
        Expr::Record(fields, _) => {
            for (_, x) in fields {
                walk(x, f);
            }
        }
        Expr::Closure(c, _) => walk(&c.body, f),
        Expr::Block(b, _) | Expr::Bg(b, _) => walk_block(b, f),
        Expr::If { cond, then, els, .. } => {
            walk(cond, f);
            walk_block(then, f);
            if let Some(e) = els {
                walk(e, f);
            }
        }
        Expr::For { iter, body, .. } => {
            walk(iter, f);
            walk_block(body, f);
        }
        Expr::While { cond, body, .. } => {
            walk(cond, f);
            walk_block(body, f);
        }
        Expr::Try { body, handler, .. } => {
            walk_block(body, f);
            if let Some(h) = handler {
                walk(&h.body, f);
            }
        }
        Expr::Call { args, .. } => {
            for a in args {
                match a {
                    Arg::Pos(x) | Arg::Named(_, x) => walk(x, f),
                }
            }
        }
    }
}

/// Projde vsechny vyrazy v bloku.
pub fn walk_block(b: &Block, f: &mut dyn FnMut(&Expr)) {
    for s in &b.stmts {
        match s {
            Stmt::Let { value, .. } | Stmt::Assign { value, .. } => walk(value, f),
            Stmt::Fn { body, .. } => walk_block(body, f),
            Stmt::Return { value: Some(v), .. } => walk(v, f),
            Stmt::Expr(e) => walk(e, f),
            _ => {}
        }
    }
}
