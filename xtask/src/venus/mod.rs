//! `cargo xtask venus-gen [--check]`: generator serializace Venusu.
//!
//! Vstup: registr Vulkanu `libs/venus/xml/vk.xml` a XML Venusu
//! (`VK_EXT_command_serialization.xml`, `VK_MESA_venus_protocol.xml`,
//! z projektu venus-protocol) a seznam `libs/venus/subset.txt` (ktere
//! prikazy a struktury retezcu pNext). Vystup: `libs/venus/src/vk/`
//! (typy, konstanty, struktury, prikazy), soubory do ~1000 radku.
//!
//! `--check` nic nezapisuje, jen overi, ze vygenerovany kod v repozitari
//! odpovida vstupum (soucast `cargo xtask test`). Kdyz se vstupy ani vystup
//! od posledni uspesne kontroly nezmenily (otisk v
//! `target/venus-check.stamp`), XML se znovu neparsuje.

mod emit;
mod emit_cmd;
mod model;
mod registry;
mod xml;

use std::fs;
use std::path::Path;

use crate::{Result, workspace_root};

const MAX_LINES: usize = 1000;

const BANNER: &str = "// Vygenerovano `cargo xtask venus-gen` z libs/venus/xml (vk.xml a venus-protocol,\n\
// licence v libs/venus/xml/README.md) a libs/venus/subset.txt. Neupravovat rucne.\n";

/// Seznam z `subset.txt`: (prikazy, struktury retezcu).
fn read_subset(text: &str) -> std::result::Result<(Vec<String>, Vec<String>), String> {
    let (mut cmds, mut structs) = (Vec::new(), Vec::new());
    for (i, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        match line.split_once(char::is_whitespace) {
            Some(("command", n)) => cmds.push(n.trim().to_string()),
            Some(("chain", n)) => structs.push(n.trim().to_string()),
            _ => return Err(format!("subset.txt radek {}: ocekavano `command X` nebo `chain X`", i + 1)),
        }
    }
    Ok((cmds, structs))
}

/// Polozky po souborech `<stem>N.rs` (nejvys `MAX_LINES` radku).
fn chunk(stem: &str, items: Vec<String>, files: &mut Vec<(String, String)>) {
    let mut cur = String::new();
    let mut n = 0;
    let header = "use alloc::vec::Vec;\n\nuse super::*;\nuse crate::cs::{CsError, Decoder, Encoder, Result};\n\n";
    let flush = |cur: &mut String, files: &mut Vec<(String, String)>, n: &mut usize| {
        if !cur.is_empty() {
            *n += 1;
            files.push((format!("{stem}{n}.rs"), format!("{BANNER}\n{header}{cur}")));
            cur.clear();
        }
    };
    for it in items {
        if cur.lines().count() + it.lines().count() > MAX_LINES {
            flush(&mut cur, files, &mut n);
        }
        cur.push_str(&it);
        cur.push('\n');
    }
    flush(&mut cur, files, &mut n);
}

