//! Vestavene nastroje `agentd`: soubory, pracovni prostor, navrh prikazu
//! shellu, priprava prekladu a testu.
//!
//! Kazdy nastroj bezi **s capability relace**: cteni pres relaci FS jen
//! pro cteni (identita uzivatele), zapis v prostoru pres relaci prostoru.
//! `agentd` jina prava k souborum nema.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agentproto::json::Value;
use agentproto::tool::{Effect, Schema, ToolSpec};
use fsproto::{FsClient, kind};
use libos::{Error, Result};

use crate::state::write_all;

/// Nejvic bajtu, ktere `fs.read` vrati najednou.
pub const READ_MAX: u64 = 64 * 1024;
/// Nejvic bajtu obsahu `fs.write` (zprava kanalu ma 64 KiB).
pub const WRITE_MAX: usize = 48 * 1024;
const SEARCH_MAX: usize = 100;
const SEARCH_DEPTH: usize = 8;
const SEARCH_FILE_MAX: u64 = 256 * 1024;

/// Popisy vestavenych nastroju.
pub fn builtin() -> Vec<ToolSpec> {
    let path = |d: &str| Schema::object().string("path", d, true);
    alloc::vec![
        ToolSpec::new("fs.read", "Precte textovy soubor (nejvys 64 KiB od offsetu).", Effect::Read,
            path("absolutni cesta k souboru").integer("offset", "od bajtu (vychozi 0)", false).build()),
        ToolSpec::new("fs.list", "Vypis slozky: jmeno, druh, velikost.", Effect::Read, path("absolutni cesta ke slozce").build()),
        ToolSpec::new("fs.stat", "Metadata cesty: druh, velikost, cas zmeny.", Effect::Read, path("absolutni cesta").build()),
        ToolSpec::new("fs.search", "Hleda pod slozkou soubory podle casti jmena a/nebo textu v obsahu (nejvys 100 vysledku).", Effect::Read,
            Schema::object().string("dir", "absolutni cesta ke slozce", true).string("name", "cast jmena", false)
                .string("text", "text v obsahu", false).build()),
        ToolSpec::new("fs.write", "Zapise cely obsah souboru (vytvori ho). Mimo pracovni prostor se uzivatel pta.", Effect::Write,
            path("absolutni cesta").string("content", "novy obsah (text, nejvys 48 KiB)", true).build()),
        ToolSpec::new("fs.mkdir", "Vytvori slozku.", Effect::Write, path("absolutni cesta").build()),
        ToolSpec::new("fs.remove", "Presune soubor nebo slozku do kose (ne trvale).", Effect::Write, path("absolutni cesta").build()),
        ToolSpec::new("fs.rename", "Prejmenuje nebo presune.", Effect::Write,
            Schema::object().string("from", "puvodni cesta", true).string("to", "nova cesta", true).build()),
        ToolSpec::new("ws.steps", "Kroky relace v pracovnim prostoru (commity).", Effect::Read, Schema::object().build()),
        ToolSpec::new("ws.diff", "Zmeny v pracovnim prostoru: krok N proti predchozimu, 0 = vse od zacatku relace.", Effect::Read,
            Schema::object().integer("step", "cislo kroku (0 = od zacatku)", false).build()),
        ToolSpec::new("shell.propose", "Navrhne prikaz shellu (zapis B nebo C); parser ho overi. Pro vice kroku pole steps.", Effect::Propose,
            Schema::object().string("command", "prikaz (nebo prvni krok)", true).string("style", "B nebo C", false)
                .string("explanation", "co prikaz dela", true).strings("steps", "dalsi kroky ulohy", false).build()),
        ToolSpec::new("dev.build", "Prelozi projekt v pracovnim prostoru (az bude prekladac v systemu).", Effect::Exec,
            Schema::object().string("target", "cil prekladu", false).build()),
        ToolSpec::new("dev.test", "Spusti testy projektu (az bude prekladac v systemu).", Effect::Exec,
            Schema::object().string("filter", "vyber testu", false).build()),
    ]
}

fn kind_name(k: u8) -> &'static str {
    match k {
        kind::FILE => "soubor",
        kind::DIR => "slozka",
        kind::SYMLINK => "odkaz",
        kind::DATASET => "dataset",
        _ => "?",
    }
}

