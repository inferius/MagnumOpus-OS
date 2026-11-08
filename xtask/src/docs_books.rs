//! Knihy webu dokumentace: definice knih, cteni rozcestniku (README)
//! a mapovani vystupnich cest.
//!
//! Web ma dve knihy se stejnou stavbou (`<kniha>/cs`, `<kniha>/en`):
//! - `prirucka` (uzivatelska prirucka, vychozi stranka) v `docs/prirucka/`
//!   s vystupem do `target/docs/prirucka/{cs,en}/`,
//! - `dev` (dokumentace pro vyvojare) v `docs/` s vystupem do
//!   `target/docs/{cs,en}/` (kvuli stavajicim odkazum zustava na miste).

use std::fs;
use std::path::Path;

use crate::Result;

/// Jedna kniha: kde jsou zdroje a kam jde vystup.
pub struct BookDef {
    /// Zdrojova slozka relativne ke korenu repozitare (obsahuje README, `cs/`, `en/`).
    pub src: &'static str,
    /// Podslozka vystupu (`""` pro vyvojarskou knihu).
    pub out_sub: &'static str,
    /// Nazev knihy v zahlavi (cs, en).
    pub names: [&'static str; 2],
    /// Titulek a uvod rozcestniku knihy (cs, en).
    pub titles: [&'static str; 2],
    pub intros: [&'static str; 2],
    /// Predpona odkazu do `design/` v markdownu (relativne ke zdrojove strance).
    pub design_src: &'static str,
    /// Tataz predpona na webu (relativne k vystupni strance).
    pub design_web: &'static str,
}

/// Knihy webu; poradi = poradi prepinace v zahlavi. Prvni je vychozi.
pub const BOOKS: [BookDef; 2] = [
    BookDef {
        src: "docs/prirucka",
        out_sub: "prirucka",
        names: ["Příručka", "User guide"],
        titles: ["Příručka", "User guide"],
        intros: [
            "Příručka pro běžné uživatele: co udělat a kde to najdete.",
            "A guide for everyday users: what to do and where to find it.",
        ],
        design_src: "../../../design/",
        design_web: "../../design/",
    },
    BookDef {
        src: "docs",
        out_sub: "",
        names: ["Pro vývojáře", "For developers"],
        titles: ["Dokumentace operačního systému", "Operating system documentation"],
        intros: [
            "Nový operační systém psaný od nuly v Rustu: mikrokernel, capabilities, ovladače a služby v userspace, později Windows aplikace v izolované vrstvě.",
            "A new operating system written from scratch in Rust: a microkernel, capabilities, drivers and services in userspace, later Windows applications in an isolated layer.",
        ],
        design_src: "../../design/",
        design_web: "../design/",
    },
];

impl BookDef {
    /// Identifikator knihy (pro soubor vyhledavani a hlasky).
    pub fn id(&self) -> &'static str {
        if self.out_sub.is_empty() { "dev" } else { self.out_sub }
    }

    /// Webova slozka jazyka relativne k `target/docs` (`prirucka/cs`, `cs`).
    pub fn web_dir(&self, lang: &str) -> String {
        if self.out_sub.is_empty() {
            lang.to_string()
        } else {
            format!("{}/{lang}", self.out_sub)
        }
    }

    /// Predpona `../` z webove slozky jazyka ke korenu webu.
    pub fn root_prefix(&self) -> &'static str {
        if self.out_sub.is_empty() { "../" } else { "../../" }
    }

    /// Soubor s indexem vyhledavani relativne k `assets/`.
    pub fn search_file(&self, lang: &str) -> String {
        if self.out_sub.is_empty() {
            format!("search-{lang}.js")
        } else {
            format!("search-{}-{lang}.js", self.out_sub)
        }
    }
}

/// Jedna stranka v obou jazycich.
pub struct PageDef {
    pub files: [String; 2],
    /// Nazev v menu (text odkazu v README).
    pub names: [String; 2],
    pub descriptions: [String; 2],
    /// Skupina v menu (nadpis `###` v README).
    pub group: [String; 2],
}

/// Precte stranky z README knihy: skupiny `### cs / en` a radky
/// `| [Nazev](cs/x.md) | [Name](en/y.md) | popis cs / popis en |`.
pub fn read_index(path: &Path) -> Result<Vec<PageDef>> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_index(&text).ok_or_else(|| format!("{} neobsahuje tabulku stranek", path.display()).into())
}

/// Cista cast cteni rozcestniku (testovatelna bez souboru).
pub fn parse_index(text: &str) -> Option<Vec<PageDef>> {
    let mut pages = Vec::new();
    let mut group = [String::new(), String::new()];
    for line in text.lines() {
        if let Some(h) = line.strip_prefix("### ") {
            let (cs, en) = h.split_once(" / ").unwrap_or((h, h));
            group = [cs.trim().to_string(), en.trim().to_string()];
            continue;
        }
        if !line.starts_with("| [") {
            continue;
        }
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() < 3 {
            continue;
        }
        let target = |cell: &str| -> Option<String> {
            let start = cell.find("](")? + 2;
            let end = cell[start..].find(')')? + start;
            Some(cell[start..end].rsplit('/').next()?.to_string())
        };
        let (Some(cs), Some(en)) = (target(cells[0]), target(cells[1])) else {
            continue;
        };
        let name = |cell: &str| -> String {
            cell.find("](").map_or(String::new(), |e| cell[1..e].trim_start_matches('[').to_string())
        };
        let (dcs, den) = cells[2].split_once(" / ").unwrap_or((cells[2], cells[2]));
        pages.push(PageDef {
            files: [cs, en],
            names: [name(cells[0]), name(cells[1])],
            descriptions: [dcs.to_string(), den.to_string()],
            group: group.clone(),
        });
    }
    if pages.is_empty() { None } else { Some(pages) }
}
