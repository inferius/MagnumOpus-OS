//! Konfigurace v TOML: cteni a zapis (bez `std`).
//!
//! Podmnozina TOML 1.0, ktera staci na konfiguraci psanou clovekem
//! (manifesty aplikaci, nastaveni firewallu):
//!
//! - komentare `#`, tabulky `[a.b]`, pole tabulek `[[a.b]]`,
//! - klice holé (`A-Z a-z 0-9 _ -`) i v uvozovkach, teckovane `a.b = 1`,
//! - hodnoty: retezec (`"..."` s escape `\" \\ \n \t \r \uXXXX`, `'...'`
//!   doslovne), cele cislo (i `_`, `0x`), desetinne cislo (`16.7`, `1e3`),
//!   `true`/`false`, pole `[...]` (i pres vic radku), inline tabulka
//!   `{ a = 1 }`.
//!
//! Neumi: datumy a casy, `inf`/`nan`, viceradkove retezce `"""`.
//! Chyba nese cislo radku. Zapis ([`Table::to_toml`]) vytvori text, ktery
//! jde znovu precist; poradi klicu se zachovava.
//!
//! ```
//! let t = conf::parse("[app]\nid = \"org.test\"\n[[rule]]\nport = 443\n").unwrap();
//! assert_eq!(t.get_str("app.id"), Some("org.test"));
//! assert_eq!(t.tables("rule")[0].get_int("port"), Some(443));
//! ```

#![no_std]

extern crate alloc;

mod parse;
mod write;

#[cfg(test)]
mod tests;

use alloc::string::String;
use alloc::vec::Vec;

pub use parse::parse;

/// Hodnota.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Str(String),
    Int(i64),
    Float(Float),
    Bool(bool),
    Array(Vec<Value>),
    Table(Table),
}

/// Desetinne cislo (porovnani podle bitu, aby slo odvodit `Eq`).
#[derive(Debug, Clone, Copy)]
pub struct Float(pub f64);

impl PartialEq for Float {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for Float {}

/// Tabulka: dvojice klic - hodnota v poradi zapisu.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Table {
    pub entries: Vec<(String, Value)>,
}

/// Chyba cteni.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// Radek (od 1).
    pub line: usize,
    pub msg: &'static str,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "radek {}: {}", self.line, self.msg)
    }
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(v) => Some(*v),
            _ => None,
        }
    }

    /// Cislo jako `f64` (cele i desetinne).
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Float(v) => Some(v.0),
            Self::Int(v) => Some(*v as f64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_table(&self) -> Option<&Table> {
        match self {
            Self::Table(t) => Some(t),
            _ => None,
        }
    }
}

impl Table {
    pub fn new() -> Self {
        Self::default()
    }

    /// Hodnota klice v teto tabulce (bez tecek).
    pub fn key(&self, key: &str) -> Option<&Value> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn key_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.entries.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Hodnota podle cesty `a.b.c` (pres vnorene tabulky).
    pub fn get(&self, path: &str) -> Option<&Value> {
        let mut t = self;
        let mut parts = path.split('.').peekable();
        while let Some(p) = parts.next() {
            let v = t.key(p)?;
            if parts.peek().is_none() {
                return Some(v);
            }
            t = v.as_table()?;
        }
        None
    }

    pub fn get_str(&self, path: &str) -> Option<&str> {
        self.get(path)?.as_str()
    }

    pub fn get_int(&self, path: &str) -> Option<i64> {
        self.get(path)?.as_int()
    }

    /// Cislo (cele i desetinne) jako `f64`.
    pub fn get_f64(&self, path: &str) -> Option<f64> {
        self.get(path).and_then(Value::as_f64)
    }

    pub fn get_bool(&self, path: &str) -> Option<bool> {
        self.get(path)?.as_bool()
    }

    pub fn get_table(&self, path: &str) -> Option<&Table> {
        self.get(path)?.as_table()
    }

    /// Pole retezcu (`["a", "b"]`); jeden retezec = pole s jednim prvkem.
    pub fn get_strs(&self, path: &str) -> Option<Vec<&str>> {
        match self.get(path)? {
            Value::Str(s) => Some(alloc::vec![s.as_str()]),
            Value::Array(a) => a.iter().map(Value::as_str).collect(),
            _ => None,
        }
    }

    /// Tabulky pole tabulek `[[path]]` (prazdne, kdyz neni).
    pub fn tables(&self, path: &str) -> Vec<&Table> {
        match self.get(path) {
            Some(Value::Array(a)) => a.iter().filter_map(Value::as_table).collect(),
            Some(Value::Table(t)) => alloc::vec![t],
            _ => Vec::new(),
        }
    }

    /// Nastavi klic (prepise, nebo prida na konec).
    pub fn set(&mut self, key: &str, value: Value) {
        match self.key_mut(key) {
            Some(v) => *v = value,
            None => self.entries.push((String::from(key), value)),
        }
    }

    /// Odebere klic.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        let i = self.entries.iter().position(|(k, _)| k == key)?;
        Some(self.entries.remove(i).1)
    }

    /// Vnorena tabulka `key` (zalozi ji, kdyz neni).
    pub fn table_mut(&mut self, key: &str) -> &mut Table {
        if !matches!(self.key(key), Some(Value::Table(_))) {
            self.set(key, Value::Table(Table::new()));
        }
        match self.key_mut(key) {
            Some(Value::Table(t)) => t,
            _ => unreachable!("tabulka prave zalozena"),
        }
    }

    /// Pole `key` (zalozi ho, kdyz neni).
    pub fn array_mut(&mut self, key: &str) -> &mut Vec<Value> {
        if !matches!(self.key(key), Some(Value::Array(_))) {
            self.set(key, Value::Array(Vec::new()));
        }
        match self.key_mut(key) {
            Some(Value::Array(a)) => a,
            _ => unreachable!("pole prave zalozeno"),
        }
    }

    /// Text TOML (tabulky jako `[sekce]`, pole tabulek jako `[[sekce]]`).
    pub fn to_toml(&self) -> String {
        write::write(self)
    }
}

/// Velikost s jednotkou: `65536`, `"64K"`, `"16M"`, `"2G"` (nasobky 1024).
pub fn parse_size(v: &Value) -> Option<u64> {
    match v {
        Value::Int(n) if *n >= 0 => Some(*n as u64),
        Value::Str(s) => {
            let s = s.trim();
            let (num, mul) = match s.as_bytes().last()? {
                b'K' | b'k' => (&s[..s.len() - 1], 1u64 << 10),
                b'M' | b'm' => (&s[..s.len() - 1], 1 << 20),
                b'G' | b'g' => (&s[..s.len() - 1], 1 << 30),
                b'T' | b't' => (&s[..s.len() - 1], 1 << 40),
                _ => (s, 1),
            };
            num.trim().parse::<u64>().ok()?.checked_mul(mul)
        }
        _ => None,
    }
}
