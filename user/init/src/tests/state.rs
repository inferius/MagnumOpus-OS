//! Stav systemu pred sadou a po ni: co sada nechala (soubory v `/`
//! a `/tmp`, popis opravneni `/` a `/home`, bezici procesy, pripojena
//! media). Rozdil se vypise do logu (`init: stav po sade ...`); soubory
//! a datasety, ktere sada v `/` a `/tmp` zalozila, se po ni smazou
//! ([`cleanup`]). Kazda sada musi projit samostatne i v libovolne
//! kombinaci - zbyly stav jine sady je nejcastejsi pricina "nahodnych"
//! selhani (napr. nahled `preview` v terminalu videl soubor jine sady).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::devices;

#[derive(PartialEq, Eq)]
pub struct Snapshot {
    /// `None` = vypis selhal (napr. hned po restartu fsd): neporovnavat
    /// ani neuklizet.
    root: Option<Vec<String>>,
    tmp: Option<Vec<String>>,
    acl: Vec<String>,
    procs: BTreeMap<String, usize>,
    media: Vec<String>,
}

fn names(path: &str) -> Option<Vec<String>> {
    let fs = devices().and_then(|d| d.fs.as_ref())?;
    let mut v: Vec<String> = fs.list(path).ok()?.into_iter().map(|e| e.name).collect();
    v.sort();
    Some(v)
}

pub fn take() -> Snapshot {
    let fs = devices().and_then(|d| d.fs.as_ref());
    let acl = ["/", "/home"].iter().map(|p| format!("{p}: {:?}", fs.and_then(|f| f.permissions(p).ok()).map(|x| x.acl))).collect();
    let mut procs = BTreeMap::new();
    if let Ok(ps) = libos::job::current().and_then(|j| j.processes()) {
        for p in ps.iter().filter(|p| p.flags & libos::abi::job::proc_flags::FINISHED == 0) {
            let n = p.name.split(|&b| b == 0).next().unwrap_or(&[]);
            *procs.entry(String::from_utf8_lossy(n).into_owned()).or_insert(0) += 1;
        }
    }
    let media = fs.and_then(|f| f.media_list().ok()).map(|l| l.into_iter().map(|m| m.name).collect()).unwrap_or_default();
    Snapshot { root: names("/"), tmp: names("/tmp"), acl, procs, media }
}

/// Snimek po sade: procesy, ktere sada ukoncila (drop registru, `kill`),
/// konci asynchronne - dokud se pocty procesu lisi od `before`, snimek se
/// opakuje, nejdele 3 s. Co zustane i potom, je skutecny zbytek.
pub fn settled(before: &Snapshot) -> Snapshot {
    let end = libos::time::deadline_after(core::time::Duration::from_secs(3));
    loop {
        let s = take();
        if s.procs == before.procs || libos::time::now() > end {
            return s;
        }
        libos::time::sleep(core::time::Duration::from_millis(100));
    }
}

fn diff_list(what: &str, a: &[String], b: &[String], out: &mut Vec<String>) {
    let added: Vec<&String> = b.iter().filter(|x| !a.contains(x)).collect();
    let removed: Vec<&String> = a.iter().filter(|x| !b.contains(x)).collect();
    if !added.is_empty() {
        out.push(format!("{what} navic {added:?}"));
    }
    if !removed.is_empty() {
        out.push(format!("{what} chybi {removed:?}"));
    }
}

/// Rozdil `before` -> `after` (prazdne = sada nechala stejny stav).
pub fn diff(before: &Snapshot, after: &Snapshot) -> Vec<String> {
    let mut out = Vec::new();
    for (what, a, b) in [("v /", &before.root, &after.root), ("v /tmp", &before.tmp, &after.tmp)] {
        if let (Some(a), Some(b)) = (a, b) {
            diff_list(what, a, b, &mut out);
        }
    }
    diff_list("opravneni", &before.acl, &after.acl, &mut out);
    diff_list("media", &before.media, &after.media, &mut out);
    let mut keys: Vec<&String> = before.procs.keys().chain(after.procs.keys()).collect();
    keys.sort();
    keys.dedup();
    for k in keys {
        let (a, b) = (before.procs.get(k).copied().unwrap_or(0), after.procs.get(k).copied().unwrap_or(0));
        if a != b {
            out.push(format!("procesy {k}: {a} -> {b}"));
        }
    }
    out
}

/// Smaze polozky, ktere sada zalozila v `/` a `/tmp` (stromy i datasety);
/// vraci, co smazala. Kdyz vypis pred nebo po sade selhal, nic.
pub fn cleanup(before: &Snapshot, after: &Snapshot) -> Vec<String> {
    let Some(fs) = devices().and_then(|d| d.fs.as_ref()) else { return Vec::new() };
    let mut out = Vec::new();
    for (dir, a, b) in [("", &before.root, &after.root), ("/tmp", &before.tmp, &after.tmp)] {
        let (Some(a), Some(b)) = (a, b) else { continue };
        for n in b.iter().filter(|n| !a.contains(n)) {
            let p = format!("{dir}/{n}");
            match super::restore::remove_tree(fs, &p) {
                Ok(()) => out.push(p),
                Err(e) => out.push(format!("{p} (nejde smazat: {e})")),
            }
        }
    }
    out
}
