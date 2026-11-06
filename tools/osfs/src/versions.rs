//! Datasety a verze nad verejnym API `fs`: k ceste najit dataset, vypsat
//! datasety svazku, citelny radek zmeny z rozdilu verzi.
//!
//! Dataset je slozka s vlastni historii (snapshoty, klony). Snapshoty,
//! rozdily a zdroj obnovy pracuji s cestou **uvnitr datasetu** (`/cfg.toml`
//! v datasetu `/projekt`), zivy svazek s cestou ve svazku
//! (`/projekt/cfg.toml`). Prevod resi [`dataset_of`].

use fs::{BlockDevice, Change, ChangeKind, Error, Fs, Kind};

use crate::image::join;

/// Dataset, ve kterem lezi cesta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetPath {
    /// Koren datasetu ve svazku (`/` = korenovy dataset).
    pub root: String,
    /// Cislo datasetu (1 = korenovy).
    pub id: u64,
    /// Cesta uvnitr datasetu (`/` = jeho koren).
    pub inner: String,
}

fn comps(p: &str) -> Vec<&str> {
    p.split('/').filter(|c| !c.is_empty()).collect()
}

/// Najde dataset, ve kterem lezi `path` (cesta ve svazku). Projde slozky
/// od korene: kde se zmeni cislo datasetu (`Stat::dataset`), zacina
/// vnoreny dataset. Cesta nemusi v zive verzi existovat cela (napr. soubor
/// smazany od snapshotu) - rozhoduje nejhlubsi existujici slozka.
pub fn dataset_of<D: BlockDevice>(fs: &mut Fs<D>, path: &str) -> Result<DatasetPath, Error> {
    let parts = comps(path);
    let mut root = String::from("/");
    let mut id = fs.stat("/")?.dataset;
    let mut root_depth = 0;
    let mut cur = String::from("/");
    for (i, c) in parts.iter().enumerate() {
        cur = join(&cur, c);
        match fs.stat(&cur) {
            Ok(s) => {
                if s.dataset != id {
                    id = s.dataset;
                    root = cur.clone();
                    root_depth = i + 1;
                }
            }
            Err(Error::NotFound) => break,
            Err(e) => return Err(e),
        }
    }
    let mut inner = String::new();
    for c in &parts[root_depth..] {
        inner.push('/');
        inner.push_str(c);
    }
    if inner.is_empty() {
        inner.push('/');
    }
    Ok(DatasetPath { root, id, inner })
}

/// Je `path` korenem vnoreneho datasetu (polozka druhu `Dataset`)?
pub fn is_dataset_root<D: BlockDevice>(fs: &mut Fs<D>, path: &str) -> bool {
    !path.trim_end_matches('/').is_empty()
        && dataset_of(fs, path).is_ok_and(|d| d.inner == "/" && d.root != "/")
}

/// Radek vypisu datasetu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetRow {
    pub path: String,
    pub id: u64,
    /// Klon: (cesta puvodniho datasetu, jmeno snapshotu). Kdyz puvodni
    /// dataset nebo snapshot nejde dohledat, jen cisla.
    pub origin: Option<String>,
}

/// Vsechny datasety svazku (vcetne korenoveho `/`), serazene podle cesty.
pub fn datasets<D: BlockDevice>(fs: &mut Fs<D>) -> Result<Vec<DatasetRow>, Error> {
    // Cesty: pruchod stromem, polozky druhu Dataset (i vnorene).
    let mut found: Vec<(String, u64)> = vec![("/".into(), fs.stat("/")?.dataset)];
    let mut stack = vec![String::from("/")];
    while let Some(dir) = stack.pop() {
        for e in fs.list(&dir)? {
            let p = join(&dir, &e.name);
            match e.kind {
                Kind::Dataset => {
                    found.push((p.clone(), fs.stat(&p)?.dataset));
                    stack.push(p);
                }
                Kind::Dir => stack.push(p),
                _ => {}
            }
        }
    }
    let info = fs.datasets()?;
    let mut rows = Vec::new();
    for (path, id) in &found {
        let origin = match info.iter().find(|d| d.id == *id).and_then(|d| d.origin) {
            None => None,
            Some((ods, osnap, _)) => {
                let opath = found
                    .iter()
                    .find(|(_, i)| *i == ods)
                    .map(|(p, _)| p.clone());
                let sname = match &opath {
                    Some(p) => fs
                        .snapshots_of(p)
                        .ok()
                        .and_then(|v| v.into_iter().find(|s| s.id == osnap).map(|s| s.name)),
                    None => None,
                };
                Some(match (opath, sname) {
                    (Some(p), Some(s)) => format!("{p}@{s}"),
                    (Some(p), None) => format!("{p}@#{osnap}"),
                    _ => format!("dataset #{ods}@#{osnap}"),
                })
            }
        };
        rows.push(DatasetRow {
            path: path.clone(),
            id: *id,
            origin,
        });
    }
    rows.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(rows)
}

/// Znacka zmeny pro vypis (jako `git status --short`).
pub fn change_mark(k: ChangeKind) -> &'static str {
    match k {
        ChangeKind::Added => "A",
        ChangeKind::Removed => "D",
        ChangeKind::Modified => "M",
        ChangeKind::Moved => "R",
        ChangeKind::MovedModified => "RM",
    }
}

/// Citelny radek zmeny: `M  /cfg.toml`, `R  /a.txt -> /b.txt`.
pub fn change_line(c: &Change) -> String {
    let slash = if c.is_dir { "/" } else { "" };
    match &c.old_path {
        Some(old) => format!(
            "{:<2} {old}{slash} -> {}{slash}",
            change_mark(c.kind),
            c.path
        ),
        None => format!("{:<2} {}{slash}", change_mark(c.kind), c.path),
    }
}
