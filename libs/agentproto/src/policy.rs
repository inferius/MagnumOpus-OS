//! Pravidla relace: co smi agent volne, co se potvrzuje a co ne.
//!
//! Toto je **prvni** vrstva ochrany (srozumitelne odmitnuti a dotaz
//! uzivateli). Druha vrstva jsou capability relace: `agentd` ma k souborum
//! uzivatele jen spojeni pro cteni a spojeni na pracovni prostor, takze
//! zapis mimo prostor neprojde, ani kdyby tato pravidla selhala.
//!
//! Cesty ve vstupu nastroju jsou v polozkach `path`, `from`, `to` a `dir`
//! (konvence pro vsechny nastroje). Cesta musi byt absolutni, bez `.`
//! a `..`; jinak se volani odmitne.

use alloc::string::String;
use alloc::vec::Vec;

use crate::json::Value;
use crate::tool::{Effect, ToolSpec};
use crate::{Error, Result};

/// Rezim relace: urcuje sadu nastroju.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Vychozi: cteni, zapis (v prostoru volne, jinak potvrdit), systemove nastroje.
    Chat,
    /// Pomocnik v shellu: jen cteni a navrh prikazu.
    Shell,
    /// Diagnostika: jen cteni.
    Diag,
    /// Vyvoj: pracovni prostor projektu, preklad a testy (az bude prekladac).
    Dev,
}

impl Mode {
    pub fn code(self) -> u8 {
        match self {
            Mode::Chat => 1,
            Mode::Shell => 2,
            Mode::Diag => 3,
            Mode::Dev => 4,
        }
    }

    pub fn from_code(c: u8) -> Result<Self> {
        Ok(match c {
            1 => Mode::Chat,
            2 => Mode::Shell,
            3 => Mode::Diag,
            4 => Mode::Dev,
            _ => return Err(Error::InvalidArgs),
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Chat => "chat",
            Mode::Shell => "shell",
            Mode::Diag => "diag",
            Mode::Dev => "dev",
        }
    }

    /// Nabizi rezim tento nastroj?
    pub fn offers(self, tool: &ToolSpec) -> bool {
        let group = tool.name.split('.').next().unwrap_or("");
        match self {
            Mode::Shell => matches!(tool.effect, Effect::Read | Effect::Propose) && group != "dev",
            Mode::Diag => tool.effect == Effect::Read,
            Mode::Chat => group != "shell" && group != "dev",
            Mode::Dev => group != "shell",
        }
    }
}

/// Co smi cist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadScope {
    /// Vse, co smi uzivatel (bez cest, ktere neopusti pocitac).
    User,
    /// Jen pracovni prostor (citliva data).
    Workspace,
}

/// Vysledek pravidel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Volne (cteni, navrh).
    Allow,
    /// Volne v pracovnim prostoru (zapis pres capability prostoru, commit kroku).
    Workspace,
    /// Zeptat se uzivatele (klient akci provede sam).
    Confirm,
    /// Odmitnout (duvod pro model a audit).
    Deny(String),
}

impl Decision {
    pub fn name(&self) -> &'static str {
        match self {
            Decision::Allow => "allow",
            Decision::Workspace => "allow-ws",
            Decision::Confirm => "confirm",
            Decision::Deny(_) => "deny",
        }
    }
}

/// Cesty, ktere nikdy neopusti pocitac (nastroje je neprectou).
pub const NEVER_LEAVE: &[&str] = &[
    "/system/agent/**",
    "/system/keys/**",
    "**/.ssh/**",
    "**/.gnupg/**",
    "**/*.key",
    "**/*.pem",
    "**/.config/agent/**",
    "**/hesla/**",
    "**/passwords/**",
];

/// Pravidla jedne relace.
#[derive(Debug, Clone)]
pub struct Policy {
    pub mode: Mode,
    /// Pracovni prostor (normalizovana cesta), pokud je.
    pub workspace: Option<String>,
    pub read_scope: ReadScope,
    /// Vzory navic k [`NEVER_LEAVE`] (seznam jde jen rozsirit).
    pub never_leave: Vec<String>,
    /// Ma klient uzivatelske rozhrani pro potvrzeni? (bez nej = odmitnout)
    pub can_confirm: bool,
}

