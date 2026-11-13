//! Zvyrazneni syntaxe: useky radku se semantickou tridou (ne barvou).
//!
//! Jediny zdroj pravdy pro terminal (okno i sluzbu relaci) i dokumentaci
//! (`xtask`, bloky `shell-b`/`shell-c`): pravidla bere z lexeru
//! ([`crate::lex`]: retezce, vlozene vyrazy, cisla s jednotkami,
//! interpunkce) a parseru (klicova slova, konce holych slov zapisu B,
//! [`crate::parse::cmd::is_bare_end`]).
//!
//! Dve urovne:
//! - **lexikalni** (`ctx = None`): okamzite v okne pri kazde klavese; slovo
//!   na miste prikazu je vzdy `Command`,
//! - **vyznamova** (`Some(ctx)`): sluzba zna prikazy, funkce, programy
//!   a promenne relace - neznamy prikaz je `Unknown`, promenna `Var`,
//!   pole v podmince `where` `Field`; navic misto chyby parseru.
//!
//! Barvy prideli az motiv terminalu ([`Class::name`] = jmeno tridy v CSS
//! dokumentace i v souboru motivu).

use alloc::string::String;
use alloc::vec::Vec;

use crate::lex::{Lexer, RawPart, is_ident_char, is_ident_start, word_number};
use crate::parse::cmd::is_bare_end;
use crate::parse::{Options, is_keyword, parse};
use crate::sig::Sigs;

/// Semanticka trida useku.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Class {
    Plain = 0,
    /// Znamy prikaz (vestaveny, funkce, program).
    Command = 1,
    /// Neznamy prikaz (varovani uz pri psani).
    Unknown = 2,
    Keyword = 3,
    /// Prepinac `-r`, `--depth`.
    Flag = 4,
    /// Promenna `$x` nebo jmeno promenne ve vyrazu.
    Var = 5,
    Str = 6,
    /// Zavorky vlozeneho vyrazu `{...}` v retezci nebo slove.
    Interp = 7,
    /// Cislo (i s jednotkou `5MB`, `2s`).
    Num = 8,
    Comment = 9,
    Op = 10,
    /// Roura `|`.
    Pipe = 11,
    /// Pole zaznamu (`it.size`, `size` v podmince `where`).
    Field = 12,
    /// Cesta nebo vzor (`~/foto`, `*.jpg`).
    Path = 13,
}

impl Class {
    pub const ALL: &'static [Class] = &[
        Class::Plain,
        Class::Command,
        Class::Unknown,
        Class::Keyword,
        Class::Flag,
        Class::Var,
        Class::Str,
        Class::Interp,
        Class::Num,
        Class::Comment,
        Class::Op,
        Class::Pipe,
        Class::Field,
        Class::Path,
    ];

    /// Jmeno tridy (CSS `hl-<jmeno>`, klic v souboru motivu).
    pub fn name(self) -> &'static str {
        match self {
            Class::Plain => "plain",
            Class::Command => "cmd",
            Class::Unknown => "unknown",
            Class::Keyword => "kw",
            Class::Flag => "flag",
            Class::Var => "var",
            Class::Str => "str",
            Class::Interp => "interp",
            Class::Num => "num",
            Class::Comment => "comment",
            Class::Op => "op",
            Class::Pipe => "pipe",
            Class::Field => "field",
            Class::Path => "path",
        }
    }

    pub fn from_u8(v: u8) -> Option<Class> {
        Class::ALL.get(v as usize).copied()
    }
}

/// Usek `start..end` (bajty) s tridou.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub class: Class,
}

/// Misto chyby parseru (podtrhnout).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorMark {
    pub start: usize,
    pub end: usize,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Highlight {
    /// Serazene, neprekryvajici se useky (mezery a `Plain` vynechane).
    pub spans: Vec<Span>,
    pub error: Option<ErrorMark>,
}