/// Vygeneruje soubory (jmeno v `gen/`, obsah).
pub fn generate(root: &Path) -> std::result::Result<Vec<(String, String)>, String> {
    let dir = root.join("libs/venus/xml");
    let mut docs = Vec::new();
    for f in ["vk.xml", "VK_EXT_command_serialization.xml", "VK_MESA_venus_protocol.xml"] {
        let p = dir.join(f);
        let text = fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        docs.push(xml::parse(&text).map_err(|e| format!("{f}: {e}"))?);
    }
    let reg = registry::Registry::load(&docs)?;
    let subset = root.join("libs/venus/subset.txt");
    let text = fs::read_to_string(&subset).map_err(|e| format!("{}: {e}", subset.display()))?;
    let (cmds, chain) = read_subset(&text)?;
    let mut plan = model::Plan::build(&reg, &cmds, &chain)?;
    // Typy, ktere pouziva rucne psany kod (cs, klient).
    for (n, p) in [
        ("VkResult", model::Prim::I32),
        ("VkStructureType", model::Prim::I32),
        ("VkCommandTypeEXT", model::Prim::I32),
        ("VkCommandFlagsEXT", model::Prim::U32),
        ("VkCommandFlagBitsEXT", model::Prim::U32),
    ] {
        plan.scalars.insert(n.to_string(), p);
    }
    let mut files = Vec::new();
    chunk("types", emit::emit_types(&reg, &plan), &mut files);
    let mut structs = Vec::new();
    if let Some(c) = emit::emit_chain(&reg, &plan) {
        structs.push(c);
    }
    let all: std::collections::BTreeSet<&String> = plan.input.iter().chain(plan.output.iter()).collect();
    for s in all {
        structs.push(emit::emit_struct(&reg, &plan, s)?);
    }
    for u in &plan.unions {
        structs.push(emit::emit_union(&reg, u)?);
    }
    chunk("structs", structs, &mut files);
    let mut cmd_items = Vec::new();
    for c in &plan.commands {
        cmd_items.push(emit_cmd::emit_command(&reg, &plan, c)?);
    }
    chunk("cmds", cmd_items, &mut files);
    let mut m = format!(
        "{BANNER}\n//! Vygenerovana serializace Vulkanu pro Venus: typy a konstanty, struktury\n\
         //! (`encode`, `encode_partial`, `decode`) a prikazy (funkce v `cmd`,\n\
         //! odpovedi `*Reply`).\n\n\
         #![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]\n\
         #![allow(dead_code, unused_imports, unused_mut, clippy::all)]\n\n"
    );
    let mut types_mods = Vec::new();
    let mut cmd_mods = Vec::new();
    for (f, _) in &files {
        let stem = f.trim_end_matches(".rs");
        if stem.starts_with("cmds") {
            cmd_mods.push(stem.to_string());
        } else {
            types_mods.push(stem.to_string());
        }
    }
    for s in &types_mods {
        m.push_str(&format!("mod {s};\npub use {s}::*;\n"));
    }
    m.push_str("\n/// Prikazy: kodovani a cteni odpovedi.\npub mod cmd {\n    use super::*;\n");
    for s in &cmd_mods {
        m.push_str(&format!("    #[path = \"../{s}.rs\"]\n    mod {s};\n    pub use {s}::*;\n"));
    }
    m.push_str("}\n");
    // Prikazy jsou v `cmd`: jejich `use super::*` musi videt typy.
    let files = files
        .into_iter()
        .map(|(f, c)| if f.starts_with("cmds") { (f, c.replace("use super::*;", "use super::super::*;")) } else { (f, c) })
        .collect::<Vec<_>>();
    let mut out = vec![(String::from("mod.rs"), m)];
    out.extend(files);
    Ok(out)
}

/// Otisk vstupu (XML, subset.txt, zdroj generatoru) a vygenerovanych
/// souboru: kdyz se nezmenil od posledni uspesne kontroly, neni treba
/// znovu parsovat 3,3 MB vk.xml.
fn fingerprint(root: &Path) -> Option<u64> {
    use std::hash::Hasher;
    let mut paths = Vec::new();
    for d in ["libs/venus/xml", "libs/venus/src/vk", "xtask/src/venus"] {
        for e in fs::read_dir(root.join(d)).ok()?.flatten() {
            paths.push(e.path());
        }
    }
    paths.push(root.join("libs/venus/subset.txt"));
    paths.sort();
    // FNV-1a (stabilni napric verzemi Rustu, na rozdil od DefaultHasher).
    struct Fnv(u64);
    impl Hasher for Fnv {
        fn finish(&self) -> u64 {
            self.0
        }
        fn write(&mut self, b: &[u8]) {
            for &x in b {
                self.0 = (self.0 ^ x as u64).wrapping_mul(0x100_0000_01b3);
            }
        }
    }
    let mut h = Fnv(0xcbf2_9ce4_8422_2325);
    for p in paths {
        h.write(p.file_name()?.to_string_lossy().as_bytes());
        h.write(&fs::read(&p).ok()?);
    }
    Some(h.finish())
}

pub fn run(check: bool) -> Result<()> {
    let root = workspace_root();
    let stamp = root.join("target/venus-check.stamp");
    let print = fingerprint(&root);
    if check
        && let Some(f) = print
        && fs::read_to_string(&stamp).ok().as_deref() == Some(f.to_string().as_str())
    {
        return Ok(());
    }
    let files = generate(&root)?;
    let dir = root.join("libs/venus/src/vk");
    if check {
        let mut stale = Vec::new();
        for (f, c) in &files {
            if fs::read_to_string(dir.join(f)).ok().as_deref() != Some(c.as_str()) {
                stale.push(f.clone());
            }
        }
        if let Ok(rd) = fs::read_dir(&dir) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if !files.iter().any(|(f, _)| *f == n) {
                    stale.push(n);
                }
            }
        }
        if !stale.is_empty() {
            return Err(format!("libs/venus/src/vk neodpovida vstupum ({}): spust `cargo xtask venus-gen`", stale.join(", ")).into());
        }
        if let Some(f) = print {
            let _ = fs::create_dir_all(root.join("target"));
            let _ = fs::write(&stamp, f.to_string());
        }
        return Ok(());
    }
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::create_dir_all(&dir)?;
    for (f, c) in &files {
        fs::write(dir.join(f), c)?;
    }
    println!("venus-gen: {} souboru v {}", files.len(), dir.display());
    Ok(())
}
