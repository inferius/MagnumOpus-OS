//! Lexer vyrazoveho rezimu (spolecny pro B i C).
//!
//! Prikazovy rezim zapisu B (holá slova, prepinace) cte slova primo ze
//! zdroje v `parse::cmd`; tento lexer pouziva pro vse ostatni. Lexer je
//! "na vyzadani": parser si pamatuje pozici a muze se vratit (`pos`).

use alloc::string::String;
use alloc::vec::Vec;

use crate::ast::{Num, Unit};
use crate::error::{ShError, code};
use crate::span::Span;

/// Usek retezce v uvozovkach.
#[derive(Debug, Clone, PartialEq)]
pub enum RawPart {
    Text(String),
    /// Vlozeny vyraz `{...}`: bajtovy rozsah ve zdroji (bez zavorek).
    Code(usize, usize),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Num(Num),
    Unit(Num, Unit),
    Str(Vec<RawPart>),
    Punct(&'static str),
    Newline,
    Eof,
}

const PUNCT: &[&str] = &[
    "=>", "==", "!=", "<=", ">=", "&&", "||", ">>", "(", ")", "[", "]", "{", "}", ",", ":", ".", ";", "<",
    ">", "+", "-", "*", "/", "%", "!", "|", "?", "=", "&", "$", "#",
];

pub fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

pub fn is_ident_char(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

#[derive(Clone)]
pub struct Lexer<'a> {
    pub src: &'a str,
    pub pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str, pos: usize) -> Self {
        Self { src, pos }
    }

    pub fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    pub fn peek_char(&self) -> Option<char> {
        self.rest().chars().next()
    }

    pub fn peek_char2(&self) -> Option<char> {
        let mut it = self.rest().chars();
        it.next();
        it.next()
    }

    pub fn bump(&mut self) -> Option<char> {
        let c = self.peek_char()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    /// Preskoci mezery a tabulatory (a pokracovani radku `\` + novy radek).
    /// Komentar ani novy radek nepreskakuje.
    pub fn skip_space(&mut self) {
        loop {
            match self.peek_char() {
                Some(' ' | '\t' | '\r') => {
                    self.pos += 1;
                }
                Some('\\') if self.rest()[1..].starts_with('\n') => self.pos += 2,
                Some('\\') if self.rest()[1..].starts_with("\r\n") => self.pos += 3,
                _ => return,
            }
        }
    }

    /// Preskoci mezery, nove radky i komentare (uvnitr zavorek).
    pub fn skip_space_nl(&mut self) {
        loop {
            self.skip_space();
            match self.peek_char() {
                Some('\n') => self.pos += 1,
                Some('#') => self.skip_comment(),
                _ => return,
            }
        }
    }

    pub fn skip_comment(&mut self) {
        while let Some(c) = self.peek_char() {
            if c == '\n' {
                break;
            }
            self.pos += c.len_utf8();
        }
    }

    /// Dalsi token (bez preskoceni mezer - to dela parser).
    pub fn token(&mut self) -> Result<(Tok, Span), ShError> {
        let start = self.pos;
        let Some(c) = self.peek_char() else {
            return Ok((Tok::Eof, Span::new(start, start)));
        };
        let tok = if c == '\n' {
            self.pos += 1;
            Tok::Newline
        } else if c.is_ascii_digit() {
            self.number()?
        } else if is_ident_start(c) {
            let s = self.ident();
            Tok::Ident(s.into())
        } else if c == '"' {
            Tok::Str(self.string()?)
        } else if c == '\'' {
            Tok::Str(self.raw_string()?)
        } else {
            let rest = self.rest();
            match PUNCT.iter().find(|p| rest.starts_with(**p)) {
                Some(p) => {
                    self.pos += p.len();
                    Tok::Punct(p)
                }
                None => {
                    return Err(ShError::new(
                        code::PARSE,
                        alloc::format!("neocekavany znak '{c}'"),
                    )
                    .at(Span::new(start, start + c.len_utf8())));
                }
            }
        };
        Ok((tok, Span::new(start, self.pos)))
    }

    /// Nahled na dalsi token bez posunu.
    pub fn peek(&self) -> Result<Tok, ShError> {
        let mut l = self.clone();
        l.token().map(|(t, _)| t)
    }

    pub fn ident(&mut self) -> &'a str {
        let start = self.pos;
        while let Some(c) = self.peek_char() {
            if !is_ident_char(c) {
                break;
            }
            self.pos += c.len_utf8();
        }
        &self.src[start..self.pos]
    }

