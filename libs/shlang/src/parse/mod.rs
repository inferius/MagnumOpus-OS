//! Parsery zapisu B (prikazovy) a C (vyrazovy). Oba vytvari stejny strom
//! ([`crate::ast`]).
//!
//! - Spolecna cast (vyrazy, prikazy `let`/`fn`/`if`/..., bloky, rozsah
//!   promennych) je v tomto modulu a v [`expr`].
//! - Prikazovy rezim zapisu B (hola slova, prepinace, `$x`, roura `|`,
//!   presmerovani, `&`) je v [`cmd`]. Zapis C ho nepouziva: prikaz se v nem
//!   vola jako funkce `ls("~")` a roura je retez `.where(...)`.
//!
//! Parser zna promenne v rozsahu (rozhodnuti 1A): hole slovo na zacatku
//! casti roury je prikaz, pokud to neni promenna ani klicove slovo. Podle
//! rozsahu se rozhoduje i v podmince `where` (pole radku vs promenna).

pub(crate) mod cmd;
mod expr;

pub use cmd::usage;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::ast::{Block, Expr, Needs, Script, Stmt};
use crate::error::{ShError, Warning, code};
use crate::lex::{Lexer, Tok};
use crate::sig::Sigs;
use crate::span::Span;

/// Zapis jazyka.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Prikazovy (vychozi): `ls ~/Dok -r | where size > 100MB`.
    B,
    /// Vyrazovy blizky Rustu: `ls("~/Dok", recursive: true).where(f => f.size > 100MB)`.
    C,
}

/// Nejnovejsi verze jazyka.
pub const LANG_LATEST: u32 = 1;

/// Nastaveni parseru.
pub struct Options<'a> {
    /// Vynuceny zapis; `None` = podle hlavicky `lang 1 c`, jinak B
    /// (ktery prijme i prikazy zapsane jako C, rozhodnuti 4A).
    pub style: Option<Style>,
    pub sigs: &'a dyn Sigs,
    /// Promenne, ktere uz existuji (interaktivni relace).
    pub vars: Vec<String>,
    /// Skript ze souboru (chybejici `lang` je varovani).
    pub script: bool,
}

/// Vysledek parsovani.
#[derive(Debug)]
pub struct Parsed {
    pub script: Script,
    /// Zapis, ve kterem byl zdroj (podle hlavicky nebo vynuceny).
    pub style: Style,
    pub warnings: Vec<Warning>,
}

/// Rozparsuje zdroj.
pub fn parse(src: &str, opts: &Options) -> Result<Parsed, ShError> {
    let mut p = Parser {
        lx: Lexer::new(src, 0),
        style: opts.style.unwrap_or(Style::B),
        sigs: opts.sigs,
        scopes: alloc::vec![opts.vars.clone()],
        row: Vec::new(),
        input_fields: None,
        warnings: Vec::new(),
        depth: 0,
        stop_else: false,
        stack_base: 0,
    };
    let mut script = Script::default();
    let mut comments = Vec::new();
    p.header(&mut script, opts, &mut comments)?;
    let mut body = p.stmts(true)?;
    if !comments.is_empty() {
        comments.append(&mut body.stmts);
        body.stmts = comments;
    }
    script.body = body;
    Ok(Parsed { script, style: p.style, warnings: p.warnings })
}

/// Kontext podminky `where` v zapisu B: hola jmena jsou pole radku.
pub(crate) struct Row {
    /// Zname pole vstupu (z podpisu predchoziho prikazu), pro kontrolu kolize.
    fields: Option<&'static [&'static str]>,
    /// Hloubka rozsahu, kde zacina podminka (promenne nad ni jsou "venku").
    scope_depth: usize,
}

