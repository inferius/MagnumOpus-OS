//! Maly JSON pro rozhrani modelu (bez `std`).
//!
//! Objekt zachovava poradi klicu: telo pozadavku na model je tak
//! deterministicke (stejne bajty = funguje cache promptu poskytovatele)
//! a testy muzou porovnavat text. Cisla jsou cela (`Int`) nebo s desetinnou
//! carkou (`Float`); cele cislo mimo `i64` je `Float`.
//!
//! Ochrana pameti: hloubka vnoreni nejvys [`MAX_DEPTH`], vstup od modelu
//! nebo ze site se nikdy nerozbali do nekonecna.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

/// Nejvetsi hloubka vnoreni pri cteni.
pub const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

/// Chyba cteni: pozice v bajtech a popis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub pos: usize,
    pub msg: &'static str,
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "neplatny JSON na pozici {}: {}", self.pos, self.msg)
    }
}

impl Value {
    pub fn str(s: &str) -> Self {
        Value::Str(String::from(s))
    }

    /// Prazdny objekt.
    pub fn object() -> Self {
        Value::Object(Vec::new())
    }

    /// Prida (nebo nahradi) klic objektu; na jinem typu nic.
    pub fn set(&mut self, key: &str, v: Value) -> &mut Self {
        if let Value::Object(items) = self {
            if let Some(slot) = items.iter_mut().find(|(k, _)| k == key) {
                slot.1 = v;
            } else {
                items.push((String::from(key), v));
            }
        }
        self
    }

    /// Stavitel: objekt s klicem navic.
    pub fn with(mut self, key: &str, v: Value) -> Self {
        self.set(key, v);
        self
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(items) => items.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Float(f) if *f == (*f as i64) as f64 => Some(*f as i64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Retezec klice `key` (prazdny, kdyz chybi).
    pub fn str_of(&self, key: &str) -> &str {
        self.get(key).and_then(Value::as_str).unwrap_or("")
    }

    /// Cislo klice `key` (0, kdyz chybi).
    pub fn u64_of(&self, key: &str) -> u64 {
        self.get(key).and_then(Value::as_i64).map_or(0, |v| v.max(0) as u64)
    }

    /// Jmeno typu (pro hlaseni overeni schematu).
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Int(_) => "integer",
            Value::Float(_) => "number",
            Value::Str(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }

    /// Kompaktni zapis (bez mezer).
    pub fn to_json(&self) -> String {
        let mut s = String::new();
        write_value(&mut s, self);
        s
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::str(s)
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Str(s)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::Int(i)
    }
}

impl From<u64> for Value {
    fn from(i: u64) -> Self {
        if i <= i64::MAX as u64 { Value::Int(i as i64) } else { Value::Float(i as f64) }
    }
}

/// Zapise retezec jako JSON (s uvozovkami a escapovanim).
pub fn write_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_value(out: &mut String, v: &Value) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Value::Float(f) => {
            if f.is_finite() {
                let _ = write!(out, "{f}");
            } else {
                out.push_str("null");
            }
        }
        Value::Str(s) => write_str(out, s),
        Value::Array(items) => {
            out.push('[');
            for (i, x) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, x);
            }
            out.push(']');
        }
        Value::Object(items) => {
            out.push('{');
            for (i, (k, x)) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_str(out, k);
                out.push(':');
                write_value(out, x);
            }
            out.push('}');
        }
    }
}

/// Precte cely text jako jednu hodnotu (mezery okolo jsou dovolene).
pub fn parse(text: &str) -> Result<Value, ParseError> {
    let mut p = Parser { b: text.as_bytes(), pos: 0 };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.pos != p.b.len() {
        return Err(p.err("text za hodnotou"));
    }
    Ok(v)
}

