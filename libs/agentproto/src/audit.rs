//! Auditni log: kazde volani nastroje a modelu.
//!
//! Zaznam je jeden radek textu (pole oddelena tabulatorem, `\t`, `\n`
//! a `\\` uvnitr pole escapovane), aby log sel cist i bez nastroju.
//! Obsah zapisu se do logu **nedava** - jen cesta, delka a hash
//! ([`content_hash`]), API klic nikdy.

use alloc::string::String;
use alloc::vec::Vec;

use crate::{Error, Result};

/// Druh zaznamu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Volani nastroje.
    Tool,
    /// Volani modelu (poskytovatel, tokeny, cena, odeslane bajty).
    Model,
    /// Relace: otevreni, vraceni, konec.
    Session,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Tool => "tool",
            Kind::Model => "model",
            Kind::Session => "session",
        }
    }

    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "tool" => Kind::Tool,
            "model" => Kind::Model,
            "session" => Kind::Session,
            _ => return Err(Error::InvalidArgs),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub seq: u64,
    /// Cas (sekundy od 1970, UTC).
    pub time: i64,
    pub session: u64,
    pub step: u32,
    pub kind: Kind,
    /// Nastroj (`fs.write`), model (`claude/claude-opus-5-5`) nebo udalost relace.
    pub what: String,
    /// Souhrn vstupu (cesta, delka a hash obsahu; nikdy obsah).
    pub input: String,
    /// `allow`, `allow-ws`, `confirm-yes`, `confirm-no`, `deny`, `-`.
    pub decision: String,
    /// `ok`, kod chyby, nebo u modelu tokeny / cena / odeslane bajty.
    pub result: String,
}

fn esc(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
}

fn unesc(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

/// Nejvic znaku souhrnu vstupu v zaznamu.
pub const MAX_INPUT: usize = 300;

impl Record {
    /// Radek logu (s `\n` na konci).
    pub fn line(&self) -> String {
        let mut s = alloc::format!("{}\t{}\t{}\t{}\t{}\t", self.seq, self.time, self.session, self.step, self.kind.name());
        esc(&self.what, &mut s);
        s.push('\t');
        let input: String = self.input.chars().take(MAX_INPUT).collect();
        esc(&input, &mut s);
        s.push('\t');
        esc(&self.decision, &mut s);
        s.push('\t');
        esc(&self.result, &mut s);
        s.push('\n');
        s
    }

    pub fn parse(line: &str) -> Result<Self> {
        let f: Vec<&str> = line.trim_end_matches('\n').split('\t').collect();
        if f.len() != 9 {
            return Err(Error::InvalidArgs);
        }
        let num = |s: &str| s.parse::<u64>().map_err(|_| Error::InvalidArgs);
        Ok(Self {
            seq: num(f[0])?,
            time: f[1].parse().map_err(|_| Error::InvalidArgs)?,
            session: num(f[2])?,
            step: f[3].parse().map_err(|_| Error::InvalidArgs)?,
            kind: Kind::parse(f[4])?,
            what: unesc(f[5]),
            input: unesc(f[6]),
            decision: unesc(f[7]),
            result: unesc(f[8]),
        })
    }
}

/// FNV-1a 64 obsahu (identifikace zapsanych dat v auditu bez obsahu).
pub fn content_hash(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// Souhrn vstupu nastroje pro audit: retezce zkracene, obsah (`content`)
/// nahrazeny delkou a hashem.
pub fn summarize_input(input: &crate::json::Value) -> String {
    use crate::json::Value;
    let mut out = String::new();
    if let Value::Object(items) = input {
        for (k, v) in items {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(k);
            out.push('=');
            match v {
                Value::Str(s) if k == "content" => {
                    out.push_str(&alloc::format!("<{} B, fnv {:016x}>", s.len(), content_hash(s.as_bytes())));
                }
                Value::Str(s) => out.extend(s.chars().take(120)),
                other => out.extend(other.to_json().chars().take(120)),
            }
        }
    }
    out
}
