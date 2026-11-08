//! Cteni TOML.

use alloc::string::String;
use alloc::vec::Vec;

use crate::{Error, Table, Value};

/// Nejvetsi hloubka vnoreni (pole a inline tabulky): ochrana zasobniku
/// pred zamerne hlubokym vstupem.
const MAX_DEPTH: usize = 32;

struct P<'a> {
    s: &'a [u8],
    i: usize,
    line: usize,
    depth: usize,
}

/// Precte dokument TOML.
pub fn parse(text: &str) -> Result<Table, Error> {
    let mut p = P {
        s: text.as_bytes(),
        i: 0,
        line: 1,
        depth: 0,
    };
    let mut root = Table::new();
    // Cesta aktualni tabulky (pro `[[pole]]` index posledniho prvku).
    let mut current: Vec<Seg> = Vec::new();
    // Tabulky zalozene hlavickou (dvakrat stejna = chyba).
    let mut defined: Vec<Vec<String>> = Vec::new();
    loop {
        p.skip_ws_comments_newlines();
        let Some(c) = p.peek() else {
            break;
        };
        if c == b'[' {
            let array = p.s.get(p.i + 1) == Some(&b'[');
            p.i += if array { 2 } else { 1 };
            p.skip_ws();
            let keys = p.key_path()?;
            p.skip_ws();
            if !p.eat(b']') || (array && !p.eat(b']')) {
                return Err(p.err("ocekavano ]"));
            }
            if array {
                let parent = resolve(&mut root, &current_prefix(&keys), p.line)?;
                let last = keys.last().unwrap();
                let arr = match parent.key_mut(last) {
                    Some(Value::Array(a)) => a,
                    Some(_) => return Err(p.err("klic uz neni pole tabulek")),
                    None => parent.array_mut(last),
                };
                arr.push(Value::Table(Table::new()));
                // Novy prvek: jeho podtabulky jde definovat znovu.
                defined.retain(|d| !d.starts_with(&keys));
                current = keys.iter().map(|k| Seg::Key(k.clone())).collect();
                current.push(Seg::Last);
                p.end_of_line()?;
            } else {
                if defined.contains(&keys) {
                    return Err(p.err("tabulka je definovana dvakrat"));
                }
                defined.push(keys.clone());
                resolve(&mut root, &keys, p.line)?;
                current = keys.iter().map(|k| Seg::Key(k.clone())).collect();
                p.end_of_line()?;
            }
            continue;
        }
        let keys = p.key_path()?;
        p.skip_ws();
        if !p.eat(b'=') {
            return Err(p.err("ocekavano ="));
        }
        p.skip_ws();
        let value = p.value()?;
        let table = walk(&mut root, &current).ok_or_else(|| p.err("neplatna tabulka"))?;
        let (last, path) = keys.split_last().unwrap();
        let t = resolve(table, path, p.line)?;
        if t.key(last).is_some() {
            return Err(p.err("klic je definovany dvakrat"));
        }
        t.entries.push((last.clone(), value));
        p.end_of_line()?;
    }
    Ok(root)
}

#[derive(Debug, Clone)]
enum Seg {
    Key(String),
    /// Posledni prvek pole tabulek.
    Last,
}

fn current_prefix(keys: &[String]) -> Vec<String> {
    keys[..keys.len() - 1].to_vec()
}

/// Projde cestu hlavicky (pole tabulek = jeho posledni prvek, jako v TOML).
fn walk<'t>(root: &'t mut Table, path: &[Seg]) -> Option<&'t mut Table> {
    let mut t = root;
    for seg in path {
        let Seg::Key(k) = seg else {
            continue;
        };
        t = match t.key_mut(k)? {
            Value::Table(x) => x,
            Value::Array(a) => match a.last_mut()? {
                Value::Table(x) => x,
                _ => return None,
            },
            _ => return None,
        };
    }
    Some(t)
}

