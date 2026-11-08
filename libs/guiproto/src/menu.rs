//! Deklarace menu aplikace (`docs/cs/gui.md`, Menu aplikaci a paleta
//! prikazu): aplikace menu jen popise, kresli ho system (paleta prikazu,
//! pozdeji menu v titulku). Polozky jsou akce z API akci (id akce
//! z manifestu a pevne argumenty), kompozitor ji spusti stejnou cestou
//! jako volani `actions` (overeni argumentu, potvrzeni u `confirm`).
//!
//! Zprava `MENU_UPDATE { window, items }`: strom; polozka na nejvyssi
//! urovni s detmi je nabidka (skupina), polozka bez akce s detmi podmenu.

use alloc::string::String;
use alloc::vec::Vec;

use crate::action::{Arg, decode_args, encode_args};
use crate::{Error, Reader, Result, Writer};

/// Nejvic polozek celeho menu, hloubka stromu a delka textu.
pub const MAX_ITEMS: usize = 256;
pub const MAX_DEPTH: usize = 3;
pub const MAX_TEXT: usize = 96;

/// Role polozky se systemovym textem a zkratkou (jednotne ve vsech
/// aplikacich).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Role {
    #[default]
    None,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
}

impl Role {
    fn code(self) -> u8 {
        match self {
            Role::None => 0,
            Role::Undo => 1,
            Role::Redo => 2,
            Role::Cut => 3,
            Role::Copy => 4,
            Role::Paste => 5,
            Role::SelectAll => 6,
        }
    }

    fn from_code(c: u8) -> Result<Self> {
        Ok(match c {
            0 => Role::None,
            1 => Role::Undo,
            2 => Role::Redo,
            3 => Role::Cut,
            4 => Role::Copy,
            5 => Role::Paste,
            6 => Role::SelectAll,
            _ => return Err(Error::InvalidArgs),
        })
    }

    /// Jednotny text a zkratka role (prebiji text aplikace).
    pub fn system_text(self) -> Option<(&'static str, &'static str)> {
        Some(match self {
            Role::None => return None,
            Role::Undo => ("Zpět", "Ctrl+Z"),
            Role::Redo => ("Znovu", "Ctrl+Shift+Z"),
            Role::Cut => ("Vyjmout", "Ctrl+X"),
            Role::Copy => ("Kopírovat", "Ctrl+C"),
            Role::Paste => ("Vložit", "Ctrl+V"),
            Role::SelectAll => ("Vybrat vše", "Ctrl+A"),
        })
    }
}

/// Polozka menu.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MenuItem {
    pub label: String,
    /// Id akce z manifestu (prazdne = nabidka nebo podmenu).
    pub action: String,
    /// Pevne argumenty volani ("Pricist 1": `by = 1`).
    pub args: Vec<Arg>,
    pub role: Role,
    /// Zaskrtnuti: `None` = neni prepinac.
    pub checked: Option<bool>,
    /// Zakazana polozka s duvodem (prazdny = povolena).
    pub disabled: String,
    /// Zkratka pro zobrazeni ("Ctrl+S").
    pub shortcut: String,
    /// Klicova slova a synonyma pro hledani.
    pub keywords: Vec<String>,
    pub children: Vec<MenuItem>,
}

impl MenuItem {
    pub fn encode(&self, w: &mut Writer) {
        w.str(&self.label).str(&self.action);
        encode_args(w, &self.args);
        w.u8(self.role.code()).u8(match self.checked {
            None => 0,
            Some(false) => 1,
            Some(true) => 2,
        });
        w.str(&self.disabled).str(&self.shortcut);
        w.u32(self.keywords.len() as u32);
        for k in &self.keywords {
            w.str(k);
        }
        w.u32(self.children.len() as u32);
        for c in &self.children {
            c.encode(w);
        }
    }

    pub fn decode(r: &mut Reader<'_>, msg_len: usize, depth: usize, count: &mut usize) -> Result<Self> {
        *count += 1;
        if depth > MAX_DEPTH || *count > MAX_ITEMS {
            return Err(Error::InvalidArgs);
        }
        let text = |r: &mut Reader<'_>| -> Result<String> {
            let s = r.str()?;
            if s.len() > MAX_TEXT { Err(Error::InvalidArgs) } else { Ok(s) }
        };
        let label = text(r)?;
        let action = text(r)?;
        let args = decode_args(r, msg_len)?;
        let role = Role::from_code(r.u8()?)?;
        let checked = match r.u8()? {
            0 => None,
            1 => Some(false),
            2 => Some(true),
            _ => return Err(Error::InvalidArgs),
        };
        let disabled = text(r)?;
        let shortcut = text(r)?;
        let n = crate::read_count(r, 4, msg_len)?;
        let mut keywords = Vec::with_capacity(n);
        for _ in 0..n {
            keywords.push(text(r)?);
        }
        let n = crate::read_count(r, 4, msg_len)?;
        let mut children = Vec::with_capacity(n.min(MAX_ITEMS));
        for _ in 0..n {
            children.push(MenuItem::decode(r, msg_len, depth + 1, count)?);
        }
        Ok(Self { label, action, args, role, checked, disabled, shortcut, keywords, children })
    }
}

/// Zakoduje cele menu (seznam nabidek).
pub fn encode_menu(w: &mut Writer, items: &[MenuItem]) {
    w.u32(items.len() as u32);
    for i in items {
        i.encode(w);
    }
}

pub fn decode_menu(r: &mut Reader<'_>, msg_len: usize) -> Result<Vec<MenuItem>> {
    let n = crate::read_count(r, 4, msg_len)?;
    let mut count = 0;
    let mut out = Vec::with_capacity(n.min(MAX_ITEMS));
    for _ in 0..n {
        out.push(MenuItem::decode(r, msg_len, 1, &mut count)?);
    }
    Ok(out)
}

/// Polozka menu po rozbaleni stromu: cesta ("Upravy › Kopirovat") a data.
#[derive(Debug, Clone, PartialEq)]
pub struct Flat {
    pub path: String,
    pub item: MenuItem,
}

/// Spustitelne polozky (s akci) v poradi menu, s cestou nabidek.
pub fn flatten(items: &[MenuItem]) -> Vec<Flat> {
    fn walk(out: &mut Vec<Flat>, prefix: &str, items: &[MenuItem]) {
        for i in items {
            let label = i.role.system_text().map_or(i.label.as_str(), |(t, _)| t);
            let path = if prefix.is_empty() { String::from(label) } else { alloc::format!("{prefix} › {label}") };
            if !i.action.is_empty() {
                out.push(Flat { path: path.clone(), item: MenuItem { children: Vec::new(), ..i.clone() } });
            }
            walk(out, &path, &i.children);
        }
    }
    let mut out = Vec::new();
    walk(&mut out, "", items);
    out
}
