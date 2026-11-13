//! Prikazovy rezim zapisu B: `prikaz arg -p hodnota --dlouhy`, roura `|`,
//! `&&`/`||` mezi rourami, presmerovani `>`/`>>`/`<` a `&` (pozadi).
//!
//! Hole slovo je text (nebo cislo s jednotkou, `true`/`false`/`null`),
//! `$x` hodnota promenne, `*.txt` vzor souboru, `{x}` uvnitr slova vlozeny
//! vyraz, `( )` vyraz nebo vnorena roura (podle rozsahu, rozhodnuti 1A).
//! Podminka `where` je vyraz, ve kterem jsou hola jmena pole radku.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::{Parser, Row, is_keyword};
use crate::ast::{Arg, BinOp, Block, Closure, Expr, Lit, Part, ROW, Stmt};
use crate::error::{ShError, Warning, code};
use crate::lex::{Tok, is_ident_char, is_ident_start, word_number};
use crate::sig::{Kind, Sig};
use crate::span::Span;

fn is_name_char(c: char) -> bool {
    is_ident_char(c) || matches!(c, '-' | '.' | '/' | '~')
}

impl<'a> Parser<'a> {
    /// `a || b` nad rourami; na konci prikazu muze byt `&` (pozadi).
    pub(crate) fn or_chain(&mut self) -> Result<Expr, ShError> {
        let mut left = self.and_chain()?;
        loop {
            self.lx.skip_space();
            if self.lx.rest().starts_with("||") {
                self.lx.pos += 2;
                self.lx.skip_space_nl();
                let right = self.and_chain()?;
                let span = left.span().to(right.span());
                left = Expr::Binary(BinOp::Or, Box::new(left), Box::new(right), span);
            } else {
                break;
            }
        }
        self.lx.skip_space();
        if self.lx.rest().starts_with('&') && !self.lx.rest().starts_with("&&") {
            let end = Span::new(self.lx.pos, self.lx.pos + 1);
            self.lx.pos += 1;
            let span = left.span().to(end);
            left = Expr::Bg(Block { stmts: alloc::vec![Stmt::Expr(left)] }, span);
        }
        Ok(left)
    }

    fn and_chain(&mut self) -> Result<Expr, ShError> {
        let mut left = self.pipeline()?;
        loop {
            self.lx.skip_space();
            if self.lx.rest().starts_with("&&") {
                self.lx.pos += 2;
                self.lx.skip_space_nl();
                let right = self.pipeline()?;
                let span = left.span().to(right.span());
                left = Expr::Binary(BinOp::And, Box::new(left), Box::new(right), span);
            } else {
                break;
            }
        }
        Ok(left)
    }

    /// Roura `cast | prikaz | prikaz` s presmerovanim na konci.
    fn pipeline(&mut self) -> Result<Expr, ShError> {
        let first = self.b_stage(false)?;
        let mut fields = self.output_fields(&first);
        let mut stages = alloc::vec![first];
        let mut redirected = false;
        loop {
            self.lx.skip_space();
            let rest = self.lx.rest();
            if rest.starts_with('|') && !rest.starts_with("||") {
                if redirected {
                    return Err(self.err("za přesměrováním '>' už nemůže být '|'", self.here()));
                }
                self.lx.pos += 1;
                self.lx.skip_space_nl();
                self.input_fields = fields;
                let c = self.command()?;
                self.input_fields = None;
                fields = self.output_fields(&c);
                stages.push(c);
            } else if rest.starts_with('>') {
                let start = self.lx.pos;
                let append = rest.starts_with(">>");
                self.lx.pos += if append { 2 } else { 1 };
                self.lx.skip_space();
                let target = self.word_arg()?;
                let mut args = alloc::vec![Arg::Pos(target)];
                if append {
                    args.push(Arg::Named("append".into(), Expr::Lit(Lit::Bool(true), Span::default())));
                }
                stages.push(Expr::Call { name: "save".into(), args, span: Span::new(start, self.lx.pos) });
                redirected = true;
            } else if rest.starts_with('<') {
                let start = self.lx.pos;
                if stages.len() != 1 || !matches!(stages[0], Expr::Call { .. }) {
                    return Err(self.err("'<' patří jen za první příkaz roury (příkaz < soubor)", self.here()));
                }
                self.lx.pos += 1;
                self.lx.skip_space();
                let target = self.word_arg()?;
                let open = Expr::Call {
                    name: "open".into(),
                    args: alloc::vec![Arg::Pos(target)],
                    span: Span::new(start, self.lx.pos),
                };
                stages.insert(0, open);
            } else {
                break;
            }
        }
        if stages.len() == 1 {
            return Ok(stages.pop().unwrap());
        }
        // `(a | b) | c` je totez co `a | b | c` (jeden tvar stromu).
        if let Expr::Pipe(..) = stages[0]
            && let Expr::Pipe(mut inner, _) = stages.remove(0)
        {
            inner.append(&mut stages);
            stages = inner;
        }
        let span = stages[0].span().to(stages[stages.len() - 1].span());
        Ok(Expr::Pipe(stages, span))
    }