/// Nastroje pro cteni (capability relace jen pro cteni).
pub fn read(fs: &FsClient, name: &str, input: &Value) -> Result<String> {
    match name {
        "fs.read" => {
            let path = input.str_of("path");
            let st = fs.stat(path)?;
            if st.kind != kind::FILE {
                return Err(Error::IsADirectory);
            }
            let off = input.u64_of("offset").min(st.size);
            let len = (st.size - off).min(READ_MAX);
            let data = fs.read(path, off, len as usize)?;
            let mut s = String::from_utf8_lossy(&data).into_owned();
            if off + len < st.size {
                s.push_str(&format!("\n[... dalsich {} B, pokracuj s offset {}]", st.size - off - len, off + len));
            }
            Ok(s)
        }
        "fs.list" => {
            let path = input.str_of("path");
            let mut out = String::new();
            for e in fs.list_stat(path)?.iter().take(1000) {
                out.push_str(&format!("{}\t{}\t{}\n", e.name, kind_name(e.kind), e.size));
            }
            Ok(out)
        }
        "fs.stat" => {
            let st = fs.stat(input.str_of("path"))?;
            Ok(format!("druh={} velikost={} zmena={} vlastnik={}", kind_name(st.kind), st.size, st.mtime.secs, st.owner))
        }
        "fs.search" => search(fs, input.str_of("dir"), input.str_of("name"), input.str_of("text")),
        _ => Err(Error::NotFound),
    }
}

fn search(fs: &FsClient, dir: &str, name: &str, text: &str) -> Result<String> {
    if name.is_empty() && text.is_empty() {
        return Err(Error::InvalidArgs);
    }
    let mut out = Vec::new();
    let mut stack = alloc::vec![(String::from(dir.trim_end_matches('/')), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(entries) = fs.list_stat(if d.is_empty() { "/" } else { &d }) else { continue };
        for e in entries {
            let full = format!("{d}/{}", e.name);
            if e.kind == kind::DIR || e.kind == kind::DATASET {
                if depth + 1 < SEARCH_DEPTH {
                    stack.push((full, depth + 1));
                }
                continue;
            }
            if e.kind != kind::FILE || (!name.is_empty() && !e.name.contains(name)) {
                continue;
            }
            if !text.is_empty() {
                if e.size > SEARCH_FILE_MAX {
                    continue;
                }
                let Ok(data) = fs.read(&full, 0, e.size as usize) else { continue };
                if !String::from_utf8_lossy(&data).contains(text) {
                    continue;
                }
            }
            out.push(full);
            if out.len() >= SEARCH_MAX {
                return Ok(out.join("\n") + "\n[... dalsi vysledky vynechany]");
            }
        }
    }
    Ok(if out.is_empty() { String::from("nic nenalezeno") } else { out.join("\n") })
}

/// Zapisove nastroje v pracovnim prostoru (capability prostoru).
pub fn write(fs: &FsClient, name: &str, input: &Value) -> Result<String> {
    match name {
        "fs.write" => {
            let (path, content) = (input.str_of("path"), input.str_of("content"));
            if content.len() > WRITE_MAX {
                return Err(Error::OutOfRange);
            }
            write_all(fs, path, content.as_bytes())?;
            Ok(format!("zapsano {} B do {path}", content.len()))
        }
        "fs.mkdir" => fs.create(input.str_of("path"), kind::DIR).map(|_| format!("vytvorena slozka {}", input.str_of("path"))),
        "fs.remove" => fs.trash(input.str_of("path")).map(|_| format!("{} presunuto do kose", input.str_of("path"))),
        "fs.rename" => fs.rename(input.str_of("from"), input.str_of("to")).map(|_| format!("{} -> {}", input.str_of("from"), input.str_of("to"))),
        _ => Err(Error::NotFound),
    }
}

/// Akce k potvrzeni pro zapisovy nastroj mimo prostor (a nahled).
pub fn action(read_fs: &FsClient, name: &str, input: &Value) -> Result<(agentproto::action::Action, String)> {
    use agentproto::action::{Action, preview_write};
    let path = String::from(input.str_of("path"));
    Ok(match name {
        "fs.write" => {
            let content = input.str_of("content").as_bytes().to_vec();
            if content.len() > WRITE_MAX {
                return Err(Error::OutOfRange);
            }
            // Stary obsah pres capability cteni (kdyz ho uzivatel smi cist).
            let old = crate::state::read_all(read_fs, &path, 1 << 20).ok();
            let preview = preview_write(&path, old.as_deref(), &content);
            (Action::FsWrite { path, content }, preview)
        }
        "fs.mkdir" => (Action::FsMkdir { path: path.clone() }, format!("nova slozka {path}")),
        "fs.remove" => (Action::FsTrash { path: path.clone() }, format!("presunout do kose: {path}")),
        "fs.rename" => {
            let (from, to) = (String::from(input.str_of("from")), String::from(input.str_of("to")));
            let p = format!("{from} -> {to}");
            (Action::FsRename { from, to }, p)
        }
        _ => return Err(Error::NotSupported),
    })
}