/// Najde nebo zalozi vnorene tabulky podle klicu (pole tabulek = jeho
/// posledni prvek).
fn resolve<'t>(t: &'t mut Table, keys: &[String], line: usize) -> Result<&'t mut Table, Error> {
    let mut t = t;
    for k in keys {
        if t.key(k).is_none() {
            t.entries.push((k.clone(), Value::Table(Table::new())));
        }
        t = match t.key_mut(k).unwrap() {
            Value::Table(x) => x,
            Value::Array(a) => match a.last_mut() {
                Some(Value::Table(x)) => x,
                _ => {
                    return Err(Error {
                        line,
                        msg: "klic neni tabulka",
                    });
                }
            },
            _ => {
                return Err(Error {
                    line,
                    msg: "klic neni tabulka",
                });
            }
        };
    }
    Ok(t)
}

impl P<'_> {
    fn err(&self, msg: &'static str) -> Error {
        Error {
            line: self.line,
            msg,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.i += 1;
        }
    }

    fn skip_comment(&mut self) {
        if self.peek() == Some(b'#') {
            while !matches!(self.peek(), None | Some(b'\n')) {
                self.i += 1;
            }
        }
    }

    fn newline(&mut self) -> bool {
        if self.peek() == Some(b'\r') && self.s.get(self.i + 1) == Some(&b'\n') {
            self.i += 1;
        }
        if self.eat(b'\n') {
            self.line += 1;
            true
        } else {
            false
        }
    }

    fn skip_ws_comments_newlines(&mut self) {
        loop {
            self.skip_ws();
            self.skip_comment();
            if !self.newline() {
                break;
            }
        }
    }

    /// Za hodnotou smi byt jen mezery, komentar a konec radku.
    fn end_of_line(&mut self) -> Result<(), Error> {
        self.skip_ws();
        self.skip_comment();
        if self.peek().is_none() || self.newline() {
            Ok(())
        } else {
            Err(self.err("neocekavany text za hodnotou"))
        }
    }

    fn key_path(&mut self) -> Result<Vec<String>, Error> {
        let mut keys = Vec::new();
        loop {
            self.skip_ws();
            keys.push(self.key()?);
            self.skip_ws();
            if !self.eat(b'.') {
                return Ok(keys);
            }
        }
    }

    fn key(&mut self) -> Result<String, Error> {
        match self.peek() {
            Some(b'"') => self.basic_string(),
            Some(b'\'') => self.literal_string(),
            _ => {
                let start = self.i;
                while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                {
                    self.i += 1;
                }
                if start == self.i {
                    return Err(self.err("ocekavan klic"));
                }
                Ok(String::from(core::str::from_utf8(&self.s[start..self.i]).unwrap()))
            }
        }
    }

    fn value(&mut self) -> Result<Value, Error> {
        match self.peek() {
            Some(b'"') => self.basic_string().map(Value::Str),
            Some(b'\'') => self.literal_string().map(Value::Str),
            Some(b'[') => self.array(),
            Some(b'{') => self.inline_table(),
            Some(b't') | Some(b'f') => self.boolean(),
            Some(c) if c.is_ascii_digit() || c == b'-' || c == b'+' => self.integer(),
            _ => Err(self.err("neplatna hodnota")),
        }
    }

    fn enter(&mut self) -> Result<(), Error> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.err("prilis hluboke vnoreni"));
        }
        Ok(())
    }

    fn array(&mut self) -> Result<Value, Error> {
        self.enter()?;
        self.i += 1;
        let mut items = Vec::new();
        loop {
            self.skip_ws_comments_newlines();
            if self.eat(b']') {
                break;
            }
            items.push(self.value()?);
            self.skip_ws_comments_newlines();
            if self.eat(b',') {
                continue;
            }
            if self.eat(b']') {
                break;
            }
            return Err(self.err("ocekavano , nebo ]"));
        }
        self.depth -= 1;
        Ok(Value::Array(items))
    }

    fn inline_table(&mut self) -> Result<Value, Error> {
        self.enter()?;
        self.i += 1;
        let mut t = Table::new();
        self.skip_ws();
        if !self.eat(b'}') {
            loop {
                let keys = self.key_path()?;
                self.skip_ws();
                if !self.eat(b'=') {
                    return Err(self.err("ocekavano ="));
                }
                self.skip_ws();
                let v = self.value()?;
                let line = self.line;
                let (last, path) = keys.split_last().unwrap();
                let target = resolve(&mut t, path, line)?;
                if target.key(last).is_some() {
                    return Err(self.err("klic je definovany dvakrat"));
                }
                target.entries.push((last.clone(), v));
                self.skip_ws();
                if self.eat(b',') {
                    self.skip_ws();
                    continue;
                }
                if self.eat(b'}') {
                    break;
                }
                return Err(self.err("ocekavano , nebo }"));
            }
        }
        self.depth -= 1;
        Ok(Value::Table(t))
    }

    fn boolean(&mut self) -> Result<Value, Error> {
        for (word, v) in [(&b"true"[..], true), (&b"false"[..], false)] {
            if self.s[self.i..].starts_with(word) {
                self.i += word.len();
                return Ok(Value::Bool(v));
            }
        }
        Err(self.err("neplatna hodnota"))
    }

    fn integer(&mut self) -> Result<Value, Error> {
        let start = self.i;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'+' | b'.'))
        {
            self.i += 1;
        }
        let raw: String = core::str::from_utf8(&self.s[start..self.i])
            .unwrap()
            .chars()
            .filter(|&c| c != '_')
            .collect();
        let (neg, body) = match raw.as_bytes().first() {
            Some(b'-') => (true, &raw[1..]),
            Some(b'+') => (false, &raw[1..]),
            _ => (false, &raw[..]),
        };
        if !body.starts_with("0x") && body.bytes().any(|b| matches!(b, b'.' | b'e' | b'E')) {
            // Desetinne cislo: jen cislice, tecka a exponent (zadne `inf`, `nan`).
            let ok = body.as_bytes().first().is_some_and(u8::is_ascii_digit)
                && body.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'-' | b'+'));
            let v: f64 = match body.parse() {
                Ok(v) if ok => v,
                _ => return Err(self.err("neplatne cislo")),
            };
            return Ok(Value::Float(crate::Float(if neg { -v } else { v })));
        }
        let v = if let Some(hex) = body.strip_prefix("0x") {
            i64::from_str_radix(hex, 16)
        } else {
            body.parse::<i64>()
        }
        .map_err(|_| self.err("neplatne cislo"))?;
        Ok(Value::Int(if neg { -v } else { v }))
    }

    fn literal_string(&mut self) -> Result<String, Error> {
        self.i += 1;
        let start = self.i;
        while let Some(c) = self.peek() {
            match c {
                b'\'' => {
                    let s = core::str::from_utf8(&self.s[start..self.i])
                        .map_err(|_| self.err("neplatne UTF-8"))?;
                    self.i += 1;
                    return Ok(String::from(s));
                }
                b'\n' => break,
                _ => self.i += 1,
            }
        }
        Err(self.err("neukonceny retezec"))
    }

    fn basic_string(&mut self) -> Result<String, Error> {
        self.i += 1;
        let mut out = Vec::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(self.err("neukonceny retezec"));
            };
            self.i += 1;
            match c {
                b'"' => break,
                b'\n' => return Err(self.err("neukonceny retezec")),
                b'\\' => {
                    let e = self.peek().ok_or_else(|| self.err("neukonceny retezec"))?;
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'u' => {
                            let hex = self
                                .s
                                .get(self.i..self.i + 4)
                                .and_then(|h| core::str::from_utf8(h).ok())
                                .and_then(|h| u32::from_str_radix(h, 16).ok())
                                .and_then(char::from_u32)
                                .ok_or_else(|| self.err("neplatne \\u"))?;
                            self.i += 4;
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(hex.encode_utf8(&mut buf).as_bytes());
                        }
                        _ => return Err(self.err("neznamy escape")),
                    }
                }
                _ => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|_| self.err("neplatne UTF-8"))
    }
}
