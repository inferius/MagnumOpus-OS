//! Minimalni JSON (RFC 8259) pro seznam OneCRL: objekty, pole, retezce,
//! cisla, `true`/`false`/`null`. Hloubka vnoreni a velikost jsou omezene.

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Self::Object(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(a) => Some(a),
            _ => None,
        }
    }
}

const MAX_DEPTH: usize = 32;

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.pos < self.s.len() && matches!(self.s[self.pos], b' ' | b'\t' | b'\n' | b'\r') {
            self.pos += 1;
        }
    }

    fn eat(&mut self, b: u8) -> Option<()> {
        self.ws();
        (self.s.get(self.pos) == Some(&b)).then(|| self.pos += 1)
    }

    fn lit(&mut self, word: &[u8], v: Value) -> Option<Value> {
        self.s[self.pos..].starts_with(word).then(|| {
            self.pos += word.len();
            v
        })
    }

    fn hex4(&mut self) -> Option<u32> {
        let h = core::str::from_utf8(self.s.get(self.pos..self.pos + 4)?).ok()?;
        self.pos += 4;
        u32::from_str_radix(h, 16).ok()
    }

    fn string(&mut self) -> Option<String> {
        self.eat(b'"')?;
        let mut out = String::new();
        loop {
            let b = *self.s.get(self.pos)?;
            self.pos += 1;
            match b {
                b'"' => return Some(out),
                b'\\' => {
                    let e = *self.s.get(self.pos)?;
                    self.pos += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut c = self.hex4()?;
                            if (0xD800..0xDC00).contains(&c) {
                                // Par nahradnich znaku UTF-16.
                                if self.s.get(self.pos..self.pos + 2)? != b"\\u" {
                                    return None;
                                }
                                self.pos += 2;
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return None;
                                }
                                c = 0x10000 + ((c - 0xD800) << 10) + (lo - 0xDC00);
                            }
                            out.push(char::from_u32(c)?);
                        }
                        _ => return None,
                    }
                }
                0..=0x1F => return None,
                _ => {
                    // Zbytek znaku UTF-8 (vstup je overeny &str).
                    let start = self.pos - 1;
                    let len = match b {
                        0xC0..=0xDF => 2,
                        0xE0..=0xEF => 3,
                        0xF0..=0xF7 => 4,
                        _ => 1,
                    };
                    let chunk = core::str::from_utf8(self.s.get(start..start + len)?).ok()?;
                    out.push_str(chunk);
                    self.pos = start + len;
                }
            }
        }
    }

    fn number(&mut self) -> Option<Value> {
        let start = self.pos;
        while self.pos < self.s.len() && matches!(self.s[self.pos], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
            self.pos += 1;
        }
        let t = core::str::from_utf8(&self.s[start..self.pos]).ok()?;
        t.parse::<f64>().ok().map(Value::Number)
    }

    fn value(&mut self, depth: usize) -> Option<Value> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.ws();
        match *self.s.get(self.pos)? {
            b'"' => self.string().map(Value::Str),
            b'{' => {
                self.pos += 1;
                let mut m = Vec::new();
                if self.eat(b'}').is_some() {
                    return Some(Value::Object(m));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.eat(b':')?;
                    let v = self.value(depth + 1)?;
                    m.push((k, v));
                    if self.eat(b',').is_some() {
                        continue;
                    }
                    self.eat(b'}')?;
                    return Some(Value::Object(m));
                }
            }
            b'[' => {
                self.pos += 1;
                let mut a = Vec::new();
                if self.eat(b']').is_some() {
                    return Some(Value::Array(a));
                }
                loop {
                    a.push(self.value(depth + 1)?);
                    if self.eat(b',').is_some() {
                        continue;
                    }
                    self.eat(b']')?;
                    return Some(Value::Array(a));
                }
            }
            b't' => self.lit(b"true", Value::Bool(true)),
            b'f' => self.lit(b"false", Value::Bool(false)),
            b'n' => self.lit(b"null", Value::Null),
            _ => self.number(),
        }
    }
}

/// Rozebere cely dokument; `None` = neplatny JSON.
pub fn parse(text: &str) -> Option<Value> {
    let mut p = Parser { s: text.as_bytes(), pos: 0 };
    let v = p.value(0)?;
    p.ws();
    (p.pos == p.s.len()).then_some(v)
}
