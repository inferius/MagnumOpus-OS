//! Jazyk shellu MagnumOpus: jeden jazyk, dva zapisy.
//!
//! - **B** (prikazovy, vychozi): `ls ~/Dok -r | where size > 100MB`
//! - **C** (vyrazovy, blizky Rustu): `ls("~/Dok", recursive: true).where(it => it.size > 100MB)`
//!
//! Oba parsery ([`parse`]) vytvari stejny strom ([`ast`]); formatovac
//! ([`fmt`]) umi strom zapsat v obou zapisech a interpret ([`eval`]) ho
//! vyhodnoti. Nic z toho nema vlastni chovani podle zapisu. Prostredi
//! (soubory, programy, cas, preruseni) dodava hostitel pres [`host::Host`],
//! takze knihovna bezi v nasem systemu (`no_std` + `alloc`) i v testech na
//! hostiteli. Navrh a gramatika: `docs/cs/navrh-shell.md`.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod ast;
pub mod complete;
pub mod doc;
pub mod error;
pub mod eval;
pub mod explain;
pub mod filekind;
pub mod fmt;
pub mod highlight;
pub mod history;
pub mod host;
pub mod json;
pub mod lex;
pub mod parse;
pub mod sig;
pub mod span;
pub mod stack;
pub mod table;
pub mod value;
pub mod vfs;

#[cfg(test)]
mod tests;

pub use error::ShError;
pub use parse::{Options, Parsed, Style, parse};
pub use value::Value;

/// Prevede zdroj do druheho zapisu (`fmt --style`). Vrati text a varovani
/// parseru.
pub fn convert(src: &str, from: Option<Style>, to: Style, sigs: &dyn sig::Sigs) -> Result<alloc::string::String, ShError> {
    let parsed = parse(src, &Options { style: from, sigs, vars: alloc::vec::Vec::new(), script: false })?;
    Ok(fmt::format(&parsed.script, &fmt::FmtOptions { style: to, sigs, vars: alloc::vec::Vec::new() }))
}
