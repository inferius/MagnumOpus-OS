//! Misto ve zdrojovem textu.
//!
//! Strom nese u kazdeho uzlu misto (pro chybova hlaseni), ale **do rovnosti
//! stromu se misto nepocita**: test prevodu `B -> strom -> C -> strom`
//! porovnava strom z ruznych zdroju, kde se pozice nutne lisi. Proto
//! `Span == Span` je vzdy `true`.

use core::fmt;

/// Rozsah bajtu `start..end` ve zdroji.
#[derive(Clone, Copy, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start: start as u32, end: end as u32 }
    }

    /// Spojeni dvou mist (od zacatku prvniho do konce druheho).
    pub fn to(self, other: Span) -> Span {
        Span { start: self.start.min(other.start), end: self.end.max(other.end) }
    }

    /// Radek a sloupec (od 1) zacatku mista v `src`. Sloupec se pocita ve
    /// znacich, ne v bajtech.
    pub fn line_col(self, src: &str) -> (u32, u32) {
        let upto = &src[..(self.start as usize).min(src.len())];
        let line = upto.matches('\n').count() as u32 + 1;
        let col = match upto.rfind('\n') {
            Some(i) => upto[i + 1..].chars().count(),
            None => upto.chars().count(),
        } as u32
            + 1;
        (line, col)
    }
}

impl PartialEq for Span {
    fn eq(&self, _: &Span) -> bool {
        true
    }
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}..{}", self.start, self.end)
    }
}