pub(crate) struct Parser<'a> {
    pub(crate) lx: Lexer<'a>,
    pub(crate) style: Style,
    pub(crate) sigs: &'a dyn Sigs,
    scopes: Vec<Vec<String>>,
    pub(crate) row: Vec<Row>,
    /// Pole vystupu predchozi casti roury (pro `where`).
    pub(crate) input_fields: Option<&'static [&'static str]>,
    pub(crate) warnings: Vec<Warning>,
    depth: usize,
    /// Slovo `else` ukoncuje prikaz (telo `try cmd else ...`).
    pub(crate) stop_else: bool,
    stack_base: usize,
}

const MAX_DEPTH: usize = 128;

/// Kolik zasobniku smi parser spotrebovat (sluzba relaci ma 256 KB).
const PARSE_STACK: usize = 64 * 1024;



const KEYWORDS: &[&str] = &[
    "let", "fn", "if", "else", "for", "in", "while", "return", "break", "continue", "try", "bg", "async", "await",
    "true", "false",
    "null", "lang", "needs",
];

pub(crate) fn is_keyword(s: &str) -> bool {
    KEYWORDS.contains(&s)
}

impl<'a> Parser<'a> {
    // ---- pomocne ----

    pub(crate) fn err(&self, msg: impl Into<String>, span: Span) -> ShError {
        ShError::new(code::PARSE, msg).at(span)
    }

    pub(crate) fn here(&self) -> Span {
        Span::new(self.lx.pos, self.lx.pos)
    }

    pub(crate) fn enter(&mut self) -> Result<(), ShError> {
        self.depth += 1;
        let here = crate::stack::addr();
        if self.stack_base == 0 {
            self.stack_base = here;
        }
        if self.depth > MAX_DEPTH || self.stack_base.saturating_sub(here) > PARSE_STACK {
            return Err(ShError::new(code::DEPTH, "příliš hluboké vnoření").at(self.here()));
        }
        Ok(())
    }

    pub(crate) fn leave(&mut self) {
        self.depth -= 1;
    }

    /// Dalsi token po preskoceni mezer.
    /// Nahled je "mekky": znak, ktery lexer vyrazu nezna (napr. `~` ve
    /// slove prikazu), se jevi jako konec - chybu ohlasi az `next`.
    pub(crate) fn peek(&mut self) -> Result<Tok, ShError> {
        self.lx.skip_space();
        Ok(self.lx.peek().unwrap_or(Tok::Eof))
    }

    pub(crate) fn next(&mut self) -> Result<(Tok, Span), ShError> {
        self.lx.skip_space();
        self.lx.token()
    }

    pub(crate) fn is_punct(&mut self, p: &str) -> Result<bool, ShError> {
        Ok(matches!(self.peek()?, Tok::Punct(q) if q == p))
    }

    pub(crate) fn eat_punct(&mut self, p: &str) -> Result<bool, ShError> {
        if self.is_punct(p)? {
            self.next()?;
            return Ok(true);
        }
        Ok(false)
    }

    pub(crate) fn expect_punct(&mut self, p: &str) -> Result<Span, ShError> {
        let (t, span) = self.next()?;
        match t {
            Tok::Punct(q) if q == p => Ok(span),
            other => Err(self.err(alloc::format!("čekám '{p}', ale je tu {}", describe(&other)), span)),
        }
    }

    pub(crate) fn is_ident(&mut self, word: &str) -> Result<bool, ShError> {
        Ok(matches!(self.peek()?, Tok::Ident(w) if w == word))
    }

    pub(crate) fn expect_ident(&mut self) -> Result<(String, Span), ShError> {
        let (t, span) = self.next()?;
        match t {
            Tok::Ident(s) if !is_keyword(&s) => Ok((s, span)),
            other => Err(self.err(alloc::format!("čekám jméno, ale je tu {}", describe(&other)), span)),
        }
    }

    // ---- rozsah promennych ----

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