    fn output_fields(&self, e: &Expr) -> Option<&'static [&'static str]> {
        let Expr::Call { name, .. } = e else { return None };
        match self.sigs.get(name)?.output {
            crate::sig::Io::Records(f) if !f.is_empty() => Some(f),
            _ => None,
        }
    }

    /// Jedna cast roury podle pravidla rozsahu: prikaz (hole slovo, ktere
    /// neni promenna ani klicove slovo), jinak vyraz. `logic` = vyraz smi
    /// obsahovat `&&`/`||` (telo closure), jinak je resi `or_chain`.
    pub(crate) fn b_stage(&mut self, logic: bool) -> Result<Expr, ShError> {
        self.lx.skip_space();
        let c = self.lx.peek_char();
        if c.is_some_and(|c| is_ident_start(c) || c == '(')
            && let Some(cl) = self.try_closure()?
        {
            return Ok(cl);
        }
        match c {
            Some('.' | '/' | '~') => return self.command(),
            Some('$') => {
                let start = self.lx.pos;
                self.lx.bump();
                let name: String = self.lx.ident().into();
                return Err(self
                    .err("na začátku příkazu je výraz a ten se píše bez '$'", Span::new(start, self.lx.pos))
                    .hint(alloc::format!("napište '{name}'")));
            }
            Some(c) if is_ident_start(c) => {
                let mut l = self.lx.clone();
                let w = l.ident();
                let call = l.peek_char() == Some('(');
                if !(is_keyword(w) || call || self.is_var(w)) {
                    return self.command();
                }
            }
            _ => {}
        }
        if logic { self.expr_bp(0) } else { self.expr_bp(3) }
    }

    fn at_terminator(&self) -> bool {
        let rest = self.lx.rest();
        match rest.chars().next() {
            None | Some('\n' | ';' | '|' | ')' | '}' | ']' | '#' | '>' | '<' | '&') => true,
            Some(_) if self.stop_else && rest.starts_with("else") => {
                rest[4..].chars().next().is_none_or(|c| c.is_whitespace())
            }
            _ => false,
        }
    }

    fn at_arg_end(&self) -> bool {
        self.lx.peek_char().is_none_or(|c| c == ' ' || c == '\t' || c == '\r') || self.at_terminator()
    }

    fn check_arg_end(&self) -> Result<(), ShError> {
        // Nahled vyrazu uz mohl preskocit mezery za argumentem.
        let prev_space = self.lx.src[..self.lx.pos].ends_with([' ', '\t']);
        if prev_space || self.at_arg_end() {
            return Ok(());
        }
        Err(self
            .err("mezi argumenty chybí mezera", Span::new(self.lx.pos, self.lx.pos + 1))
            .hint("text se zvláštními znaky dejte do uvozovek"))
    }

    /// Prikaz s argumenty.
    pub(crate) fn command(&mut self) -> Result<Expr, ShError> {
        self.lx.skip_space();
        let start = self.lx.pos;
        match self.lx.peek_char() {
            Some(c) if is_ident_start(c) || matches!(c, '.' | '/' | '~') => {}
            _ => {
                let t = self.lx.peek().map(|t| super::describe(&t)).unwrap_or_default();
                return Err(self.err(alloc::format!("čekám příkaz, ale je tu {t}"), self.here()));
            }
        }
        while let Some(c) = self.lx.peek_char() {
            if !is_name_char(c) {
                break;
            }
            self.lx.bump();
        }
        let name: String = self.lx.src[start..self.lx.pos].into();
        let name_span = Span::new(start, self.lx.pos);
        if is_keyword(&name) {
            return Err(self.err(alloc::format!("'{name}' tu nemůže být"), name_span));
        }
        // Zapis C za rourou: `| where(it => ...)`.
        if self.lx.peek_char() == Some('(') {
            let name = super::expr::demangle(self, &name);
            let args = self.call_args()?;
            let span = name_span.to(self.here());
            self.check_call(&name, &args, span)?;
            return Ok(Expr::Call { name, args, span });
        }
        self.check_arg_end()?;
        let sig = self.sigs.get(&name).copied();
        let mut args = Vec::new();
        let mut npos = 0;
        let mut flags_done = false;
        loop {
            self.lx.skip_space();
            if self.at_terminator() {
                break;
            }
            let rest = self.lx.rest();
            if !flags_done && rest.starts_with('-') && !rest[1..].starts_with(|c: char| c.is_ascii_digit()) {
                if rest.starts_with("--") && rest[2..].chars().next().is_none_or(|c| c.is_whitespace()) {
                    self.lx.pos += 2;
                    flags_done = true;
                    continue;
                }
                if rest[1..].chars().next().is_some_and(|c| c.is_alphanumeric() || c == '-') {
                    self.flags(&name, sig.as_ref(), &mut args)?;
                    continue;
                }
            }
            let kind = sig.as_ref().and_then(|s| s.pos_kind(npos));
            let arg = match kind {
                Some(Kind::Cond) => self.cond_arg()?,
                _ => self.word_arg()?,
            };
            args.push(Arg::Pos(arg));
            npos += 1;
        }
        let span = name_span.to(Span::new(self.lx.pos, self.lx.pos));
        if let Some(s) = &sig {
            check_arity(s, &name, npos, span)?;
        }
        Ok(Expr::Call { name, args, span })
    }

    /// Kontrola volani ve tvaru `jmeno(...)` (zapis C) podle podpisu:
    /// stejna pravidla jako u prikazoveho tvaru.
    pub(crate) fn check_call(&self, name: &str, args: &[Arg], span: Span) -> Result<(), ShError> {
        let Some(s) = self.sigs.get(name) else { return Ok(()) };
        let mut npos = 0;
        for a in args {
            match a {
                Arg::Pos(_) => npos += 1,
                Arg::Named(n, _) => {
                    if s.flag_long(n).is_none() {
                        return Err(unknown_flag(name, s, n, span));
                    }
                }
            }
        }
        check_arity(s, name, npos, span)
    }

    /// Prepinace `-abc`, `--dlouhy`, `--dlouhy=hodnota`, `--dlouhy hodnota`.
    fn flags(&mut self, cmd: &str, sig: Option<&Sig>, args: &mut Vec<Arg>) -> Result<(), ShError> {
        let start = self.lx.pos;
        let truth = || Expr::Lit(Lit::Bool(true), Span::default());
        if self.lx.rest().starts_with("--") {
            self.lx.pos += 2;
            let nstart = self.lx.pos;
            while let Some(c) = self.lx.peek_char() {
                if !(c.is_alphanumeric() || c == '-' || c == '_') {
                    break;
                }
                self.lx.bump();
            }
            let long: String = self.lx.src[nstart..self.lx.pos].into();
            let span = Span::new(start, self.lx.pos);
            let mut value = None;
            if self.lx.peek_char() == Some('=') {
                self.lx.bump();
                value = Some(self.word_arg()?);
            } else {
                self.check_arg_end()?;
            }
            if let Some(s) = sig {
                let Some(f) = s.flag_long(&long) else {
                    return Err(unknown_flag(cmd, s, &alloc::format!("--{long}"), span));
                };
                if f.value.is_some() && value.is_none() {
                    self.lx.skip_space();
                    if self.at_terminator() {
                        return Err(ShError::new(code::ARGS, alloc::format!("přepínač --{long} čeká hodnotu"))
                            .at(span));
                    }
                    value = Some(self.word_arg()?);
                }
            }
            args.push(Arg::Named(long, value.unwrap_or_else(truth)));
            return Ok(());
        }
        self.lx.pos += 1;
        let letters_start = self.lx.pos;
        while let Some(c) = self.lx.peek_char() {
            if !c.is_alphanumeric() {
                break;
            }
            self.lx.bump();
        }
        let letters: String = self.lx.src[letters_start..self.lx.pos].into();
        let span = Span::new(start, self.lx.pos);
        self.check_arg_end()?;
        let n = letters.chars().count();
        for (i, c) in letters.chars().enumerate() {
            let Some(s) = sig else {
                args.push(Arg::Named(c.to_string(), truth()));
                continue;
            };
            let Some(f) = s.flag_short(c) else {
                return Err(unknown_flag(cmd, s, &alloc::format!("-{c}"), span));
            };
            if f.value.is_some() {
                if i + 1 != n {
                    return Err(ShError::new(
                        code::ARGS,
                        alloc::format!("přepínač -{c} čeká hodnotu, musí být ve skupině poslední"),
                    )
                    .at(span));
                }
                self.lx.skip_space();
                if self.at_terminator() {
                    return Err(ShError::new(code::ARGS, alloc::format!("přepínač -{c} čeká hodnotu")).at(span));
                }
                let v = self.word_arg()?;
                args.push(Arg::Named(f.long.into(), v));
            } else {
                args.push(Arg::Named(f.long.into(), truth()));
            }
        }
        Ok(())
    }

    /// Podminka `where`: closure `f => ...`, nebo vyraz nad radkem `it`.
    fn cond_arg(&mut self) -> Result<Expr, ShError> {
        self.lx.skip_space();
        let start = self.lx.pos;
        if self.lx.peek_char().is_some_and(|c| is_ident_start(c) || c == '(')
            && let Some(c) = self.try_closure()?
        {
            return Ok(c);
        }
        self.push_scope();
        self.declare(ROW);
        let depth = self.scope_depth() - 1;
        self.row.push(Row { fields: self.input_fields, scope_depth: depth });
        let body = self.expr();
        self.row.pop();
        self.pop_scope();
        let body = body?;
        if let Expr::Closure(..) = body {
            return Ok(body);
        }
        let span = Span::new(start, self.lx.pos);
        Ok(Expr::Closure(Closure { params: alloc::vec![ROW.into()], body: Box::new(body) }, span))
    }

    /// Jeden argument: `$x.pole`, retezec, `( )`, `[ ]`, `{ }`, closure
    /// nebo hole slovo.
    pub(crate) fn word_arg(&mut self) -> Result<Expr, ShError> {
        self.lx.skip_space();
        let start = self.lx.pos;
        let e = match self.lx.peek_char() {
            Some('$') => self.var_arg()?,
            Some('"' | '\'' | '[' | '{') => {
                let e = self.primary_for_arg()?;
                self.postfix(e)?
            }
            Some('(') => {
                if let Some(c) = self.try_closure()? {
                    return Ok(c);
                }
                let e = self.primary_for_arg()?;
                self.postfix(e)?
            }
            Some(c) if is_ident_start(c) => {
                if let Some(cl) = self.try_closure()? {
                    return Ok(cl);
                }
                self.bare_word()?
            }
            None => return Err(self.err("chybí argument", self.here())),
            _ => self.bare_word()?,
        };
        if self.lx.pos == start {
            return Err(self.err("čekám argument", self.here()));
        }
        self.check_arg_end()?;
        Ok(e)
    }

    fn primary_for_arg(&mut self) -> Result<Expr, ShError> {
        // `(`, `[`, `{`, retezec: stejne jako ve vyrazu.
        let save_row = core::mem::take(&mut self.row);
        let r = self.expr_bp(6);
        self.row = save_row;
        r
    }

    fn var_arg(&mut self) -> Result<Expr, ShError> {
        let start = self.lx.pos;
        self.lx.bump();
        let name: String = self.lx.ident().into();
        let span = Span::new(start, self.lx.pos);
        if name.is_empty() {
            return Err(self.err("za '$' patří jméno proměnné", span));
        }
        if !self.is_var(&name) {
            return Err(ShError::new(code::UNKNOWN, alloc::format!("proměnná '{name}' neexistuje"))
                .at(span)
                .hint("proměnnou zaveďte přes let"));
        }
        let mut e = Expr::Var(name, span);
        while self.lx.peek_char() == Some('.') && self.lx.peek_char2().is_some_and(is_ident_start) {
            self.lx.bump();
            let fs = self.lx.pos;
            let field: String = self.lx.ident().into();
            let s = e.span().to(Span::new(fs, self.lx.pos));
            e = Expr::Field(Box::new(e), field, s);
        }
        if self.lx.peek_char() == Some('$') || self.lx.peek_char().is_some_and(is_name_char) {
            return Err(self
                .err("proměnná uprostřed slova", Span::new(start, self.lx.pos + 1))
                .hint("použijte vložený výraz: \"{x}/soubor\" nebo slovo ~/složka/{x}"));
        }
        Ok(e)
    }

    /// Hole slovo: text, cislo, vzor nebo slovo s vlozenymi vyrazy `{x}`.
    fn bare_word(&mut self) -> Result<Expr, ShError> {
        let start = self.lx.pos;
        let mut text = String::new();
        let mut parts: Vec<Part> = Vec::new();
        let mut glob = false;
        let mut escaped = false;
        loop {
            let Some(c) = self.lx.peek_char() else { break };
            match c {
                c if is_bare_end(c) => break,
                '\\' => {
                    self.lx.bump();
                    match self.lx.bump() {
                        Some(e) => text.push(e),
                        None => return Err(self.err("za '\\' chybí znak", self.here())),
                    }
                    escaped = true;
                }
                '{' => {
                    if !text.is_empty() {
                        parts.push(Part::Text(core::mem::take(&mut text)));
                    }
                    let code_start = self.lx.pos + 1;
                    let end = self.lx.matching_brace(code_start)?;
                    self.lx.pos = code_start;
                    self.lx.skip_space();
                    let save_row = core::mem::take(&mut self.row);
                    let e = self.expr();
                    self.row = save_row;
                    let e = e?;
                    self.lx.skip_space();
                    if self.lx.pos != end {
                        return Err(self.err("ve vloženém {výrazu} je něco navíc", Span::new(self.lx.pos, end)));
                    }
                    self.lx.pos = end + 1;
                    parts.push(Part::Expr(e));
                }
                '*' | '?' | '[' => {
                    glob = true;
                    text.push(c);
                    self.lx.bump();
                }
                '$' => {
                    return Err(self
                        .err("proměnná uprostřed slova", Span::new(self.lx.pos, self.lx.pos + 1))
                        .hint("použijte vložený výraz {x}: ~/složka/{x}"));
                }
                _ => {
                    text.push(c);
                    self.lx.bump();
                }
            }
        }
        let span = Span::new(start, self.lx.pos);
        if !parts.is_empty() {
            if glob {
                return Err(self.err("vzor souboru nejde kombinovat s {výrazem}", span));
            }
            if !text.is_empty() {
                parts.push(Part::Text(text));
            }
            return Ok(Expr::Interp(parts, span));
        }
        if glob {
            if escaped {
                return Err(self.err("ve vzoru souboru nepoužívejte '\\'", span).hint("text dejte do uvozovek"));
            }
            return Ok(Expr::Glob(text, span));
        }
        if !escaped {
            match word_number(&text) {
                Some(Tok::Num(crate::ast::Num::Int(i))) => return Ok(Expr::Lit(Lit::Int(i), span)),
                Some(Tok::Num(crate::ast::Num::Float(f))) => return Ok(Expr::Lit(Lit::Float(f), span)),
                Some(Tok::Unit(value, unit)) => return Ok(Expr::Lit(Lit::Unit { value, unit }, span)),
                _ => {}
            }
            match text.as_str() {
                "true" => return Ok(Expr::Lit(Lit::Bool(true), span)),
                "false" => return Ok(Expr::Lit(Lit::Bool(false), span)),
                "null" => return Ok(Expr::Lit(Lit::Null, span)),
                _ => {}
            }
            if self.is_var(&text) {
                self.warnings.push(Warning {
                    message: alloc::format!("'{text}' je tu text; myslel jsi ${text}?"),
                    span,
                });
            }
        }
        Ok(Expr::Lit(Lit::Str(text), span))
    }
}

