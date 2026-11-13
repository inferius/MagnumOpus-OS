//! Text a prevody: echo, log, lines, split, join, trim, upper, lower,
//! contains, starts-with, ends-with, replace, from, to, describe.
//!
//! Prevody mezi bajty, textem a zaznamy jsou jen vyslovne (`from`, `to`).

use alloc::string::String;
use alloc::vec::Vec;

use super::super::call::Args;
use super::super::{Flow, Interp, R, ops};
use super::str_arg;
use crate::error::{ShError, code};
use crate::host::Host;
use crate::value::Value;

/// Vstup jako text (bajty jen kdyz jde o platne UTF-8 - u prikazu, ktere
/// text cekaji, je to vyslovny zamer, ne skryty prevod).
fn text_input(a: &Args, cmd: &str, input: Value) -> R<String> {
    match input {
        Value::Str(s) => Ok(s),
        Value::Bytes(b) => String::from_utf8(b).map_err(|_| {
            Flow::Err(ShError::new(code::FORMAT, alloc::format!("{cmd}: data nejsou text v UTF-8")).at(a.span))
        }),
        other => Err(a.err(code::TYPE, alloc::format!("{cmd} čeká text, dostal {}", other.type_name()))),
    }
}

pub(super) fn call(_it: &mut Interp, host: &mut dyn Host, name: &str, a: Args, input: Value) -> R<Value> {
    match name {
        "echo" => {
            let parts: Vec<String> = a.pos.iter().map(|v| v.to_text()).collect();
            Ok(Value::Str(parts.join(" ")))
        }
        "log" => {
            let parts: Vec<String> = a.pos.iter().map(|v| v.to_text()).collect();
            host.note(&parts.join(" "));
            Ok(Value::Nothing)
        }
        "lines" => {
            let s = text_input(&a, "lines", input)?;
            Ok(Value::List(s.lines().map(|l| Value::Str(l.into())).collect()))
        }
        "split" => {
            let s = text_input(&a, "split", input)?;
            let sep = str_arg(&a, 0, "oddělovač")?;
            if sep.is_empty() {
                return Ok(Value::List(s.chars().map(|c| Value::Str(c.into())).collect()));
            }
            Ok(Value::List(s.split(sep.as_str()).map(|p| Value::Str(p.into())).collect()))
        }
        "join" => {
            let sep = match a.pos(0) {
                Some(_) => str_arg(&a, 0, "oddělovač")?,
                None => String::new(),
            };
            let items = super::list_input(&a, "join", input)?;
            let parts: Vec<String> = items.iter().map(|v| v.to_text()).collect();
            Ok(Value::Str(parts.join(&sep)))
        }
        "trim" => Ok(Value::Str(text_input(&a, name, input)?.trim().into())),
        "upper" => Ok(Value::Str(text_input(&a, name, input)?.to_uppercase())),
        "lower" => Ok(Value::Str(text_input(&a, name, input)?.to_lowercase())),
        "contains" => {
            let part = a.pos(0).cloned().unwrap_or(Value::Null);
            match (&input, &part) {
                (Value::Str(s), Value::Str(p)) => Ok(Value::Bool(s.contains(p.as_str()))),
                (Value::List(items), p) => {
                    for x in items {
                        if ops::equal(x, p).unwrap_or(false) {
                            return Ok(Value::Bool(true));
                        }
                    }
                    Ok(Value::Bool(false))
                }
                _ => Err(a.err(
                    code::TYPE,
                    alloc::format!("contains: {} nejde hledat v {}", part.type_name(), input.type_name()),
                )),
            }
        }
        "starts-with" | "ends-with" => {
            let s = text_input(&a, name, input)?;
            let p = str_arg(&a, 0, "text")?;
            Ok(Value::Bool(if name == "starts-with" { s.starts_with(p.as_str()) } else { s.ends_with(p.as_str()) }))
        }
        "replace" => {
            let s = text_input(&a, name, input)?;
            let from = str_arg(&a, 0, "co")?;
            let to = str_arg(&a, 1, "čím")?;
            if from.is_empty() {
                return Err(a.err(code::ARGS, "replace: prázdný hledaný text"));
            }
            Ok(Value::Str(s.replace(from.as_str(), &to)))
        }
        "from" => {
            let f = str_arg(&a, 0, "formát")?;
            match f.as_str() {
                "json" => {
                    let s = text_input(&a, "from json", input)?;
                    crate::json::parse(&s).map_err(|e| Flow::Err(e.at(a.span)))
                }
                "text" => Ok(Value::Str(text_input(&a, "from text", input)?)),
                "lines" => {
                    let s = text_input(&a, "from lines", input)?;
                    Ok(Value::List(s.lines().map(|l| Value::Str(l.into())).collect()))
                }
                other => Err(Flow::Err(
                    ShError::new(code::ARGS, alloc::format!("neznámý formát '{other}'")).at(a.span).hint("formáty: json, text, lines"),
                )),
            }
        }
        "to" => {
            let f = str_arg(&a, 0, "formát")?;
            match f.as_str() {
                "json" => crate::json::to_json(&input, true).map(Value::Str).map_err(|e| Flow::Err(e.at(a.span))),
                "text" => match input {
                    Value::Bytes(_) => Err(Flow::Err(
                        ShError::new(code::TYPE, "to text: bajty nejsou hodnoty").at(a.span).hint("bajty -> text: from text"),
                    )),
                    other => Ok(Value::Str(crate::table::render(&other))),
                },
                other => Err(Flow::Err(
                    ShError::new(code::ARGS, alloc::format!("neznámý formát '{other}'")).at(a.span).hint("formáty: json, text"),
                )),
            }
        }
        "describe" => Ok(Value::str(input.type_name())),
        _ => Err(a.err(code::UNKNOWN, "neznámý příkaz")),
    }
}