    /// Je promenna deklarovana uvnitr aktualni podminky `where` (parametr
    /// `it` nebo vnorena closure)?
    pub(crate) fn is_inner_var(&self, name: &str, from_depth: usize) -> bool {
        self.scopes[from_depth.min(self.scopes.len())..].iter().any(|s| s.iter().any(|v| v == name))
    }

    pub(crate) fn scope_depth(&self) -> usize {
        self.scopes.len()
    }

    // ---- hlavicka ----

    fn header(&mut self, script: &mut Script, opts: &Options, comments: &mut Vec<Stmt>) -> Result<(), ShError> {
        // Komentare pred hlavickou.
        loop {
            self.lx.skip_space();
            match self.lx.peek_char() {
                Some('\n') => {
                    self.lx.bump();
                }
                Some('#') => comments.push(self.comment()),
                _ => break,
            }
        }
        if self.is_ident("lang")? {
            let (_, span) = self.next()?;
            let (t, vspan) = self.next()?;
            let ver = match t {
                Tok::Num(crate::ast::Num::Int(v)) if v > 0 => v as u32,
                _ => return Err(self.err("za 'lang' patří číslo verze (lang 1)", vspan)),
            };
            if ver > LANG_LATEST {
                return Err(ShError::new(
                    code::LANG,
                    alloc::format!("verze jazyka {ver} není podporovaná (nejnovější je {LANG_LATEST})"),
                )
                .at(span.to(vspan)));
            }
            script.lang = Some(ver);
            if let Tok::Ident(s) = self.peek()? {
                let (_, sspan) = self.next()?;
                let style = match s.as_str() {
                    "b" => Style::B,
                    "c" => Style::C,
                    _ => return Err(self.err("zápis je 'b' nebo 'c' (lang 1 c)", sspan)),
                };
                if opts.style.is_none() {
                    self.style = style;
                }
            }
            self.end_of_stmt()?;
        } else if opts.script {
            self.warnings.push(Warning {
                message: alloc::format!(
                    "skript nemá hlavičku 'lang'; používám nejnovější verzi {LANG_LATEST} (doplňte 'lang {LANG_LATEST}' na začátek)"
                ),
                span: Span::new(0, 0),
            });
        }
        // needs { ... }
        loop {
            self.lx.skip_space();
            match self.lx.peek_char() {
                Some('\n') => {
                    self.lx.bump();
                }
                _ => break,
            }
        }
        if self.is_ident("needs")? {
            script.needs = Some(self.needs()?);
            self.end_of_stmt()?;
        }
        Ok(())
    }

    fn needs(&mut self) -> Result<Needs, ShError> {
        self.next()?;
        self.expect_punct("{")?;
        let mut needs = Needs::default();
        let mut net_seen = false;
        loop {
            self.lx.skip_space_nl();
            if self.eat_punct("}")? {
                break;
            }
            let (key, kspan) = self.expect_ident()?;
            self.expect_punct(":")?;
            match key.as_str() {
                "read" => needs.read.extend(self.string_list()?),
                "write" => needs.write.extend(self.string_list()?),
                "net" => {
                    net_seen = true;
                    if self.is_ident("none")? {
                        self.next()?;
                        needs.net = None;
                    } else {
                        needs.net = Some(self.string_list()?);
                    }
                }
                _ => {
                    return Err(self.err(alloc::format!("neznámý nárok '{key}' (read, write, net)"), kspan));
                }
            }
            self.lx.skip_space();
            self.eat_punct(",")?;
        }
        let _ = net_seen;
        Ok(needs)
    }

    fn string_list(&mut self) -> Result<Vec<String>, ShError> {
        self.expect_punct("[")?;
        let mut out = Vec::new();
        loop {
            self.lx.skip_space_nl();
            if self.eat_punct("]")? {
                break;
            }
            let (t, span) = self.next()?;
            match t {
                Tok::Str(parts) => match parts.as_slice() {
                    [crate::lex::RawPart::Text(s)] => out.push(s.clone()),
                    _ => return Err(self.err("v needs patří jen pevné cesty (bez {výrazu})", span)),
                },
                other => return Err(self.err(alloc::format!("čekám cestu v uvozovkách, je tu {}", describe(&other)), span)),
            }
            self.lx.skip_space_nl();
            if !self.eat_punct(",")? {
                self.lx.skip_space_nl();
                self.expect_punct("]")?;
                break;
            }
        }
        Ok(out)
    }