/// Co zna sluzba relaci (vyznamova uroven).
pub struct Ctx<'a> {
    pub sigs: &'a dyn Sigs,
    /// Promenne v rozsahu.
    pub vars: &'a [String],
    /// Funkce relace a programy (s manifestem).
    pub commands: &'a [String],
}

impl Ctx<'_> {
    fn known(&self, name: &str) -> bool {
        self.sigs.get(name).is_some() || self.commands.iter().any(|c| c == name)
    }

    fn is_var(&self, name: &str) -> bool {
        name == "it" || self.vars.iter().any(|v| v == name)
    }
}

/// Zvyrazni radek (nebo cely skript).
pub fn highlight(src: &str, ctx: Option<&Ctx>) -> Highlight {
    let mut s = Scan { src, ctx, out: Vec::new(), locals: Vec::new() };
    s.run(0, src.len(), true);
    s.out.sort_by_key(|x| x.start);
    let error = ctx.and_then(|c| error_mark(src, c));
    Highlight { spans: s.out, error }
}

/// Chyba parseru, ktera neni jen "radek jeste neni dopsany" (konec vstupu).
fn error_mark(src: &str, ctx: &Ctx) -> Option<ErrorMark> {
    let opts = Options { style: None, sigs: ctx.sigs, vars: ctx.vars.to_vec(), script: false };
    let e = parse(src, &opts).err()?;
    let span = e.span?;
    let (start, stop) = (span.start as usize, span.end as usize);
    // Chyba sahajici az na konec = radek jen jeste neni dopsany.
    if stop >= src.trim_end().len() {
        return None;
    }
    Some(ErrorMark { start, end: stop.max(start + 1).min(src.len()), message: e.message })
}

/// Ma prikaz parametr podminky (`where size > 1MB`)? Struktura jazyka,
/// proto i bez kontextu relace (vestavene podpisy).
fn has_cond(ctx: Option<&Ctx>, name: &str) -> bool {
    let sig = match ctx {
        Some(c) => c.sigs.get(name),
        None => crate::sig::Sigs::get(&crate::sig::Builtins, name),
    };
    sig.is_some_and(|s| s.params.iter().any(|p| p.kind == crate::sig::Kind::Cond))
}

struct Scan<'a, 'c> {
    src: &'a str,
    ctx: Option<&'c Ctx<'c>>,
    out: Vec<Span>,
    /// Promenne zavedene v textu (`let x`, `for f in`, `f =>`): plati i bez
    /// znalosti relace.
    locals: Vec<String>,
}

impl Scan<'_, '_> {
    fn is_var(&self, name: &str) -> bool {
        self.locals.iter().any(|l| l == name) || self.ctx.map_or(name == "it", |x| x.is_var(name))
    }

    /// Nasleduje (za mezerami) `=>`?
    fn arrow_after(&self, i: usize) -> bool {
        self.src.get(i..).is_some_and(|r| r.trim_start_matches([' ', '\t']).starts_with("=>"))
    }

    fn push(&mut self, start: usize, end: usize, class: Class) {
        if end > start && class != Class::Plain {
            self.out.push(Span { start, end, class });
        }
    }

    fn ch(&self, i: usize) -> Option<char> {
        self.src.get(i..).and_then(|r| r.chars().next())
    }

    /// Konec holeho slova od `i` (pravidla zapisu B).
    fn word_end(&self, mut i: usize, end: usize) -> usize {
        while i < end {
            let Some(c) = self.ch(i) else { break };
            if c == '{' {
                // Vlozeny vyraz patri ke slovu (`foto-{x}.jpg`).
                match Lexer::new(self.src, i).matching_brace(i + 1) {
                    Ok(b) if b < end => {
                        i = b + 1;
                        continue;
                    }
                    _ => break,
                }
            }
            if is_bare_end(c) {
                break;
            }
            i += c.len_utf8();
        }
        i
    }