    fn number(&mut self) -> Result<Tok, ShError> {
        let start = self.pos;
        let mut text = String::new();
        let mut float = false;
        while let Some(c) = self.peek_char() {
            if c.is_ascii_digit() {
                text.push(c);
                self.pos += 1;
            } else if c == '_' && self.peek_char2().is_some_and(|d| d.is_ascii_digit()) {
                self.pos += 1;
            } else if c == '.' && !float && self.peek_char2().is_some_and(|d| d.is_ascii_digit()) {
                float = true;
                text.push('.');
                self.pos += 1;
            } else {
                break;
            }
        }
        let num = if float {
            Num::Float(text.parse::<f64>().ok().filter(|f| f.is_finite()).ok_or_else(|| bad_number(start, self.pos))?)
        } else {
            Num::Int(text.parse::<i64>().map_err(|_| bad_number(start, self.pos))?)
        };
        // Jednotka hned za cislem (bez mezery).
        let mut l = self.clone();
        let suffix = l.ident();
        if !suffix.is_empty() {
            if let Some(u) = Unit::parse(suffix) {
                self.pos = l.pos;
                return Ok(Tok::Unit(num, u));
            }
            return Err(ShError::new(code::PARSE, alloc::format!("neznama jednotka '{suffix}'"))
                .at(Span::new(start, l.pos))
                .hint("velikosti: B KB MB GB TB KiB MiB GiB TiB, doby: ns us ms s min h d w y"));
        }
        Ok(Tok::Num(num))
    }

    /// Retezec `"..."` s escape sekvencemi a vlozenymi vyrazy `{...}`.
    pub fn string(&mut self) -> Result<Vec<RawPart>, ShError> {
        let start = self.pos;
        self.pos += 1;
        let mut parts = Vec::new();
        let mut text = String::new();
        loop {
            let Some(c) = self.bump() else {
                return Err(ShError::new(code::PARSE, "neukonceny retezec").at(Span::new(start, self.pos)));
            };
            match c {
                '"' => break,
                '\\' => {
                    let e = self.bump().ok_or_else(|| {
                        ShError::new(code::PARSE, "neukonceny retezec").at(Span::new(start, self.pos))
                    })?;
                    text.push(match e {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '0' => '\0',
                        '\\' | '"' | '{' | '}' | '\'' => e,
                        _ => {
                            return Err(ShError::new(code::PARSE, alloc::format!("neznama escape sekvence \\{e}"))
                                .at(Span::new(self.pos - 2, self.pos))
                                .hint("platne: \\n \\t \\r \\0 \\\\ \\\" \\{ \\}"));
                        }
                    });
                }
                '{' => {
                    if !text.is_empty() {
                        parts.push(RawPart::Text(core::mem::take(&mut text)));
                    }
                    let code_start = self.pos;
                    let code_end = self.matching_brace(code_start)?;
                    parts.push(RawPart::Code(code_start, code_end));
                    self.pos = code_end + 1;
                }
                _ => text.push(c),
            }
        }
        if !text.is_empty() || parts.is_empty() {
            parts.push(RawPart::Text(text));
        }
        Ok(parts)
    }

    /// Najde `}` k vlozenemu vyrazu, ktery zacina na `from` (uvnitr muze
    /// byt dalsi retezec nebo zaznam).
    pub fn matching_brace(&self, from: usize) -> Result<usize, ShError> {
        let b = self.src.as_bytes();
        let mut depth = 0usize;
        let mut i = from;
        while i < b.len() {
            match b[i] {
                b'{' => depth += 1,
                b'}' => {
                    if depth == 0 {
                        return Ok(i);
                    }
                    depth -= 1;
                }
                b'"' => {
                    i += 1;
                    while i < b.len() && b[i] != b'"' {
                        if b[i] == b'\\' {
                            i += 1;
                        }
                        i += 1;
                    }
                }
                b'\n' => break,
                _ => {}
            }
            i += 1;
        }
        Err(ShError::new(code::PARSE, "chybi '}' ukoncujici vlozeny vyraz").at(Span::new(from - 1, from)))
    }

    /// Retezec `'...'` bez escape sekvenci a bez vlozenych vyrazu.
    fn raw_string(&mut self) -> Result<Vec<RawPart>, ShError> {
        let start = self.pos;
        self.pos += 1;
        let Some(end) = self.rest().find('\'') else {
            return Err(ShError::new(code::PARSE, "neukonceny retezec").at(Span::new(start, self.src.len())));
        };
        let text = &self.rest()[..end];
        self.pos += end + 1;
        Ok(alloc::vec![RawPart::Text(text.into())])
    }
}

fn bad_number(start: usize, end: usize) -> ShError {
    ShError::new(code::PARSE, "cislo je mimo rozsah").at(Span::new(start, end))
}

/// Lexuje cele slovo jako cislo s jednotkou (pro hola slova v B).
pub fn word_number(word: &str) -> Option<Tok> {
    let (neg, body) = match word.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, word),
    };
    if !body.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let mut l = Lexer::new(body, 0);
    let t = l.number().ok()?;
    if l.pos != body.len() {
        return None;
    }
    if !neg {
        return Some(t);
    }
    let negate = |n: Num| match n {
        Num::Int(i) => Num::Int(-i),
        Num::Float(f) => Num::Float(-f),
    };
    Some(match t {
        Tok::Num(n) => Tok::Num(negate(n)),
        Tok::Unit(n, u) => Tok::Unit(negate(n), u),
        other => other,
    })
}
