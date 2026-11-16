//! Referencni napoveda prikazu: jeden zdroj pravdy pro `help`, budouci
//! interaktivni napovedu a stranku dokumentace „Příkazy shellu“
//! (`cargo xtask docs` ji generuje z techto dat).
//!
//! Podpis prikazu ([`crate::sig::Sig`]: argumenty, typy, prepinace, vstup
//! a vystup vroure, plne jmeno) zustava v `sig.rs`; tady je jen to, co
//! z podpisu nejde odvodit: dlouhy popis, vychozi hodnoty, priklady,
//! souvisejici prikazy, chyby a opravneni - cesky i anglicky.
//!
//! Priklady jsou spustitelne: test `doc_examples_run` je provede nad
//! testovacim FS (`tests/host.rs`) a porovna vystup, takze napoveda
//! nezastara.

mod data_files;
mod data_lists;
mod data_system;
mod data_text;

use alloc::string::String;
use alloc::vec::Vec;

use crate::sig::{BUILTINS, GROUPS, Io, Kind, Sig};

/// Jak se priklad overi v testu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// Vystup (hodnoty vykreslene jako v terminalu bez barev, oriznute
    /// mezery na koncich radku) se musi shodovat.
    Out(&'static str),
    /// Musi projit bez chyby (vystup zavisi na case, stroji...).
    Runs,
    /// Nespousti se (vypnuti pocitace, sit, zvukova karta).
    Skip,
}

#[derive(Debug, Clone, Copy)]
pub struct Example {
    pub code: &'static str,
    pub cs: &'static str,
    pub en: &'static str,
    pub check: Check,
}

/// Argument nebo prepinac (jmeno jako v podpisu: `path`, `recursive`).
#[derive(Debug, Clone, Copy)]
pub struct Arg {
    pub name: &'static str,
    /// Vychozi hodnota (prazdne = zadna / povinny).
    pub default: &'static str,
    pub cs: &'static str,
    pub en: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct Doc {
    pub name: &'static str,
    /// Kratky popis anglicky (cesky je `Sig::help`).
    pub short_en: &'static str,
    pub long_cs: &'static str,
    pub long_en: &'static str,
    pub args: &'static [Arg],
    pub examples: &'static [Example],
    pub related: &'static [&'static str],
    /// Kody chyb, ktere prikaz vraci (viz `explain <kod>`).
    pub errors: &'static [&'static str],
    /// Co prikaz potrebuje (opravneni); prazdne = nic zvlastniho.
    pub needs_cs: &'static str,
    pub needs_en: &'static str,
}

/// Anglicke nazvy skupin (poradi jako [`GROUPS`]).
pub const GROUPS_EN: &[&str] =
    &["Files", "Text and output", "Lists and tables", "Conversions and time", "Tasks", "Session and safety", "Help", "Account and permissions", "System"];

/// Vsechna metadata (poradi podle skupin dat).
pub fn all() -> impl Iterator<Item = &'static Doc> {
    data_files::DOCS.iter().chain(data_text::DOCS).chain(data_lists::DOCS).chain(data_system::DOCS)
}

pub fn get(name: &str) -> Option<&'static Doc> {
    all().find(|d| d.name == name)
}

fn sig(name: &str) -> Option<&'static Sig> {
    BUILTINS.iter().find(|s| s.name == name)
}

/// Skupina prikazu: (index, cesky nazev).
pub fn group_of(name: &str) -> Option<(usize, &'static str)> {
    GROUPS.iter().enumerate().find(|(_, (_, ns))| ns.contains(&name)).map(|(i, (t, _))| (i, *t))
}

fn kind_name(k: Kind, en: bool) -> &'static str {
    match (k, en) {
        (Kind::Any, false) => "hodnota",
        (Kind::Any, true) => "value",
        (Kind::Path, false) => "cesta",
        (Kind::Path, true) => "path",
        (Kind::Str, false) => "text",
        (Kind::Str, true) => "text",
        (Kind::Int, false) => "číslo",
        (Kind::Int, true) => "number",
        (Kind::Duration, false) => "doba",
        (Kind::Duration, true) => "duration",
        (Kind::Closure, false) => "funkce",
        (Kind::Closure, true) => "closure",
        (Kind::Cond, false) => "podmínka",
        (Kind::Cond, true) => "condition",
        (Kind::Block, false) => "blok",
        (Kind::Block, true) => "block",
    }
}