    fn ident_end(&self, mut i: usize, end: usize) -> usize {
        while i < end {
            match self.ch(i) {
                Some(c) if is_ident_char(c) || c == '-' && self.ch(i + 1).is_some_and(is_ident_char) => {
                    i += c.len_utf8()
                }
                _ => break,
            }
        }
        i
    }

    /// Retezec od `i` (`"` nebo `'`); vrati konec.
    fn string(&mut self, i: usize, end: usize) -> usize {
        let mut l = Lexer::new(self.src, i);
        let raw = self.src.as_bytes()[i] == b'\'';
        let parts = if raw {
            match self.src[i + 1..end].find('\'') {
                Some(k) => {
                    self.push(i, i + k + 2, Class::Str);
                    return i + k + 2;
                }
                None => None,
            }
        } else {
            l.string().ok()
        };
        let Some(parts) = parts else {
            // Nedopsany retezec: do konce.
            self.push(i, end, Class::Str);
            return end;
        };
        let stop = l.pos.min(end);
        let mut at = i;
        for p in parts {
            if let RawPart::Code(a, b) = p {
                // `{` je na a-1, `}` na b.
                self.push(at, a - 1, Class::Str);
                self.push(a - 1, a, Class::Interp);
                self.run(a, b, false);
                self.push(b, b + 1, Class::Interp);
                at = b + 1;
            }
        }
        self.push(at, stop, Class::Str);
        stop
    }

    /// Vlozeny vyraz `{...}` ve slove od `i` (na `{`); vrati konec.
    fn interp(&mut self, i: usize, end: usize) -> usize {
        let l = Lexer::new(self.src, i);
        match l.matching_brace(i + 1) {
            Ok(b) if b < end => {
                self.push(i, i + 1, Class::Interp);
                self.run(i + 1, b, false);
                self.push(b, b + 1, Class::Interp);
                b + 1
            }
            _ => {
                self.push(i, i + 1, Class::Interp);
                i + 1
            }
        }
    }