struct Parser<'a> {
    b: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn err(&self, msg: &'static str) -> ParseError {
        ParseError { pos: self.pos, msg }
    }

    fn ws(&mut self) {
        while let Some(&c) = self.b.get(self.pos) {
            if matches!(c, b' ' | b'\t' | b'\n' | b'\r') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn eat(&mut self, lit: &[u8]) -> bool {
        if self.b[self.pos..].starts_with(lit) {
            self.pos += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, ParseError> {
        if depth > MAX_DEPTH {
            return Err(self.err("prilis hluboke vnoreni"));
        }
        match self.b.get(self.pos) {
            None => Err(self.err("neocekavany konec")),
            Some(b'n') if self.eat(b"null") => Ok(Value::Null),
            Some(b't') if self.eat(b"true") => Ok(Value::Bool(true)),
            Some(b'f') if self.eat(b"false") => Ok(Value::Bool(false)),
            Some(b'"') => self.string().map(Value::Str),
            Some(b'[') => {
                self.pos += 1;
                let mut items = Vec::new();
                self.ws();
                if self.eat(b"]") {
                    return Ok(Value::Array(items));
                }
                loop {
                    self.ws();
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    if self.eat(b",") {
                        continue;
                    }
                    if self.eat(b"]") {
                        return Ok(Value::Array(items));
                    }
                    return Err(self.err("ocekavano , nebo ]"));
                }
            }
            Some(b'{') => {
                self.pos += 1;
                let mut items: Vec<(String, Value)> = Vec::new();
                self.ws();
                if self.eat(b"}") {
                    return Ok(Value::Object(items));
                }
                loop {
                    self.ws();
                    if self.b.get(self.pos) != Some(&b'"') {
                        return Err(self.err("ocekavan klic"));
                    }
                    let k = self.string()?;
                    self.ws();
                    if !self.eat(b":") {
                        return Err(self.err("ocekavana :"));
                    }
                    self.ws();
                    let v = self.value(depth + 1)?;
                    // Duplicitni klic: plati posledni (jako vetsina parseru).
                    if let Some(slot) = items.iter_mut().find(|(x, _)| *x == k) {
                        slot.1 = v;
                    } else {
                        items.push((k, v));
                    }
                    self.ws();
                    if self.eat(b",") {
                        continue;
                    }
                    if self.eat(b"}") {
                        return Ok(Value::Object(items));
                    }
                    return Err(self.err("ocekavano , nebo }"));
                }
            }
            Some(c) if *c == b'-' || c.is_ascii_digit() => self.number(),
            Some(_) => Err(self.err("neocekavany znak")),
        }
    }

    fn number(&mut self) -> Result<Value, ParseError> {
        let start = self.pos;
        let mut float = false;
        if self.b.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        let digits = |p: &mut Self| {
            let s = p.pos;
            while p.b.get(p.pos).is_some_and(u8::is_ascii_digit) {
                p.pos += 1;
            }
            p.pos > s
        };
        if !digits(self) {
            return Err(self.err("ocekavana cislice"));
        }
        if self.b.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            float = true;
            if !digits(self) {
                return Err(self.err("ocekavana cislice za teckou"));
            }
        }
        if matches!(self.b.get(self.pos), Some(b'e' | b'E')) {
            self.pos += 1;
            float = true;
            if matches!(self.b.get(self.pos), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !digits(self) {
                return Err(self.err("ocekavan exponent"));
            }
        }
        let text = core::str::from_utf8(&self.b[start..self.pos]).map_err(|_| self.err("neplatne cislo"))?;
        if !float && let Ok(i) = text.parse::<i64>() {
            return Ok(Value::Int(i));
        }
        text.parse::<f64>().map(Value::Float).map_err(|_| self.err("neplatne cislo"))
    }

    fn hex4(&mut self) -> Result<u32, ParseError> {
        let s = self.b.get(self.pos..self.pos + 4).ok_or_else(|| self.err("kratke \\u"))?;
        let s = core::str::from_utf8(s).map_err(|_| self.err("neplatne \\u"))?;
        let v = u32::from_str_radix(s, 16).map_err(|_| self.err("neplatne \\u"))?;
        self.pos += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.pos += 1; // uvozovka
        let mut out = String::new();
        loop {
            // Usek bez escapu najednou (UTF-8 zustava cely).
            let start = self.pos;
            while let Some(&c) = self.b.get(self.pos) {
                if c == b'"' || c == b'\\' || c < 0x20 {
                    break;
                }
                self.pos += 1;
            }
            let chunk = core::str::from_utf8(&self.b[start..self.pos]).map_err(|_| self.err("neplatne UTF-8"))?;
            out.push_str(chunk);
            match self.b.get(self.pos) {
                None => return Err(self.err("neukonceny retezec")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let c = *self.b.get(self.pos).ok_or_else(|| self.err("neukonceny escape"))?;
                    self.pos += 1;
                    match c {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) {
                                // Par nahradnich znaku (UTF-16).
                                if !self.eat(b"\\u") {
                                    return Err(self.err("chybi druha polovina paru"));
                                }
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return Err(self.err("neplatny par"));
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else {
                                hi
                            };
                            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err(self.err("neznamy escape")),
                    }
                }
                Some(_) => return Err(self.err("ridici znak v retezci")),
            }
        }
    }
}
