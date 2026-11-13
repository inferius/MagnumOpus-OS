//! JSON <-> hodnoty (`from json`, `to json`).
//!
//! Velikost se zapise jako pocet bajtu, doba jako nanosekundy, cas jako
//! text `RRRR-MM-DD hh:mm:ss`. Bajty a closure do JSON nejdou.

use alloc::string::String;
use alloc::vec::Vec;

use crate::error::{ShError, code};
use crate::value::{Value, format_time};

pub fn parse(src: &str) -> Result<Value, ShError> {
    let mut p = P { s: src.as_bytes(), i: 0, depth: 0 };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(p.err("za hodnotou JSON je něco navíc"));
    }
    Ok(v)
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
}

impl P<'_> {
    fn err(&self, m: &str) -> ShError {
        ShError::new(code::FORMAT, alloc::format!("neplatný JSON na pozici {}: {m}", self.i))
    }

    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\n' | b'\r' | b'\t') {
            self.i += 1;
        }
    }

    fn lit(&mut self, w: &str, v: Value) -> Result<Value, ShError> {
        if self.s[self.i..].starts_with(w.as_bytes()) {
            self.i += w.len();
            Ok(v)
        } else {
            Err(self.err("neznámá hodnota"))
        }
    }

    fn value(&mut self) -> Result<Value, ShError> {
        self.depth += 1;
        if self.depth > 128 {
            return Err(self.err("příliš hluboké vnoření"));
        }
        let r = match self.s.get(self.i) {
            None => Err(self.err("chybí hodnota")),
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => self.string().map(Value::Str),
            Some(b't') => self.lit("true", Value::Bool(true)),
            Some(b'f') => self.lit("false", Value::Bool(false)),
            Some(b'n') => self.lit("null", Value::Null),
            Some(_) => self.number(),
        };
        self.depth -= 1;
        r
    }

    fn object(&mut self) -> Result<Value, ShError> {
        self.i += 1;
        let mut r = Vec::new();
        self.ws();
        if self.s.get(self.i) == Some(&b'}') {
            self.i += 1;
            return Ok(Value::Record(r));
        }
        loop {
            self.ws();
            if self.s.get(self.i) != Some(&b'"') {
                return Err(self.err("čekám klíč v uvozovkách"));
            }
            let k = self.string()?;
            self.ws();
            if self.s.get(self.i) != Some(&b':') {
                return Err(self.err("čekám ':'"));
            }
            self.i += 1;
            self.ws();
            let v = self.value()?;
            r.push((k, v));
            self.ws();
            match self.s.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Value::Record(r));
                }
                _ => return Err(self.err("čekám ',' nebo '}'")),
            }
        }
    }

    fn array(&mut self) -> Result<Value, ShError> {
        self.i += 1;
        let mut r = Vec::new();
        self.ws();
        if self.s.get(self.i) == Some(&b']') {
            self.i += 1;
            return Ok(Value::List(r));
        }
        loop {
            self.ws();
            r.push(self.value()?);
            self.ws();
            match self.s.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Value::List(r));
                }
                _ => return Err(self.err("čekám ',' nebo ']'")),
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, ShError> {
        let h = self.s.get(self.i..self.i + 4).ok_or_else(|| self.err("krátká \\u sekvence"))?;
        let t = core::str::from_utf8(h).map_err(|_| self.err("neplatná \\u sekvence"))?;
        let v = u32::from_str_radix(t, 16).map_err(|_| self.err("neplatná \\u sekvence"))?;
        self.i += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, ShError> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let start = self.i;
            while self.i < self.s.len() && self.s[self.i] != b'"' && self.s[self.i] != b'\\' {
                self.i += 1;
            }
            out.push_str(core::str::from_utf8(&self.s[start..self.i]).map_err(|_| self.err("neplatné UTF-8"))?);
            match self.s.get(self.i) {
                None => return Err(self.err("neukončený řetězec")),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                _ => {
                    self.i += 1;
                    let e = *self.s.get(self.i).ok_or_else(|| self.err("neukončený řetězec"))?;
                    self.i += 1;
                    match e {
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'/' => out.push('/'),
                        b'\\' => out.push('\\'),
                        b'"' => out.push('"'),
                        b'u' => {
                            let mut c = self.hex4()?;
                            if (0xD800..0xDC00).contains(&c) && self.s[self.i..].starts_with(b"\\u") {
                                self.i += 2;
                                let lo = self.hex4()?;
                                c = 0x10000 + ((c - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            out.push(char::from_u32(c).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err(self.err("neznámá escape sekvence")),
                    }
                }
            }
        }
    }

    fn number(&mut self) -> Result<Value, ShError> {
        let start = self.i;
        while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
            self.i += 1;
        }
        let t = core::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
        if t.is_empty() {
            return Err(self.err("neočekávaný znak"));
        }
        if !t.contains(['.', 'e', 'E'])
            && let Ok(i) = t.parse::<i64>()
        {
            return Ok(Value::Int(i));
        }
        t.parse::<f64>().map(Value::Float).map_err(|_| self.err("neplatné číslo"))
    }
}

fn esc(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&alloc::format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Hodnota -> JSON (`pretty` = odsazene).
pub fn to_json(v: &Value, pretty: bool) -> Result<String, ShError> {
    let mut out = String::new();
    write(&mut out, v, pretty, 0)?;
    Ok(out)
}

fn indent(out: &mut String, n: usize) {
    out.push('\n');
    for _ in 0..n {
        out.push_str("  ");
    }
}

fn write(out: &mut String, v: &Value, pretty: bool, lvl: usize) -> Result<(), ShError> {
    match v {
        Value::Nothing | Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) | Value::Size(i) | Value::Duration(i) => out.push_str(&alloc::format!("{i}")),
        Value::Float(f) => {
            if f.is_finite() {
                out.push_str(&crate::fmt::num_text(*f))
            } else {
                out.push_str("null")
            }
        }
        Value::Time(t) => esc(out, &format_time(*t)),
        Value::Str(s) => esc(out, s),
        Value::List(items) => {
            out.push('[');
            for (i, x) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if pretty {
                    indent(out, lvl + 1);
                }
                write(out, x, pretty, lvl + 1)?;
            }
            if pretty && !items.is_empty() {
                indent(out, lvl);
            }
            out.push(']');
        }
        Value::Record(r) => {
            out.push('{');
            for (i, (k, x)) in r.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if pretty {
                    indent(out, lvl + 1);
                }
                esc(out, k);
                out.push(':');
                if pretty {
                    out.push(' ');
                }
                write(out, x, pretty, lvl + 1)?;
            }
            if pretty && !r.is_empty() {
                indent(out, lvl);
            }
            out.push('}');
        }
        Value::Error(e) => {
            out.push_str("{\"code\":");
            esc(out, &e.code);
            out.push_str(",\"message\":");
            esc(out, &e.message);
            out.push('}');
        }
        Value::Bytes(_) | Value::Closure(_) => {
            return Err(ShError::new(code::TYPE, alloc::format!("hodnotu typu {} nejde zapsat do JSON", v.type_name()))
                .hint("bajty nejdřív převeďte: from text / from json"));
        }
    }
    Ok(())
}
