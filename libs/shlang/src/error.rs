//! Chyba jako hodnota: kod, popis, misto, pricina a rada.
//!
//! Kody jsou bud nazvy chyb systemu (`NOT_FOUND`, viz `docs/cs/chyby.md`),
//! nebo kody shellu `SH_*` (seznam v [`code`], popis v kapitole Chyby
//! shellu v `docs/cs/navrh-shell.md`). Hlaseni jsou cesky (rozhodnuti 6).

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use core::fmt;

use crate::span::Span;

/// Kody chyb shellu.
pub mod code {
    /// Syntakticka chyba.
    pub const PARSE: &str = "SH_PARSE";
    /// Nezname jmeno (promenna, funkce, prikaz).
    pub const UNKNOWN: &str = "SH_UNKNOWN";
    /// Hodnota jineho typu, nez se ceka.
    pub const TYPE: &str = "SH_TYPE";
    /// Spatne argumenty prikazu (chybi, navic, neznamy prepinac).
    pub const ARGS: &str = "SH_ARGS";
    /// Vzor souboru nic nenasel.
    pub const NO_MATCH: &str = "SH_NO_MATCH";
    /// Preruseno (Ctrl-C).
    pub const INTERRUPTED: &str = "SH_INTERRUPTED";
    /// Skript chce neco, co manifest `needs` nepovoluje.
    pub const NEEDS: &str = "SH_NEEDS";
    /// Uzivatel zmenu v nahledu odmitl.
    pub const REJECTED: &str = "SH_REJECTED";
    /// Neni co vratit (`undo`).
    pub const NO_UNDO: &str = "SH_NO_UNDO";
    /// Deleni nulou, preteceni.
    pub const ARITH: &str = "SH_ARITH";
    /// Chyba vyvolana skriptem (`error "..."`).
    pub const USER: &str = "SH_USER";
    /// Data se nepodarilo prevest (`from json`).
    pub const FORMAT: &str = "SH_FORMAT";
    /// Nepodporovana verze jazyka v `lang`.
    pub const LANG: &str = "SH_LANG";
    /// Prilis hluboke vnoreni (rekurze).
    pub const DEPTH: &str = "SH_DEPTH";

    /// Vsechny kody shellu (test hlida, ze jsou popsane v dokumentaci).
    pub const ALL: &[&str] = &[
        PARSE, UNKNOWN, TYPE, ARGS, NO_MATCH, INTERRUPTED, NEEDS, REJECTED, NO_UNDO, ARITH, USER, FORMAT, LANG,
        DEPTH,
    ];
}

/// Chyba shellu (i hodnota typu `error` v jazyce).
#[derive(Debug, Clone, PartialEq)]
pub struct ShError {
    pub code: String,
    pub message: String,
    pub span: Option<Span>,
    /// Rada, co s tim (napr. "myslel jsi $x?").
    pub hint: Option<String>,
    pub cause: Option<Box<ShError>>,
}

impl ShError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into(), span: None, hint: None, cause: None }
    }

    /// Chyba systemu (`abi::Error`) s popisem, co se delalo.
    pub fn sys(e: abi::Error, what: impl Into<String>) -> Self {
        let mut s = Self::new(e.name(), what);
        s.cause = Some(Box::new(Self::new(e.name(), e.description())));
        s
    }

    pub fn at(mut self, span: Span) -> Self {
        if self.span.is_none() {
            self.span = Some(span);
        }
        self
    }

    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn caused_by(mut self, cause: ShError) -> Self {
        self.cause = Some(Box::new(cause));
        self
    }

    /// Hlaseni pro cloveka: kod, popis, misto ve zdroji s ukazatelem,
    /// rada a pricina.
    pub fn report(&self, src: &str) -> String {
        let mut out = String::new();
        out.push_str("chyba ");
        out.push_str(&self.code);
        out.push_str(": ");
        out.push_str(&self.message);
        if let Some(span) = self.span
            && (span.start as usize) <= src.len()
        {
            let (line, col) = span.line_col(src);
            out.push_str(&alloc::format!("\n  na radku {line}, sloupec {col}:\n"));
            let text = src.lines().nth(line as usize - 1).unwrap_or("");
            out.push_str("    ");
            out.push_str(text);
            out.push_str("\n    ");
            for _ in 1..col {
                out.push(' ');
            }
            let len = (span.end.saturating_sub(span.start) as usize).clamp(1, text.chars().count().max(1));
            for _ in 0..len {
                out.push('^');
            }
        }
        if let Some(h) = &self.hint {
            out.push_str("\n  rada: ");
            out.push_str(h);
        }
        let mut cause = self.cause.as_deref();
        while let Some(c) = cause {
            out.push_str("\n  pricina: ");
            out.push_str(&c.code);
            out.push_str(": ");
            out.push_str(&c.message);
            cause = c.cause.as_deref();
        }
        out
    }
}

impl fmt::Display for ShError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl From<abi::Error> for ShError {
    fn from(e: abi::Error) -> Self {
        Self::new(e.name(), e.description().to_string())
    }
}

/// Varovani parseru (neni chyba, skript pobezi).
#[derive(Debug, Clone, PartialEq)]
pub struct Warning {
    pub message: String,
    pub span: Span,
}
