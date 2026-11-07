//! Historie prikazu s kontextem: pracovni slozka, vysledek, doba, cas.

use alloc::string::String;
use alloc::vec::Vec;

use crate::value::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub line: String,
    pub cwd: String,
    pub ok: bool,
    /// Doba behu v ns.
    pub duration: Option<i64>,
    /// Kdy prikaz zacal (ns od 1970 UTC).
    pub time: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct History {
    pub entries: Vec<Entry>,
    /// Nejvic polozek (starsi se zahazuji).
    pub limit: usize,
}

impl History {
    pub fn push(&mut self, e: Entry) {
        if e.line.is_empty() {
            return;
        }
        let limit = if self.limit == 0 { 1000 } else { self.limit };
        self.entries.push(e);
        if self.entries.len() > limit {
            self.entries.remove(0);
        }
    }

    /// Hledani: prikazy obsahujici `q` (bez ohledu na velikost pismen),
    /// volitelne jen v dane slozce; nejnovejsi prvni.
    pub fn search(&self, q: &str, cwd: Option<&str>) -> Vec<&Entry> {
        let q = q.to_lowercase();
        self.entries
            .iter()
            .rev()
            .filter(|e| e.line.to_lowercase().contains(&q))
            .filter(|e| cwd.is_none_or(|c| crate::vfs::path::key(&e.cwd) == crate::vfs::path::key(c)))
            .collect()
    }

    pub fn to_value(&self) -> Value {
        Value::List(
            self.entries
                .iter()
                .map(|e| {
                    Value::record(&[
                        ("line", Value::Str(e.line.clone())),
                        ("cwd", Value::Str(e.cwd.clone())),
                        ("ok", Value::Bool(e.ok)),
                        ("duration", e.duration.map(Value::Duration).unwrap_or(Value::Null)),
                        ("time", e.time.map(Value::Time).unwrap_or(Value::Null)),
                    ])
                })
                .collect(),
        )
    }
}
