//! Diff zmen souboru: polozky pro nahled, zamek, vystup prikazu a undo.

use alloc::string::String;
use alloc::vec::Vec;

use crate::value::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Removed,
    DirAdded,
    DirRemoved,
    /// Soubor nahrazen slozkou nebo naopak.
    Replaced,
    /// Presunuto z `from` (obsah se mohl zmenit taky).
    Moved,
}

impl ChangeKind {
    pub fn name(self) -> &'static str {
        match self {
            ChangeKind::Added => "added",
            ChangeKind::Modified => "modified",
            ChangeKind::Removed => "removed",
            ChangeKind::DirAdded => "dir-added",
            ChangeKind::DirRemoved => "dir-removed",
            ChangeKind::Replaced => "replaced",
            ChangeKind::Moved => "moved",
        }
    }
}

/// Jedna polozka diffu.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffEntry {
    pub path: String,
    pub change: ChangeKind,
    /// U presunu puvodni cesta.
    pub from: Option<String>,
    pub before: Option<u64>,
    pub after: Option<u64>,
}

impl DiffEntry {
    /// Cesty, kterych se zmena tyka (u presunu obe).
    pub fn paths(&self) -> impl Iterator<Item = &String> {
        core::iter::once(&self.path).chain(self.from.iter())
    }
}

/// Diff jako tabulka (`{change, path, from, before, after}`).
pub fn diff_value(diff: &[DiffEntry]) -> Value {
    Value::List(
        diff.iter()
            .map(|d| {
                Value::record(&[
                    ("change", Value::str(d.change.name())),
                    ("path", Value::str(d.path.clone())),
                    ("from", d.from.clone().map(Value::Str).unwrap_or(Value::Null)),
                    ("before", d.before.map(|b| Value::Size(b as i64)).unwrap_or(Value::Null)),
                    ("after", d.after.map(|b| Value::Size(b as i64)).unwrap_or(Value::Null)),
                ])
            })
            .collect(),
    )
}

/// Serazeni podle cesty (bez ohledu na velikost pismen), bez duplicit.
pub fn normalize(mut diff: Vec<DiffEntry>) -> Vec<DiffEntry> {
    diff.sort_by_key(|d| super::path::key(&d.path));
    diff.dedup_by(|a, b| super::path::key(&a.path) == super::path::key(&b.path) && a.from == b.from);
    diff
}