fn check_arity(s: &Sig, name: &str, npos: usize, span: Span) -> Result<(), ShError> {
    let required = s.params.iter().filter(|p| !p.optional).count();
    if npos < required {
        let missing = s.params[npos].name;
        return Err(ShError::new(code::ARGS, alloc::format!("příkaz {name}: chybí argument '{missing}'"))
            .at(span)
            .hint(usage(s)));
    }
    if s.rest.is_none() && npos > s.params.len() {
        return Err(ShError::new(code::ARGS, alloc::format!("příkaz {name}: příliš mnoho argumentů"))
            .at(span)
            .hint(usage(s)));
    }
    Ok(())
}

fn unknown_flag(cmd: &str, s: &Sig, flag: &str, span: Span) -> ShError {
    let mut known = String::new();
    for f in s.flags {
        if !known.is_empty() {
            known.push_str(", ");
        }
        if let Some(c) = f.short {
            known.push_str(&alloc::format!("-{c} "));
        }
        known.push_str(&alloc::format!("--{}", f.long));
    }
    let hint = if known.is_empty() {
        alloc::format!("{cmd} nemá žádné přepínače")
    } else {
        alloc::format!("přepínače: {known}")
    };
    ShError::new(code::ARGS, alloc::format!("příkaz {cmd} nezná přepínač {flag}")).at(span).hint(hint)
}

/// Kratky popis pouziti (`ls [path] [-r] [-a]`).
pub fn usage(s: &Sig) -> String {
    let mut u = String::from(s.name);
    for p in s.params {
        if p.optional {
            u.push_str(&alloc::format!(" [{}]", p.name));
        } else {
            u.push_str(&alloc::format!(" {}", p.name));
        }
    }
    if s.rest.is_some() {
        u.push_str(" ...");
    }
    for f in s.flags {
        match (f.short, f.value) {
            (Some(c), None) => u.push_str(&alloc::format!(" [-{c}]")),
            (_, Some(_)) => u.push_str(&alloc::format!(" [--{} hodnota]", f.long)),
            (None, None) => u.push_str(&alloc::format!(" [--{}]", f.long)),
        }
    }
    u
}

/// Znak, ktery ukonci hole slovo zapisu B (spolecne se zvyraznovacem
/// [`crate::highlight`]).
pub(crate) fn is_bare_end(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n' | '|' | ';' | '&' | '(' | ')' | '"' | '\'' | '<' | '>' | '}')
}