    // ---- prikazy (statements) ----

    fn comment(&mut self) -> Stmt {
        let start = self.lx.pos + 1;
        self.lx.skip_comment();
        Stmt::Comment(self.lx.src[start..self.lx.pos].trim_end().into())
    }

    /// Posloupnost prikazu az do `}` (nebo konce zdroje, kdyz `top`).
    pub(crate) fn stmts(&mut self, top: bool) -> Result<Block, ShError> {
        let mut block = Block::default();
        loop {
            self.lx.skip_space();
            match self.lx.peek_char() {
                None => {
                    if top {
                        return Ok(block);
                    }
                    return Err(self.err("chybí '}'", self.here()));
                }
                Some('\n' | ';') => {
                    self.lx.bump();
                    continue;
                }
                Some('#') => {
                    block.stmts.push(self.comment());
                    continue;
                }
                Some('}') if !top => return Ok(block),
                _ => {}
            }
            let stmt = self.stmt()?;
            block.stmts.push(stmt);
            // Komentar za prikazem na stejnem radku.
            self.lx.skip_space();
            if self.lx.peek_char() == Some('#') {
                block.stmts.push(self.comment());
            }
            self.end_of_stmt_in(top)?;
        }
    }

    fn end_of_stmt(&mut self) -> Result<(), ShError> {
        self.end_of_stmt_in(true)
    }

    fn end_of_stmt_in(&mut self, top: bool) -> Result<(), ShError> {
        self.lx.skip_space();
        match self.lx.peek_char() {
            None | Some('\n' | ';' | '#') => Ok(()),
            Some('}') if !top => Ok(()),
            Some(c) => {
                let hint = match c {
                    ')' => "chybí '(' k této závorce",
                    '}' => "chybí '{' k této závorce",
                    _ => "příkazy oddělte novým řádkem nebo ';'",
                };
                Err(self.err(alloc::format!("neočekávané '{c}' za koncem příkazu"), self.here()).hint(hint))
            }
        }
    }

    /// Blok `{ ... }` s vlastnim rozsahem; `params` se deklaruji uvnitr.
    pub(crate) fn block(&mut self, params: &[String]) -> Result<(Block, Span), ShError> {
        let start = self.expect_punct("{")?;
        self.enter()?;
        self.push_scope();
        for p in params {
            self.declare(p);
        }
        let block = self.stmts(false);
        self.pop_scope();
        self.leave();
        let block = block?;
        let end = self.expect_punct("}")?;
        Ok((block, start.to(end)))
    }