    /// Projde `start..end`; `cmd` = na zacatku je misto prikazu.
    fn run(&mut self, start: usize, end: usize, mut cmd: bool) {
        let mut i = start;
        let mut last_cmd: Option<String> = None;
        // Predchozi token bylo jmeno volani `f(` (argumenty = vyrazy).
        let mut call_args = false;
        let mut redirect = false;
        // Za prikazem s podminkou (`where`) az do roury vyraz: `&&`, `<`
        // jsou operatory, hola jmena pole.
        let mut cond = false;
        // Hloubka zavorek `(`: uvnitr (vyraz, volani C) neni `>` presmerovani.
        let mut depth = 0usize;
        while i < end {
            let Some(c) = self.ch(i) else { break };
            let n = c.len_utf8();
            match c {
                ' ' | '\t' | '\r' => {
                    i += 1;
                    continue;
                }
                '\n' | ';' => {
                    if c == ';' {
                        self.push(i, i + 1, Class::Op);
                    }
                    cmd = true;
                    cond = false;
                    last_cmd = None;
                    i += 1;
                    continue;
                }
                '#' => {
                    let e = self.src[i..end].find('\n').map_or(end, |k| i + k);
                    self.push(i, e, Class::Comment);
                    i = e;
                    continue;
                }
                '"' | '\'' => {
                    i = self.string(i, end);
                    cmd = false;
                    continue;
                }
                '$' => {
                    let e = self.ident_end(i + 1, end);
                    self.push(i, e, Class::Var);
                    i = self.fields(e, end);
                    cmd = false;
                    continue;
                }
                '|' | '&' => {
                    let two = self.src[i..end].starts_with("||") || self.src[i..end].starts_with("&&");
                    let len = if two { 2 } else { 1 };
                    let class = if c == '|' && !two { Class::Pipe } else { Class::Op };
                    self.push(i, i + len, class);
                    if cond && two {
                        // Logicky operator v podmince.
                        i += len;
                        continue;
                    }
                    cmd = true;
                    cond = false;
                    last_cmd = None;
                    i += len;
                    continue;
                }
                '{' => {
                    self.push(i, i + 1, Class::Op);
                    cmd = true;
                    i += 1;
                    continue;
                }
                '(' => {
                    self.push(i, i + 1, Class::Op);
                    depth += 1;
                    cmd = !call_args;
                    call_args = false;
                    i += 1;
                    continue;
                }
                '}' | ')' | '[' | ']' | ',' | ':' => {
                    if c == ')' {
                        depth = depth.saturating_sub(1);
                    }
                    self.push(i, i + 1, Class::Op);
                    i += 1;
                    continue;
                }
                '>' | '<' if !cmd => {
                    let len = if self.src[i..end].starts_with(">>") || self.src[i..end].starts_with(">=") || self.src[i..end].starts_with("<=") { 2 } else { 1 };
                    self.push(i, i + len, Class::Op);
                    // V prikazu B je `>`, `>>`, `<` presmerovani: dal cesta.
                    redirect = !cond && depth == 0 && last_cmd.is_some() && !self.src[i..i + len].contains('=');
                    i += len;
                    continue;
                }
                '-' if self.ch(i + 1).is_some_and(|d| d == '-' || is_ident_start(d)) && !cmd => {
                    let e = self.word_end(i, end);
                    self.push(i, e, Class::Flag);
                    i = e;
                    continue;
                }
                '=' | '!' | '+' | '*' | '/' | '%' | '?' | '.' | '-' if !self.is_word_char_start(i, cmd) => {
                    let len = ["=>", "==", "!="].iter().find(|p| self.src[i..end].starts_with(**p)).map_or(n, |p| p.len());
                    self.push(i, i + len, Class::Op);
                    // Za prirazenim a za `=>` (telo closure, mimo podminku
                    // `where`) je misto prikazu: `let x = ls`, `else e => log`.
                    if c == '=' && (len == 1 || self.src[i..].starts_with("=>") && !cond) {
                        cmd = true;
                    }
                    i += len;
                    continue;
                }
                _ => {}
            }
            // Slovo.
            if c.is_ascii_digit() || c == '-' && self.ch(i + 1).is_some_and(|d| d.is_ascii_digit()) {
                let e = self.word_end(i, end);
                let w = &self.src[i..e];
                let e2 = self.number_end(i, e);
                if word_number(w).is_some() {
                    self.push(i, e, Class::Num);
                } else if e2 > i && word_number(&self.src[i..e2]).is_some() {
                    self.push(i, e2, Class::Num);
                    i = e2;
                    continue;
                } else {
                    self.push(i, e, Class::Path);
                }
                i = e;
                cmd = false;
                redirect = false;
                continue;
            }
            let id_end = if is_ident_start(c) { self.ident_end(i, end) } else { i };
            let name = &self.src[i..id_end];
            if id_end > i && is_keyword(name) {
                self.push(i, id_end, Class::Keyword);
                cmd = matches!(name, "try" | "else" | "return");
                i = id_end;
                if matches!(name, "let" | "for") {
                    // Dalsi jmeno je nova promenna.
                    let mut k = i;
                    while self.ch(k).is_some_and(|c| c == ' ' || c == '\t') {
                        k += 1;
                    }
                    let e = self.ident_end(k, end);
                    if e > k && !is_keyword(&self.src[k..e]) {
                        self.locals.push(self.src[k..e].into());
                        self.push(k, e, Class::Var);
                        i = e;
                    }
                }
                continue;
            }
            if id_end > i && self.arrow_after(id_end) {
                // Parametr closure `f => ...`.
                self.locals.push(name.into());
                self.push(i, id_end, Class::Var);
                cmd = false;
                i = id_end;
                continue;
            }
            if id_end > i && self.ch(id_end) == Some(':') && self.ch(id_end + 1) != Some(':') {
                // Klic `read:` (manifest), pojmenovany argument `recursive:`.
                self.push(i, id_end, Class::Field);
                cmd = false;
                i = id_end;
                continue;
            }
            if id_end > i && self.ch(id_end) == Some('(') {
                // Volani zapisu C `f(...)`.
                let known = self.ctx.is_none_or(|x| x.known(name));
                self.push(i, id_end, if known { Class::Command } else { Class::Unknown });
                call_args = true;
                last_cmd = Some(name.into());
                cmd = false;
                i = id_end;
                continue;
            }
            if i > start && self.src.as_bytes()[i - 1] == b'.' && id_end > i {
                // Metoda zapisu C `.where(`, nebo pole `.size`.
                let class = if self.ch(id_end) == Some('(') { Class::Command } else { Class::Field };
                self.push(i, id_end, class);
                i = id_end;
                continue;
            }
            if id_end > i && self.ch(id_end) == Some('.') && self.ch(id_end + 1).is_some_and(is_ident_start)
                && self.is_var(name)
            {
                // Promenna s polem `it.size`.
                self.push(i, id_end, Class::Var);
                i = self.fields(id_end, end);
                cmd = false;
                continue;
            }
            let e = self.word_end(i, end);
            if cmd {
                let w = &self.src[i..e];
                let class = match self.ctx {
                    _ if self.is_var(w) => Class::Var,
                    // Hola cesta (slozka, skript, program): cesta, ne
                    // neznamy prikaz.
                    _ if crate::eval::is_bare_path(w) => Class::Path,
                    None => Class::Command,
                    Some(x) if x.known(w) => Class::Command,
                    Some(_) => Class::Unknown,
                };
                self.push(i, e, class);
                cond = has_cond(self.ctx, w);
                last_cmd = Some(w.into());
                cmd = false;
                i = e;
                continue;
            }
            // Argument nebo vyraz.
            let word = &self.src[i..e];
            let class = if redirect || word.contains(['/', '~', '*', '?']) || word.starts_with('.') {
                Class::Path
            } else if id_end == e && self.is_var(word) {
                Class::Var
            } else if id_end == e && cond && !is_keyword(word) {
                Class::Field
            } else {
                Class::Plain
            };
            redirect = false;
            // Slovo s vlozenym vyrazem `foto-{x}`.
            let mut k = i;
            while k < e {
                let next = self.src[k..e].find('{').map_or(e, |p| k + p);
                self.push(k, next, class);
                if next < e {
                    k = self.interp(next, end);
                } else {
                    k = e;
                }
            }
            i = if class == Class::Var { self.fields(e, end) } else { k.max(e) };
        }
    }

