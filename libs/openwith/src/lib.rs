//! "Otevri cestu": ktera aplikace otevre soubor nebo slozku.
//!
//! Aplikace to deklaruje v manifestu (`<program>.manifest.toml`):
//!
//! ```toml
//! [open]
//! action = "editor.open"        # akce manifestu s parametrem `path` typu path
//! extensions = ["txt", "md"]    # pripony (bez tecky, bez ohledu na velikost)
//! folders = true                # otevira slozky (spravce souboru)
//! reveal = true                 # soubor bez jine aplikace ukaze (spravce)
//! ```
//!
//! [`Registry::resolve`]: slozka -> aplikace s `folders`; soubor -> prvni
//! aplikace s jeho priponou, jinak aplikace s `reveal` (soubor se ukaze ve
//! spravci souboru). Akce musi byt v manifestu deklarovana s povinnym
//! parametrem `path` typu `path`, jinak se `[open]` ignoruje (chyba se
//! vrati z [`parse`]). Volbu "Otevrit v..." (dialog) resi system pozdeji.

#![no_std]

extern crate alloc;

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests;

use alloc::string::String;
use alloc::vec::Vec;

use guiproto::action::ParamType;

/// Asociace jedne aplikace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assoc {
    /// Program v initfs (`gui-files`).
    pub program: String,
    /// Id aplikace z manifestu (`org.magnumopus.files`).
    pub app: String,
    pub action: String,
    /// Pripony malymi pismeny.
    pub extensions: Vec<String>,
    pub folders: bool,
    pub reveal: bool,
}

/// Co s cestou udelat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target<'a> {
    /// Otevrit v aplikaci (slozka nebo soubor s jeji priponou).
    Open(&'a Assoc),
    /// Zadna aplikace soubor neotevre: ukazat ho (spravce souboru).
    Reveal(&'a Assoc),
}

impl Target<'_> {
    pub fn assoc(&self) -> &Assoc {
        match self {
            Target::Open(a) | Target::Reveal(a) => a,
        }
    }
}

/// Asociace z manifestu programu; `Ok(None)` = manifest `[open]` nema.
pub fn parse(program: &str, manifest: &str) -> Result<Option<Assoc>, String> {
    let t = conf::parse(manifest).map_err(|e| alloc::format!("{program}: manifest: {e:?}"))?;
    if t.get_table("open").is_none() {
        return Ok(None);
    }
    let app = t.get_str("app.id").ok_or_else(|| alloc::format!("{program}: [open] bez app.id"))?;
    let action = t.get_str("open.action").ok_or_else(|| alloc::format!("{program}: [open] bez action"))?;
    let actions = guiproto::action::parse_table(&t).map_err(|e| alloc::format!("{program}: {}", e.0))?;
    let decl = actions.iter().find(|a| a.id == action).ok_or_else(|| alloc::format!("{program}: [open] action {action} neni deklarovana"))?;
    if !decl.params.iter().any(|p| p.name == "path" && p.ty == ParamType::Path && p.required) {
        return Err(alloc::format!("{program}: akce {action} nema povinny parametr path typu path"));
    }
    let extensions: Vec<String> = t
        .get_strs("open.extensions")
        .unwrap_or_default()
        .into_iter()
        .map(|e| e.trim_start_matches('.').to_ascii_lowercase())
        .filter(|e| !e.is_empty())
        .collect();
    Ok(Some(Assoc {
        program: String::from(program),
        app: String::from(app),
        action: String::from(action),
        extensions,
        folders: t.get_bool("open.folders").unwrap_or(false),
        reveal: t.get_bool("open.reveal").unwrap_or(false),
    }))
}

/// Pripona souboru malymi pismeny (`""` bez pripony; jmeno zacinajici
/// teckou bez dalsi tecky priponu nema).
pub fn extension(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or("");
    match name.rfind('.') {
        Some(i) if i > 0 => name[i + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// Asociace vsech aplikaci (poradi = poradi manifestu, prvni vyhrava).
#[derive(Debug, Clone, Default)]
pub struct Registry {
    pub list: Vec<Assoc>,
}

impl Registry {
    /// Z manifestu `(program, text)`; vadne `[open]` se preskoci a vrati
    /// jako chyby (do logu).
    pub fn build<'a>(manifests: impl IntoIterator<Item = (&'a str, &'a str)>) -> (Self, Vec<String>) {
        let mut list = Vec::new();
        let mut errors = Vec::new();
        for (p, text) in manifests {
            match parse(p, text) {
                Ok(Some(a)) => list.push(a),
                Ok(None) => {}
                Err(e) => errors.push(e),
            }
        }
        (Self { list }, errors)
    }

    pub fn resolve(&self, path: &str, is_dir: bool) -> Option<Target<'_>> {
        if is_dir {
            return self.list.iter().find(|a| a.folders).map(Target::Open);
        }
        let ext = extension(path);
        if !ext.is_empty()
            && let Some(a) = self.list.iter().find(|a| a.extensions.iter().any(|e| *e == ext))
        {
            return Some(Target::Open(a));
        }
        self.list.iter().find(|a| a.reveal).map(Target::Reveal)
    }
}

/// Zpravy sluzby "otevri cestu" (v initu, kanal relace): zadost
/// `open <cesta>` s jednim handlem (kanal pro odpoved), odpoved
/// [`Reply`]. Text UTF-8; cesta muze obsahovat mezery.
pub mod msg {
    use alloc::string::String;
    use alloc::vec::Vec;

    pub fn request(path: &str) -> Vec<u8> {
        alloc::format!("open {path}").into_bytes()
    }

    /// Cesta ze zadosti (`None` = neplatna zprava).
    pub fn parse_request(b: &[u8]) -> Option<&str> {
        core::str::from_utf8(b).ok()?.strip_prefix("open ").filter(|p| p.starts_with('/'))
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Reply {
        /// Otevreno v aplikaci (id).
        Opened(String),
        /// Zadna aplikace soubor neotevre; ukazan ve spravci (id).
        Revealed(String),
        /// Chyba (text pro cloveka).
        Failed(String),
    }

    impl Reply {
        pub fn encode(&self) -> Vec<u8> {
            match self {
                Reply::Opened(a) => alloc::format!("ok {a}"),
                Reply::Revealed(a) => alloc::format!("reveal {a}"),
                Reply::Failed(m) => alloc::format!("err {m}"),
            }
            .into_bytes()
        }

        pub fn decode(b: &[u8]) -> Option<Self> {
            let s = core::str::from_utf8(b).ok()?;
            let (k, rest) = s.split_once(' ')?;
            let rest = String::from(rest);
            match k {
                "ok" => Some(Reply::Opened(rest)),
                "reveal" => Some(Reply::Revealed(rest)),
                "err" => Some(Reply::Failed(rest)),
                _ => None,
            }
        }
    }
}
