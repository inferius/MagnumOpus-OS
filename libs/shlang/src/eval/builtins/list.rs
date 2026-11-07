//! Seznamy a tabulky: where, each, select, sort, first, last, count, sum,
//! group, reverse, range.

use alloc::string::String;
use alloc::vec::Vec;

use super::super::call::Args;
use super::super::{Flow, Interp, R, field, ops};
use super::{int_arg, list_input, str_arg};
use crate::error::{ShError, code};
use crate::host::Host;
use crate::value::Value;

pub(super) fn call(it: &mut Interp, host: &mut dyn Host, name: &str, a: Args, input: Value) -> R<Value> {
    match name {
        "where" => {
            let f = a.pos(0).cloned().ok_or_else(|| a.err(code::ARGS, "where: chybi podminka"))?;
            let items = list_input(&a, "where", input)?;
            let mut out = Vec::new();
            for x in items {
                if host.interrupted() {
                    return Err(Flow::Err(ShError::new(code::INTERRUPTED, "preruseno (Ctrl-C)")));
                }
                match it.call_value(host, &f, alloc::vec![x.clone()])? {
                    Value::Bool(true) => out.push(x),
                    Value::Bool(false) => {}
                    other => {
                        return Err(a.err(
                            code::TYPE,
                            alloc::format!("podminka where musi vratit bool, vratila {}", other.type_name()),
                        ));
                    }
                }
            }
            Ok(Value::List(out))
        }
        "each" => {
            let f = a.pos(0).cloned().ok_or_else(|| a.err(code::ARGS, "each: chybi funkce"))?;
            let items = list_input(&a, "each", input)?;
            let mut out = Vec::with_capacity(items.len());
            for x in items {
                if host.interrupted() {
                    return Err(Flow::Err(ShError::new(code::INTERRUPTED, "preruseno (Ctrl-C)")));
                }
                let v = it.call_value(host, &f, alloc::vec![x])?;
                if !matches!(v, Value::Nothing) {
                    out.push(v);
                }
            }
            Ok(Value::List(out))
        }
        "select" => {
            let mut cols = Vec::new();
            for i in 0..a.pos.len() {
                cols.push(str_arg(&a, i, "sloupec")?);
            }
            let pick = |v: &Value| -> R<Value> {
                let mut r = Vec::new();
                for c in &cols {
                    r.push((c.clone(), v.field(c).unwrap_or(Value::Null)));
                }
                Ok(Value::Record(r))
            };
            match input {
                Value::Record(_) => pick(&input),
                other => {
                    let items = list_input(&a, "select", other)?;
                    Ok(Value::List(items.iter().map(&pick).collect::<R<Vec<_>>>()?))
                }
            }
        }
        "sort" => {
            let by = match a.pos(0) {
                Some(_) => Some(str_arg(&a, 0, "sloupec")?),
                None => None,
            };
            let items = list_input(&a, "sort", input)?;
            let mut keyed = Vec::with_capacity(items.len());
            for x in items {
                let k = match &by {
                    Some(c) => field(&x, c).map_err(|e| Flow::Err(e.at(a.span)))?,
                    None => x.clone(),
                };
                keyed.push((k, x));
            }
            // Stabilni razeni vkladanim do serazene posloupnosti (chyba
            // porovnani se ohlasi, ne ignoruje).
            let mut err: Option<ShError> = None;
            keyed.sort_by(|p, q| match ops::compare(&p.0, &q.0) {
                Ok(o) => o,
                Err(e) => {
                    err.get_or_insert(e);
                    core::cmp::Ordering::Equal
                }
            });
            if let Some(e) = err {
                return Err(Flow::Err(e.at(a.span)));
            }
            let mut out: Vec<Value> = keyed.into_iter().map(|(_, x)| x).collect();
            if a.flag("desc") {
                out.reverse();
            }
            Ok(Value::List(out))
        }
        "first" | "last" => {
            let n = int_arg(&a, 0, "pocet")?;
            let items = list_input(&a, name, input)?;
            match n {
                None => Ok((if name == "first" { items.first() } else { items.last() }).cloned().unwrap_or(Value::Null)),
                Some(n) if n < 0 => Err(a.err(code::ARGS, "pocet nesmi byt zaporny")),
                Some(n) => {
                    let n = (n as usize).min(items.len());
                    Ok(Value::List(if name == "first" {
                        items[..n].to_vec()
                    } else {
                        items[items.len() - n..].to_vec()
                    }))
                }
            }
        }
        "count" => Ok(Value::Int(match &input {
            Value::List(v) => v.len(),
            Value::Str(s) => s.chars().count(),
            Value::Bytes(b) => b.len(),
            Value::Record(r) => r.len(),
            other => return Err(a.err(code::TYPE, alloc::format!("count nejde na {}", other.type_name()))),
        } as i64)),
        "sum" => {
            let items = list_input(&a, "sum", input)?;
            let mut acc = Value::Int(0);
            for (i, x) in items.into_iter().enumerate() {
                let v = match a.pos(0) {
                    Some(_) => field(&x, &str_arg(&a, 0, "sloupec")?).map_err(|e| Flow::Err(e.at(a.span)))?,
                    None => x,
                };
                acc = if i == 0 { v } else { ops::binary(crate::ast::BinOp::Add, acc, v).map_err(|e| Flow::Err(e.at(a.span)))? };
            }
            Ok(acc)
        }
        "group" => {
            let by = str_arg(&a, 0, "sloupec")?;
            let items = list_input(&a, "group", input)?;
            let mut groups: Vec<(Value, Vec<Value>)> = Vec::new();
            for x in items {
                let k = field(&x, &by).map_err(|e| Flow::Err(e.at(a.span)))?;
                match groups.iter_mut().find(|(g, _)| *g == k) {
                    Some((_, v)) => v.push(x),
                    None => groups.push((k, alloc::vec![x])),
                }
            }
            Ok(Value::List(
                groups.into_iter().map(|(k, v)| Value::record(&[("key", k), ("items", Value::List(v))])).collect(),
            ))
        }
        "reverse" => match input {
            Value::Str(s) => Ok(Value::Str(s.chars().rev().collect::<String>())),
            other => {
                let mut v = list_input(&a, "reverse", other)?;
                v.reverse();
                Ok(Value::List(v))
            }
        },
        "range" => {
            let from = int_arg(&a, 0, "od")?.unwrap_or(0);
            let to = int_arg(&a, 1, "do")?.unwrap_or(0);
            if to.saturating_sub(from) > 10_000_000 {
                return Err(a.err(code::ARGS, "range: prilis mnoho cisel (nejvys 10 milionu)"));
            }
            Ok(Value::List((from..to).map(Value::Int).collect()))
        }
        _ => Err(a.err(code::UNKNOWN, "neznamy prikaz")),
    }
}
