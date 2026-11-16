//! Hola cesta jako prikaz (`/`, `./x`, `../`, `~`, `~/Dokumenty`, `.`,
//! `..`): akce podle druhu cile.
//!
//! - slozka = `cd` (bez argumentu),
//! - skript `.mos` = `run` (s kontrolou manifestu `needs`),
//! - program (ELF) = spustit jako kazdy program (stejna kontrola prav,
//!   nahled, undo); argumenty dostane program,
//! - jiny soubor = aplikace podle typu (sluzba "otevri cestu", viz
//!   `builtins::start`); bez aplikace nebo bez sluzby souhrn jako `open`,
//! - neexistujici cesta = chyba s radou.

use alloc::string::String;
use alloc::vec::Vec;

use super::call::Args;
use super::{Flow, Interp, R};
use crate::ast::Arg;
use crate::error::{ShError, code};
use crate::host::{Host, NodeKind};
use crate::span::Span;
use crate::value::Value;

/// Je jmeno prikazu holou cestou?
pub(crate) fn is_bare_path(name: &str) -> bool {
    name == "." || name == ".." || name == "~" || ["/", "./", "../", "~/"].iter().any(|p| name.starts_with(p))
}

impl Interp {
    /// Hola cesta `name` s argumenty `args` (viz modul).
    pub(crate) fn bare_path(&mut self, host: &mut dyn Host, name: &str, args: &[Arg], input: Value, span: Span) -> R<Value> {
        let path = self.abs(name);
        let meta = match self.fs_stat(host, &path) {
            Ok(m) => m,
            Err(Flow::Err(e)) if e.code == "NOT_FOUND" => {
                return Err(Flow::Err(
                    ShError::new("NOT_FOUND", alloc::format!("{name} neexistuje"))
                        .at(span)
                        .hint("holá cesta otevře složku (cd), spustí skript .mos nebo program; co ve složce je, ukáže ls")
                        .caused_by(e),
                ));
            }
            Err(e) => return Err(e),
        };
        let simple = |what: &str| -> R<()> {
            if args.is_empty() {
                Ok(())
            } else {
                Err(Flow::Err(ShError::new(code::ARGS, alloc::format!("{what} nebere argumenty")).at(span)))
            }
        };
        let one = |it: &mut Interp, host: &mut dyn Host, cmd: &str| -> R<Value> {
            let a = Args { pos: alloc::vec![Value::Str(String::from(name))], named: Vec::new(), span, order: Vec::new() };
            super::builtins::call(it, host, cmd, a, Value::Nothing)
        };
        match meta.kind {
            NodeKind::Dir => {
                simple("složka")?;
                one(self, host, "cd")
            }
            _ if name.to_lowercase().ends_with(".mos") => {
                simple("skript .mos")?;
                one(self, host, "run")
            }
            _ => {
                let head = self.fs_read_range(host, &path, 0, 4)?;
                if head.starts_with(b"\x7fELF") {
                    // Program: stejna cesta jako jmeno programu (prava
                    // z manifestu, nahled, undo).
                    return self.call_program(host, name, args, input, span);
                }
                simple("soubor")?;
                if super::builtins::start::bare_file(host, &path, span)? {
                    return Ok(Value::Nothing);
                }
                one(self, host, "open")
            }
        }
    }
}
