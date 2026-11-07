//! Vestavene prikazy (podpisy v [`crate::sig::BUILTINS`]).

mod fs;
mod list;
mod misc;
mod text;

use super::call::Args;
use super::{Interp, R};
use crate::error::code;
use crate::host::Host;
use crate::value::Value;

/// Zavola vestaveny prikaz podle kratkeho jmena.
pub(crate) fn call(it: &mut Interp, host: &mut dyn Host, name: &str, a: Args, input: Value) -> R<Value> {
    match name {
        "ls" | "cd" | "pwd" | "open" | "save" | "rm" | "cp" | "mv" | "mkdir" | "exists" | "df" => {
            fs::call(it, host, name, a, input)
        }
        "where" | "each" | "select" | "sort" | "first" | "last" | "count" | "sum" | "group" | "reverse"
        | "range" => list::call(it, host, name, a, input),
        "echo" | "log" | "lines" | "split" | "join" | "trim" | "upper" | "lower" | "contains" | "starts-with"
        | "ends-with" | "replace" | "from" | "to" | "describe" => text::call(it, host, name, a, input),
        _ => misc::call(it, host, name, a, input),
    }
}

/// Vstup roury musi byt seznam.
pub(crate) fn list_input(a: &Args, cmd: &str, input: Value) -> R<alloc::vec::Vec<Value>> {
    match input {
        Value::List(v) => Ok(v),
        Value::Bytes(_) => Err(super::Flow::Err(
            crate::ShError::new(code::TYPE, alloc::format!("{cmd} ceka seznam, dostal bajty"))
                .at(a.span)
                .hint("bajty prevedte vyslovne: from json, lines, from text"),
        )),
        Value::Str(_) => Err(super::Flow::Err(
            crate::ShError::new(code::TYPE, alloc::format!("{cmd} ceka seznam, dostal text"))
                .at(a.span)
                .hint("text rozdelte: lines nebo split \",\""),
        )),
        other => Err(a.err(code::TYPE, alloc::format!("{cmd} ceka seznam, dostal {}", other.type_name()))),
    }
}

/// Argument jako text.
pub(crate) fn str_arg(a: &Args, i: usize, what: &str) -> R<alloc::string::String> {
    match a.pos(i) {
        Some(Value::Str(s)) => Ok(s.clone()),
        Some(other) => Err(a.err(code::TYPE, alloc::format!("{what}: cekam text, dostal jsem {}", other.type_name()))),
        None => Err(a.err(code::ARGS, alloc::format!("chybi argument {what}"))),
    }
}

/// Argument jako cele cislo.
pub(crate) fn int_arg(a: &Args, i: usize, what: &str) -> R<Option<i64>> {
    match a.pos(i) {
        Some(Value::Int(n)) => Ok(Some(*n)),
        Some(other) => Err(a.err(code::TYPE, alloc::format!("{what}: cekam cele cislo, dostal jsem {}", other.type_name()))),
        None => Ok(None),
    }
}
