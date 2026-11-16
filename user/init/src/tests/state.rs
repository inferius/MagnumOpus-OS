//! Stav systemu pred sadou a po ni: co sada nechala (soubory v `/`
//! a `/tmp`, popis opravneni `/` a `/home`, bezici procesy, pripojena
//! media). Rozdil mimo [`EXCEPTIONS`] je selhani sady (`[SELHAL] <sada>:
//! stav po sade: ...`); soubory a datasety, ktere sada v `/` a `/tmp`
//! zalozila, se po ni pojistkou smazou ([`cleanup`]). Kazda sada musi projit samostatne i v libovolne
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
    // Chyba cteni je soucast stavu (videt v rozdilu i s kodem).
    let acl = ["/", "/home"].iter().map(|p| format!("{p}: {:?}", fs.map(|f| f.permissions(p).map(|x| x.acl)))).collect();
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

/// Co se smi po sade lisit: soubor v `/` nebo `/tmp` (cesta), proces
/// (jmeno), pripojena media.
#[derive(Clone, Copy)]
pub enum Kept {
    Path(&'static str),
    Proc(&'static str),
    Media,
}

/// Zamerne zmeny stavu: (sada, co, proc). Vse ostatni, co sada nechala, je
/// selhani sady (`run_all`). Nova vyjimka jen s duvodem; "nestihl jsem
/// uklidit" duvod neni - na to je `restore.rs` a test `uklid` sady.
pub const EXCEPTIONS: &[(&str, Kept, &str)] = &[
    ("opakovany-start", Kept::Path("/opakovany-start.txt"), "pocitadlo startu musi prezit restart: xtask boottest pousti sadu opakovane nad jednim diskem a po poslednim startu ho cte na hostiteli"),
    ("opakovany-start", Kept::Path("/opakovany-ds"), "dataset se souborem za kazdy start, overuje se pri dalsim startu (viz pocitadlo)"),
    ("firewall", Kept::Proc("netprobe"), "sdileny pomocnik vsech testu firewallu (`PROBE`, spin::Once): jedna relace site podle manifestu na cely beh, novy start za kazdy test by beh zdrzel"),
    ("TLS a HTTP", Kept::Proc("fetch"), "sdileny klient testu TLS a HTTP (`FETCH`, spin::Once), stejny duvod jako netprobe"),
    ("vymenna media", Kept::Media, "sada na konci vsechny flashky vytahne (UNPLUG_ALL); zapojene byly od startu QEMU"),
    ("vymenna media", Kept::Proc("usb-storage"), "skonci s vytazenim flashek; sada sama overuje, ze skoncily"),
    ("vymenna media", Kept::Proc("volfs"), "skonci s vytazenim flashek; sada sama overuje, ze skoncily"),
    // ZNAMA CHYBA (ne zamer): po padu fsd se vymenna media znovu
    // nepripoji (volfs ztrati kanal k fsd a skonci, `media_add` nikdo
    // nezopakuje). Vyjimka plati, dokud to obnova fsd neumi; pak smazat.
    ("obnova fsd", Kept::Media, "ZNAMA CHYBA: media se po obnove fsd nepripoji znovu"),
    ("obnova fsd", Kept::Proc("volfs"), "ZNAMA CHYBA: volfs po padu fsd skonci a znovu se nespusti"),
];

fn allowed(suite: &str, k: impl Fn(Kept) -> bool) -> bool {
    EXCEPTIONS.iter().any(|(s, kept, _)| *s == suite && k(*kept))
}

/// Snimek po sade: procesy, ktere sada ukoncila (drop registru, `kill`),
/// konci asynchronne a vypis FS muze byt hned po obnove fsd chvili jiny -
/// dokud se stav lisi, snimek se opakuje, nejdele 3 s. Co zustane i potom,
/// je skutecny zbytek.
pub fn settled(suite: &str, before: &Snapshot) -> Snapshot {
    let end = libos::time::deadline_after(core::time::Duration::from_secs(3));
    loop {
        let s = take();
        if diff(suite, before, &s).is_empty() || libos::time::now() > end {
            return s;
        }
        libos::time::sleep(core::time::Duration::from_millis(100));
    }
}

fn diff_list(what: &str, a: &[String], b: &[String], ok: impl Fn(&str) -> bool, out: &mut Vec<String>) {
    let added: Vec<&String> = b.iter().filter(|x| !a.contains(x) && !ok(x)).collect();
    let removed: Vec<&String> = a.iter().filter(|x| !b.contains(x) && !ok(x)).collect();
    if !added.is_empty() {
        out.push(format!("{what} navic {added:?}"));
    }
    if !removed.is_empty() {
        out.push(format!("{what} chybi {removed:?}"));
    }
}

/// Rozdil `before` -> `after` bez vyjimek sady (prazdne = sada nechala
/// stejny stav).
pub fn diff(suite: &str, before: &Snapshot, after: &Snapshot) -> Vec<String> {
    let mut out = Vec::new();
    for (what, dir, a, b) in [("v /", "", &before.root, &after.root), ("v /tmp", "/tmp", &before.tmp, &after.tmp)] {
        if let (Some(a), Some(b)) = (a, b) {
            let ok = |n: &str| allowed(suite, |k| matches!(k, Kept::Path(p) if p.strip_prefix(dir).and_then(|r| r.strip_prefix('/')) == Some(n)));
            diff_list(what, a, b, ok, &mut out);
        }
    }
    diff_list("opravneni", &before.acl, &after.acl, |_| false, &mut out);
    if !allowed(suite, |k| matches!(k, Kept::Media)) {
        diff_list("media", &before.media, &after.media, |_| false, &mut out);
    }
    let mut keys: Vec<&String> = before.procs.keys().chain(after.procs.keys()).collect();
    keys.sort();
    keys.dedup();
    for k in keys {
        let (a, b) = (before.procs.get(k).copied().unwrap_or(0), after.procs.get(k).copied().unwrap_or(0));
        if a != b && !allowed(suite, |x| matches!(x, Kept::Proc(n) if n == k.as_str())) {
            out.push(format!("procesy {k}: {a} -> {b}"));
        }
    }
    out
}

/// Pojistka: smaze polozky, ktere sada zalozila v `/` a `/tmp` (stromy
/// i datasety) mimo vyjimky, aby jedna chyba nestrhla dalsi sady. Sada je
/// presto selhana (`diff`). Vraci, co smazala; kdyz vypis selhal, nic.
pub fn cleanup(suite: &str, before: &Snapshot, after: &Snapshot) -> Vec<String> {
    let Some(fs) = devices().and_then(|d| d.fs.as_ref()) else { return Vec::new() };
    let mut out = Vec::new();
    for (dir, a, b) in [("", &before.root, &after.root), ("/tmp", &before.tmp, &after.tmp)] {
        let (Some(a), Some(b)) = (a, b) else { continue };
        for n in b.iter().filter(|n| !a.contains(n)) {
            let p = format!("{dir}/{n}");
            if allowed(suite, |k| matches!(k, Kept::Path(x) if x == p)) {
                continue;
            }
            match super::restore::remove_tree(fs, &p) {
                Ok(()) => out.push(p),
                Err(e) => out.push(format!("{p} (nejde smazat: {e})")),
            }
        }
    }
    out
}