    fn stmt(&mut self) -> Result<Stmt, ShError> {
        let start = self.lx.pos;
        if let Tok::Ident(w) = self.peek()? {
            match w.as_str() {
                "let" => {
                    self.next()?;
                    self.lx.skip_space();
                    if self.lx.peek_char() == Some('(') {
                        // `let (a, b) = seznam`
                        self.lx.bump();
                        let mut names = Vec::new();
                        loop {
                            self.lx.skip_space();
                            let (n, _) = self.expect_ident()?;
                            names.push(n);
                            self.lx.skip_space();
                            if self.lx.peek_char() == Some(',') {
                                self.lx.bump();
                                continue;
                            }
                            self.expect_punct(")")?;
                            break;
                        }
                        self.lx.skip_space();
                        self.expect_punct("=")?;
                        self.lx.skip_space_nl();
                        let value = self.chain()?;
                        for n in &names {
                            self.declare(n);
                        }
                        return Ok(Stmt::LetList { names, value, span: Span::new(start, self.lx.pos) });
                    }
                    let (name, _) = self.expect_ident()?;
                    self.expect_punct("=")?;
                    self.lx.skip_space_nl();
                    let value = self.chain()?;
                    self.declare(&name);
                    return Ok(Stmt::Let { name, value, span: Span::new(start, self.lx.pos) });
                }
                "fn" => return self.fn_decl(start),
                "return" => {
                    self.next()?;
                    self.lx.skip_space();
                    let value = match self.lx.peek_char() {
                        None | Some('\n' | ';' | '}' | '#') => None,
                        _ => Some(self.chain()?),
                    };
                    return Ok(Stmt::Return { value, span: Span::new(start, self.lx.pos) });
                }
                "break" => {
                    self.next()?;
                    return Ok(Stmt::Break { span: Span::new(start, self.lx.pos) });
                }
                "continue" => {
                    self.next()?;
                    return Ok(Stmt::Continue { span: Span::new(start, self.lx.pos) });
                }
                "lang" | "needs" => {
                    return Err(self.err(alloc::format!("'{w}' patří jen na začátek skriptu"), self.here()));
                }
                _ if !is_keyword(&w) => {
                    // Prirazeni `x = ...`
                    let save = self.lx.pos;
                    self.next()?;
                    self.lx.skip_space();
                    let r = self.lx.rest();
                    let known_cmd = self.sigs.get(&w).is_some();
                    if r.starts_with('=') && !r.starts_with("==") && !r.starts_with("=>") && (self.is_var(&w) || !known_cmd) {
                        if !self.is_var(&w) {
                            return Err(ShError::new(code::UNKNOWN, alloc::format!("proměnná '{w}' neexistuje"))
                                .at(Span::new(start, start + w.len()))
                                .hint(alloc::format!("novou proměnnou zaveďte: let {w} = ...")));
                        }
                        self.next()?;
                        self.lx.skip_space_nl();
                        let value = self.chain()?;
                        return Ok(Stmt::Assign { name: w, value, span: Span::new(start, self.lx.pos) });
                    }
                    self.lx.pos = save;
                }
                _ => {}
            }
        }
        Ok(Stmt::Expr(self.chain()?))
    }

    fn fn_decl(&mut self, start: usize) -> Result<Stmt, ShError> {
        self.next()?;
        let (name, nspan) = self.expect_ident()?;
        if self.sigs.get(&name.replace('_', "-")).is_some() {
            return Err(self.err(alloc::format!("'{name}' je vestavěný příkaz, zvolte jiné jméno"), nspan));
        }
        self.expect_punct("(")?;
        let mut params = Vec::new();
        loop {
            self.lx.skip_space_nl();
            if self.eat_punct(")")? {
                break;
            }
            let (p, _) = self.expect_ident()?;
            params.push(p);
            self.lx.skip_space_nl();
            if !self.eat_punct(",")? {
                self.lx.skip_space_nl();
                self.expect_punct(")")?;
                break;
            }
        }
        let mut inner = params.clone();
        inner.push("input".into());
        self.lx.skip_space();
        let (body, _) = self.block(&inner)?;
        Ok(Stmt::Fn { name, params, body, span: Span::new(start, self.lx.pos) })
    }

    /// Prikaz nebo vyraz s `&&`/`||` (v B roury, v C vyraz).
    pub(crate) fn chain(&mut self) -> Result<Expr, ShError> {
        match self.style {
            Style::C => self.expr(),
            Style::B => self.or_chain(),
        }
    }
}

pub(crate) fn describe(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => alloc::format!("'{s}'"),
        Tok::Num(_) | Tok::Unit(..) => "číslo".into(),
        Tok::Str(_) => "řetězec".into(),
        Tok::Punct(p) => alloc::format!("'{p}'"),
        Tok::Newline => "konec řádku".into(),
        Tok::Eof => "konec vstupu".into(),
    }
}