/// Normalizovana absolutni cesta: bez `.` a `..`, bez zdvojenych
/// a koncovych lomitek.
pub fn normalize(path: &str) -> Result<String> {
    if !path.starts_with('/') || path.len() > 4096 || path.contains('\0') {
        return Err(Error::InvalidArgs);
    }
    let mut out = String::new();
    for seg in path.split('/') {
        match seg {
            "" => continue,
            "." | ".." => return Err(Error::InvalidArgs),
            s => {
                out.push('/');
                out.push_str(s);
            }
        }
    }
    if out.is_empty() {
        out.push('/');
    }
    Ok(out)
}

/// Lezi `path` v `base` (stejna nebo pod ni)? Obe normalizovane.
pub fn under(path: &str, base: &str) -> bool {
    base == "/" || path == base || path.strip_prefix(base).is_some_and(|r| r.starts_with('/'))
}

/// Shoda segmentu se vzorem (`*` = libovolny usek uvnitr jmena).
fn seg_match(pat: &str, s: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == s;
    }
    let mut rest = s;
    for (i, p) in parts.iter().enumerate() {
        if i == 0 {
            match rest.strip_prefix(p) {
                Some(r) => rest = r,
                None => return false,
            }
        } else if i == parts.len() - 1 {
            return rest.ends_with(p);
        } else {
            match rest.find(p) {
                Some(at) => rest = &rest[at + p.len()..],
                None => return false,
            }
        }
    }
    true
}

fn glob_segs(pat: &[&str], path: &[&str]) -> bool {
    match pat.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|i| glob_segs(rest, &path[i..])),
        Some((p, rest)) => !path.is_empty() && seg_match(p, path[0]) && glob_segs(rest, &path[1..]),
    }
}

/// Odpovida cesta vzoru? (`**` = libovolny pocet segmentu)
pub fn glob(pattern: &str, path: &str) -> bool {
    let p: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let s: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    glob_segs(&p, &s)
}

/// Cesty ve vstupu nastroje (polozky `path`, `from`, `to`, `dir`).
pub fn paths_of(input: &Value) -> Vec<String> {
    ["path", "from", "to", "dir"].iter().filter_map(|k| input.get(k).and_then(Value::as_str)).map(String::from).collect()
}

impl Policy {
    /// Neopusti cesta pocitac? (vychozi vzory + vzory relace)
    pub fn never_leaves(&self, path: &str) -> bool {
        NEVER_LEAVE.iter().any(|p| glob(p, path)) || self.never_leave.iter().any(|p| glob(p, path))
    }

    pub fn in_workspace(&self, path: &str) -> bool {
        self.workspace.as_deref().is_some_and(|w| under(path, w))
    }

    /// Rozhodnuti o volani nastroje `tool` se vstupem `input`.
    pub fn decide(&self, tool: &ToolSpec, input: &Value) -> Decision {
        if !self.mode.offers(tool) {
            return Decision::Deny(alloc::format!("nastroj {} neni v rezimu {}", tool.name, self.mode.name()));
        }
        let mut paths = Vec::new();
        for p in paths_of(input) {
            match normalize(&p) {
                Ok(n) => paths.push(n),
                Err(_) => return Decision::Deny(alloc::format!("neplatna cesta '{p}' (musi byt absolutni, bez . a ..)")),
            }
        }
        if let Some(p) = paths.iter().find(|p| self.never_leaves(p)) {
            return Decision::Deny(alloc::format!("cesta {p} nesmi opustit pocitac"));
        }
        match tool.effect {
            Effect::Propose => Decision::Allow,
            Effect::Read => {
                if self.read_scope == ReadScope::Workspace
                    && let Some(p) = paths.iter().find(|p| !self.in_workspace(p))
                {
                    return Decision::Deny(alloc::format!("relace smi cist jen v pracovnim prostoru, ne {p}"));
                }
                Decision::Allow
            }
            Effect::Write => {
                if !paths.is_empty() && paths.iter().all(|p| self.in_workspace(p)) {
                    Decision::Workspace
                } else if self.can_confirm {
                    Decision::Confirm
                } else {
                    Decision::Deny(String::from("zapis mimo pracovni prostor vyzaduje potvrzeni a klient ho neumi"))
                }
            }
            Effect::Exec | Effect::Net | Effect::Settings => {
                if self.can_confirm {
                    Decision::Confirm
                } else {
                    Decision::Deny(alloc::format!("{} vyzaduje potvrzeni a klient ho neumi", tool.effect.name()))
                }
            }
        }
    }
}