/// Vstup / vystup v roure lidsky.
pub fn io_name(io: Io, en: bool) -> String {
    match (io, en) {
        (Io::Nothing, false) => "nic".into(),
        (Io::Nothing, true) => "nothing".into(),
        (Io::Any, false) => "libovolná hodnota".into(),
        (Io::Any, true) => "any value".into(),
        (Io::Bytes, false) => "bajty".into(),
        (Io::Bytes, true) => "bytes".into(),
        (Io::Text, false) => "text".into(),
        (Io::Text, true) => "text".into(),
        (Io::Records(f), false) if f.is_empty() => "seznam".into(),
        (Io::Records(f), true) if f.is_empty() => "list".into(),
        (Io::Records(f), false) => alloc::format!("tabulka ({})", f.join(", ")),
        (Io::Records(f), true) => alloc::format!("table ({})", f.join(", ")),
    }
}

/// Radky argumentu: (zapis, typ, vychozi, popis).
fn arg_rows(s: &Sig, d: Option<&Doc>, en: bool) -> Vec<(String, &'static str, &'static str, &'static str)> {
    let find = |n: &str| d.and_then(|d| d.args.iter().find(|a| a.name == n));
    let mut rows = Vec::new();
    for p in s.params {
        let a = find(p.name);
        let text = a.map_or("", |a| if en { a.en } else { a.cs });
        let name = if p.optional { alloc::format!("[{}]", p.name) } else { String::from(p.name) };
        rows.push((name, kind_name(p.kind, en), a.map_or("", |a| a.default), text));
    }
    if let Some(k) = s.rest {
        let a = find("...");
        rows.push((String::from("..."), kind_name(k, en), "", a.map_or("", |a| if en { a.en } else { a.cs })));
    }
    for f in s.flags {
        let a = find(f.long);
        let name = match f.short {
            Some(c) => alloc::format!("-{c}, --{}", f.long),
            None => alloc::format!("--{}", f.long),
        };
        let ty = f.value.map_or(if en { "switch" } else { "přepínač" }, |k| kind_name(k, en));
        let text = match (a, en) {
            (Some(a), true) => a.en,
            (Some(a), false) => a.cs,
            (None, _) => f.help,
        };
        rows.push((name, ty, a.map_or("", |a| a.default), text));
    }
    rows
}

/// `help <prikaz>`: sekce s nadpisy, priklady s vysvetlenim.
pub fn help_text(s: &Sig) -> String {
    let d = get(s.name);
    let mut o = alloc::format!("{} - {}\n", s.name, s.help);
    if let Some(d) = d {
        o.push_str(&alloc::format!("\n{}\n", d.long_cs));
    }
    o.push_str(&alloc::format!("\nPoužití\n  {}\n", crate::parse::usage(s)));
    o.push_str(&alloc::format!("  plné jméno: {}, zápis C: {}(...)\n", s.full, s.name.replace('-', "_")));
    let rows = arg_rows(s, d, false);
    if !rows.is_empty() {
        let w = rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(0);
        o.push_str("\nArgumenty a přepínače\n");
        for (n, ty, def, text) in &rows {
            let def = if def.is_empty() { String::new() } else { alloc::format!(" (výchozí {def})") };
            o.push_str(&alloc::format!("  {n:<w$}  {ty}: {text}{def}\n"));
        }
    }
    o.push_str(&alloc::format!("\nRoura\n  vstup: {}\n  výstup: {}\n", io_name(s.input, false), io_name(s.output, false)));
    if let Some(d) = d {
        if !d.examples.is_empty() {
            o.push_str("\nPříklady\n");
            for e in d.examples {
                o.push_str(&alloc::format!("  > {}\n    {}\n", e.code, e.cs));
                if let Check::Out(out) = e.check
                    && !out.is_empty()
                {
                    for l in out.lines() {
                        o.push_str(&alloc::format!("      {l}\n"));
                    }
                }
            }
        }
        if !d.needs_cs.is_empty() {
            o.push_str(&alloc::format!("\nPotřebuje\n  {}\n", d.needs_cs));
        }
        if !d.errors.is_empty() {
            o.push_str(&alloc::format!("\nChyby (vysvětlí explain <kód>)\n  {}\n", d.errors.join(", ")));
        }
        if !d.related.is_empty() {
            o.push_str(&alloc::format!("\nSouvisí\n  {}\n", d.related.join(", ")));
        }
    }
    o
}

