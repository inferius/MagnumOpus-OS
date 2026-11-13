//! Vestavene prikazy (podpisy v [`crate::sig::BUILTINS`]).

mod audio;
mod convert;
mod fs;
mod install;
mod list;
mod misc;
pub(crate) mod start;
#[cfg(test)]
pub(crate) use misc::help_overview;
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
        "audio" => audio::call(it, host, a),
        "start" => start::call(it, host, a),
        "convert" => convert::call(it, host, a, input),
        "install" => install::run(it, host, &a),
        _ => misc::call(it, host, name, a, input),
    }
}

/// Vstup roury musi byt seznam.
pub(crate) fn list_input(a: &Args, cmd: &str, input: Value) -> R<alloc::vec::Vec<Value>> {
    match input {
        Value::List(v) => Ok(v),
        Value::Bytes(_) => Err(super::Flow::Err(
            crate::ShError::new(code::TYPE, alloc::format!("{cmd} čeká seznam, dostal bajty"))
                .at(a.span)
                .hint("bajty převeďte výslovně: from json, lines, from text"),
        )),
        Value::Str(_) => Err(super::Flow::Err(
            crate::ShError::new(code::TYPE, alloc::format!("{cmd} čeká seznam, dostal text"))
                .at(a.span)
                .hint("text rozdělte: lines nebo split \",\""),
        )),
        other => Err(a.err(code::TYPE, alloc::format!("{cmd} čeká seznam, dostal {}", other.type_name()))),
    }
}

/// Argument jako text.
pub(crate) fn str_arg(a: &Args, i: usize, what: &str) -> R<alloc::string::String> {
    match a.pos(i) {
        Some(Value::Str(s)) => Ok(s.clone()),
        Some(other) => Err(a.err(code::TYPE, alloc::format!("{what}: čekám text, dostal jsem {}", other.type_name()))),
        None => Err(a.err(code::ARGS, alloc::format!("chybí argument {what}"))),
    }
}

/// Argument jako cele cislo.
pub(crate) fn int_arg(a: &Args, i: usize, what: &str) -> R<Option<i64>> {
    match a.pos(i) {
        Some(Value::Int(n)) => Ok(Some(*n)),
        Some(other) => Err(a.err(code::TYPE, alloc::format!("{what}: čekám celé číslo, dostal jsem {}", other.type_name()))),
        None => Ok(None),
    }
}
