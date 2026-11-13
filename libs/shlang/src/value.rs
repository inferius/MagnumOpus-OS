//! Hodnoty jazyka.
//!
//! Bajtovy proud (`Bytes`) je jiny typ nez text a seznam zaznamu; mezi nimi
//! se prevadi jen vyslovne (`from json`, `to text`, `lines`). Zadne skryte
//! prevody: jedina vyjimka je cele cislo -> desetinne v aritmetice.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use crate::ast::Expr;
use crate::error::ShError;

/// Zaznam: pole v poradi vlozeni.
pub type Record = Vec<(String, Value)>;

/// Closure s promennymi zachycenymi pri vytvoreni (kopie hodnot).
#[derive(Debug, PartialEq)]
pub struct ClosureVal {
    pub params: Vec<String>,
    pub body: Expr,
    pub captured: Vec<(String, Value)>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Value {
    /// Zadna hodnota (prikaz nic nevraci; terminal nic nevypise).
    #[default]
    Nothing,
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// Velikost v bajtech.
    Size(i64),
    /// Doba v nanosekundach.
    Duration(i64),
    /// Okamzik: nanosekundy od 1970-01-01 UTC.
    Time(i64),
    Str(String),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Record(Record),
    Closure(Arc<ClosureVal>),
    Error(Box<ShError>),
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Nothing => "nothing",
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Size(_) => "size",
            Value::Duration(_) => "duration",
            Value::Time(_) => "time",
            Value::Str(_) => "text",
            Value::Bytes(_) => "bytes",
            Value::List(_) => "list",
            Value::Record(_) => "record",
            Value::Closure(_) => "closure",
            Value::Error(_) => "error",
        }
    }

    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(s.into())
    }

    /// Pole zaznamu (u chyby pole `code`, `message`, `line`, `hint`, `cause`).
    pub fn field(&self, name: &str) -> Option<Value> {
        match self {
            Value::Record(r) => r.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()),
            Value::Error(e) => error_field(e, name),
            _ => None,
        }
    }

    /// Jmena poli (pro hlaseni "zaznam ma: ...").
    pub fn field_names(&self) -> Vec<String> {
        match self {
            Value::Record(r) => r.iter().map(|(k, _)| k.clone()).collect(),
            Value::Error(_) => ["code", "message", "hint", "cause", "line", "column"].iter().map(|s| s.to_string()).collect(),
            _ => Vec::new(),
        }
    }

    /// Text pro vlozeni do retezce a pro `echo` (bez uvozovek).
    pub fn to_text(&self) -> String {
        alloc::format!("{self}")
    }

    pub fn record(fields: &[(&str, Value)]) -> Value {
        Value::Record(fields.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
    }
}

fn error_field(e: &ShError, name: &str) -> Option<Value> {
    Some(match name {
        "code" => Value::str(e.code.clone()),
        "message" => Value::str(e.message.clone()),
        "hint" => e.hint.clone().map(Value::Str).unwrap_or(Value::Null),
        "cause" => e.cause.clone().map(Value::Error).unwrap_or(Value::Null),
        "start" => e.span.map(|s| Value::Int(s.start as i64)).unwrap_or(Value::Null),
        _ => return None,
    })
}

/// Lidsky citelna velikost (desitkove jednotky, jedno desetinne misto).
pub fn human_size(b: i64) -> String {
    let neg = b < 0;
    let a = b.unsigned_abs();
    let units = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut i = 0;
    let mut v = a as f64;
    while v >= 1000.0 && i + 1 < units.len() {
        v /= 1000.0;
        i += 1;
    }
    let sign = if neg { "-" } else { "" };
    if i == 0 {
        alloc::format!("{sign}{a} B")
    } else {
        alloc::format!("{sign}{:.1} {}", v, units[i])
    }
}

/// Lidsky citelna doba.
pub fn human_duration(ns: i64) -> String {
    let neg = ns < 0;
    let a = ns.unsigned_abs();
    let sign = if neg { "-" } else { "" };
    const S: u64 = 1_000_000_000;
    if a < 1000 {
        return alloc::format!("{sign}{a}ns");
    }
    if a < 1_000_000 {
        return alloc::format!("{sign}{}us", a / 1000);
    }
    if a < S {
        return alloc::format!("{sign}{}ms", a / 1_000_000);
    }
    let secs = a / S;
    if secs < 60 {
        let ms = (a % S) / 1_000_000;
        return if ms == 0 { alloc::format!("{sign}{secs}s") } else { alloc::format!("{sign}{secs}.{ms:03}s") };
    }
    let (d, h, m, s) = (secs / 86400, secs / 3600 % 24, secs / 60 % 60, secs % 60);
    let mut out = String::from(sign);
    if d > 0 {
        out.push_str(&alloc::format!("{d}d "));
    }
    if d > 0 || h > 0 {
        out.push_str(&alloc::format!("{h}h "));
    }
    out.push_str(&alloc::format!("{m}min"));
    if s > 0 {
        out.push_str(&alloc::format!(" {s}s"));
    }
    out
}

/// Cas jako `RRRR-MM-DD hh:mm:ss` (UTC).
pub fn format_time(ns: i64) -> String {
    let t = abi::time::WallTime::new(ns.div_euclid(1_000_000_000), ns.rem_euclid(1_000_000_000));
    let c = t.to_civil();
    alloc::format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        c.year, c.month, c.day, c.hour, c.minute, c.second
    )
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Nothing => Ok(()),
            Value::Null => f.write_str("null"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => f.write_str(&crate::fmt::num_text(*x)),
            Value::Size(b) => f.write_str(&human_size(*b)),
            Value::Duration(d) => f.write_str(&human_duration(*d)),
            Value::Time(t) => f.write_str(&format_time(*t)),
            Value::Str(s) => f.write_str(s),
            Value::Bytes(b) => match core::str::from_utf8(b) {
                Ok(s) if b.len() <= 64 => write!(f, "<{} B: {s:?}>", b.len()),
                _ => write!(f, "<{} B>", b.len()),
            },
            Value::List(items) => {
                f.write_str("[")?;
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write_inner(f, v)?;
                }
                f.write_str("]")
            }
            Value::Record(r) => {
                f.write_str("{")?;
                for (i, (k, v)) in r.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{k}: ")?;
                    write_inner(f, v)?;
                }
                f.write_str("}")
            }
            Value::Closure(c) => write!(f, "<closure({})>", c.params.join(", ")),
            Value::Error(e) => write!(f, "<chyba {}: {}>", e.code, e.message),
        }
    }
}

/// Uvnitr seznamu/zaznamu text v uvozovkach.
fn write_inner(f: &mut fmt::Formatter<'_>, v: &Value) -> fmt::Result {
    match v {
        Value::Str(s) => write!(f, "{s:?}"),
        other => write!(f, "{other}"),
    }
}