/// Stranka dokumentace (Markdown) v jazyce `en`: rejstrik po skupinach
/// a sekce s kotvou `#<jmeno>` (slug nadpisu) pro kazdy prikaz.
pub fn markdown(en: bool) -> String {
    let mut o = String::new();
    let t = |cs: &'static str, e: &'static str| if en { e } else { cs };
    o.push_str(t("# Příkazy shellu\n\n", "# Shell commands\n\n"));
    o.push_str(t(
        "> Stránka je vygenerovaná z metadat příkazů v `libs/shlang/src/doc/` (`cargo xtask docs`) - neupravujte ji ručně. Stejná data ukazuje `help <příkaz>` v terminálu a příklady jsou testy: běží v testovací složce `/home` (soubory `a.txt`, `b.txt`, `data.json` a složka `docs`) a jejich výstup se porovnává.\n\n",
        "> This page is generated from the command metadata in `libs/shlang/src/doc/` (`cargo xtask docs`) - do not edit it by hand. `help <command>` in the terminal shows the same data and the examples are tests: they run in the test folder `/home` (files `a.txt`, `b.txt`, `data.json` and the folder `docs`) and their output is compared.\n\n",
    ));
    o.push_str(t("## Rejstřík\n\n", "## Index\n\n"));
    for (gi, (title, names)) in GROUPS.iter().enumerate() {
        let title = if en { GROUPS_EN.get(gi).copied().unwrap_or(title) } else { title };
        o.push_str(&alloc::format!("- **{title}:** "));
        let links: Vec<String> = names.iter().map(|n| alloc::format!("[`{n}`](#{n})")).collect();
        o.push_str(&links.join(", "));
        o.push('\n');
    }
    for (gi, (title, names)) in GROUPS.iter().enumerate() {
        let title = if en { GROUPS_EN.get(gi).copied().unwrap_or(title) } else { title };
        o.push_str(&alloc::format!("\n## {title}\n"));
        for n in *names {
            let Some(s) = sig(n) else { continue };
            let d = get(n);
            let short = match (en, d) {
                (true, Some(d)) => d.short_en,
                (true, None) => "*(missing English text)*",
                (false, _) => s.help,
            };
            // Kotva je slug nadpisu: `#ls`, `#starts-with`.
            o.push_str(&alloc::format!("\n### `{n}`\n\n{short}\n\n"));
            if let Some(d) = d {
                o.push_str(if en { d.long_en } else { d.long_cs });
                o.push_str("\n\n");
            }
            o.push_str(&alloc::format!("```shell\n{}\n```\n\n", crate::parse::usage(s)));
            o.push_str(&alloc::format!(
                "{} `{}` · {} {} · {} {}\n\n",
                t("Plné jméno", "Full name"),
                s.full,
                t("vstup:", "input:"),
                io_name(s.input, en),
                t("výstup:", "output:"),
                io_name(s.output, en)
            ));
            let rows = arg_rows(s, d, en);
            if !rows.is_empty() {
                o.push_str(t("| Argument | Typ | Výchozí | Popis |\n|---|---|---|---|\n", "| Argument | Type | Default | Description |\n|---|---|---|---|\n"));
                for (n, ty, def, text) in &rows {
                    o.push_str(&alloc::format!("| `{n}` | {ty} | {def} | {text} |\n"));
                }
                o.push('\n');
            }
            if let Some(d) = d {
                for e in d.examples {
                    o.push_str(&alloc::format!("{}\n\n```shell\n{}\n```\n\n", if en { e.en } else { e.cs }, e.code));
                    if let Check::Out(out) = e.check
                        && !out.is_empty()
                    {
                        o.push_str(&alloc::format!("```text\n{out}\n```\n\n"));
                    }
                }
                let needs = if en { d.needs_en } else { d.needs_cs };
                if !needs.is_empty() {
                    o.push_str(&alloc::format!("{} {needs}\n\n", t("**Potřebuje:**", "**Needs:**")));
                }
                if !d.errors.is_empty() {
                    let codes: Vec<String> = d.errors.iter().map(|c| alloc::format!("`{c}`")).collect();
                    o.push_str(&alloc::format!("{} {}\n\n", t("**Chyby:**", "**Errors:**"), codes.join(", ")));
                }
                if !d.related.is_empty() {
                    let links: Vec<String> = d.related.iter().map(|r| alloc::format!("[`{r}`](#{r})")).collect();
                    o.push_str(&alloc::format!("{} {}\n\n", t("**Souvisí:**", "**Related:**"), links.join(", ")));
                }
            }
        }
    }
    o
}