    /// `.pole` za promennou.
    fn fields(&mut self, mut i: usize, end: usize) -> usize {
        while i < end && self.ch(i) == Some('.') && self.ch(i + 1).is_some_and(is_ident_start) {
            let e = self.ident_end(i + 1, end);
            self.push(i, i + 1, Class::Op);
            self.push(i + 1, e, Class::Field);
            i = e;
        }
        i
    }

    /// Delka cisla zapisu C (`5`, `5MB`, `1.5`) pred interpunkci.
    fn number_end(&self, mut i: usize, end: usize) -> usize {
        while i < end {
            match self.ch(i) {
                Some(c) if c.is_ascii_alphanumeric() || c == '.' && self.ch(i + 1).is_some_and(|d| d.is_ascii_digit()) => {
                    i += 1
                }
                Some('-') if i == 0 => i += 1,
                _ => break,
            }
        }
        i
    }

    /// Zacina na `i` slovo (cesta `./x`, `/tmp`, vzor `*.rs`), ne operator?
    fn is_word_char_start(&self, i: usize, cmd: bool) -> bool {
        let c = self.ch(i).unwrap_or(' ');
        let next = self.ch(i + 1).unwrap_or(' ');
        let prev_space = i == 0 || matches!(self.src.as_bytes()[i - 1], b' ' | b'\t' | b'(' | b'\n');
        match c {
            '/' | '.' | '*' | '?' => prev_space && !matches!(next, ' ' | '\t') || cmd,
            _ => false,
        }
    }
}
