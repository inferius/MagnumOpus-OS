//! Akce aplikaci: deklarace v manifestu (typovane parametry, opravneni)
//! a typovane hodnoty argumentu.
//!
//! ```toml
//! [[action]]
//! id = "note.create"
//! title = "Nova poznamka"
//! needs = ["fs:write"]
//! confirm = false
//!
//! [[action.param]]
//! name = "title"
//! type = "text"            # text, int, bool, path, paths, choice
//! required = true
//! ```
//!
//! Kompozitor overi argumenty proti deklaraci ([`Action::check`]) driv,
//! nez je aplikace dostane: aplikace tak nikdy nevidi parametr spatneho
//! typu ani parametr, ktery nedeklarovala.

use alloc::string::String;
use alloc::vec::Vec;

use conf::Table;

use crate::{Error, ReadExt, Reader, Result, WriteExt, Writer};

/// Typ parametru.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamType {
    Text,
    Int,
    Bool,
    /// Cesta v souborovem systemu (text; opravneni k ni resi volajici).
    Path,
    /// Jedna z vyctenych hodnot.
    Choice(Vec<String>),
    /// Seznam cest (hromadna operace: dialog ukaze pocet a nahled).
    Paths,
}

impl ParamType {
    fn code(&self) -> u8 {
        match self {
            ParamType::Text => 1,
            ParamType::Int => 2,
            ParamType::Bool => 3,
            ParamType::Path => 4,
            ParamType::Choice(_) => 5,
            ParamType::Paths => 6,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            ParamType::Text => "text",
            ParamType::Int => "int",
            ParamType::Bool => "bool",
            ParamType::Path => "path",
            ParamType::Choice(_) => "choice",
            ParamType::Paths => "paths",
        }
    }
}

/// Typovana hodnota argumentu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Text(String),
    Int(i64),
    Bool(bool),
    Path(String),
    Choice(String),
    Paths(Vec<String>),
}

/// Nejvic cest v jednom argumentu `paths`.
pub const MAX_PATHS: usize = 4096;

impl Value {
    pub fn encode(&self, w: &mut Writer) {
        match self {
            Value::Text(s) => w.u8(1).str(s),
            Value::Int(v) => w.u8(2).i64(*v),
            Value::Bool(b) => w.u8(3).bool(*b),
            Value::Path(s) => w.u8(4).str(s),
            Value::Choice(s) => w.u8(5).str(s),
            Value::Paths(list) => {
                w.u8(6).u32(list.len().min(MAX_PATHS) as u32);
                for p in list.iter().take(MAX_PATHS) {
                    w.str(p);
                }
                w
            }
        };
    }

    pub fn decode(r: &mut Reader<'_>) -> Result<Self> {
        Ok(match r.u8()? {
            1 => Value::Text(r.str()?),
            2 => Value::Int(r.i64()?),
            3 => Value::Bool(r.bool()?),
            4 => Value::Path(r.str()?),
            5 => Value::Choice(r.str()?),
            6 => {
                let n = r.u32()? as usize;
                if n > MAX_PATHS {
                    return Err(Error::InvalidArgs);
                }
                let mut list = Vec::new();
                for _ in 0..n {
                    list.push(r.str()?);
                }
                Value::Paths(list)
            }
            _ => return Err(Error::InvalidArgs),
        })
    }

    fn matches(&self, t: &ParamType) -> bool {
        match (self, t) {
            (Value::Text(_), ParamType::Text)
            | (Value::Int(_), ParamType::Int)
            | (Value::Bool(_), ParamType::Bool)
            | (Value::Path(_), ParamType::Path)
            | (Value::Paths(_), ParamType::Paths) => true,
            (Value::Choice(c), ParamType::Choice(list)) => list.iter().any(|x| x == c),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub ty: ParamType,
    pub required: bool,
}

/// Deklarovana akce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub id: String,
    pub title: String,
    /// Opravneni, ktera akce potrebuje (ukazou se v dialogu potvrzeni).
    pub needs: Vec<String>,
    /// Pred provedenim potvrdit systemovym dialogem.
    pub confirm: bool,
    pub params: Vec<Param>,
}

/// Argument volani: jmeno parametru a hodnota.
pub type Arg = (String, Value);

/// Nejvic akci jedne aplikace a parametru jedne akce.
pub const MAX_ACTIONS: usize = 64;
pub const MAX_PARAMS: usize = 16;

fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
}

/// Chyba manifestu akci (s popisem).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestError(pub String);

fn merr(s: &str) -> ManifestError {
    ManifestError(s.into())
}

/// Precte akce z manifestu (sekce `[[action]]`); manifest bez akci =
/// prazdny seznam.
pub fn parse_manifest(text: &str) -> core::result::Result<Vec<Action>, ManifestError> {
    let table = conf::parse(text).map_err(|e| ManifestError(alloc::format!("{e:?}")))?;
    parse_table(&table)
}

/// Jako [`parse_manifest`], z uz prectene tabulky.
pub fn parse_table(table: &Table) -> core::result::Result<Vec<Action>, ManifestError> {
    let mut out: Vec<Action> = Vec::new();
    for t in table.tables("action") {
        if out.len() >= MAX_ACTIONS {
            return Err(merr("prilis mnoho akci"));
        }
        let id = t.get_str("id").ok_or_else(|| merr("akce bez id"))?;
        if !valid_id(id) {
            return Err(merr("neplatne id akce"));
        }
        if out.iter().any(|a| a.id == id) {
            return Err(merr("dvakrat stejne id akce"));
        }
        let title = t.get_str("title").unwrap_or(id);
        let needs = t.get_strs("needs").unwrap_or_default().into_iter().map(String::from).collect();
        let confirm = t.get_bool("confirm").unwrap_or(false);
        let mut params: Vec<Param> = Vec::new();
        for p in t.tables("param") {
            if params.len() >= MAX_PARAMS {
                return Err(merr("prilis mnoho parametru"));
            }
            let name = p.get_str("name").ok_or_else(|| merr("parametr bez jmena"))?;
            if !valid_id(name) || params.iter().any(|x| x.name == name) {
                return Err(merr("neplatne nebo opakovane jmeno parametru"));
            }
            let ty = match p.get_str("type").unwrap_or("text") {
                "text" => ParamType::Text,
                "int" => ParamType::Int,
                "bool" => ParamType::Bool,
                "path" => ParamType::Path,
                "paths" => ParamType::Paths,
                "choice" => {
                    let c: Vec<String> =
                        p.get_strs("choices").unwrap_or_default().into_iter().map(String::from).collect();
                    if c.is_empty() {
                        return Err(merr("parametr choice bez voleb"));
                    }
                    ParamType::Choice(c)
                }
                _ => return Err(merr("neznamy typ parametru")),
            };
            params.push(Param {
                name: name.into(),
                ty,
                required: p.get_bool("required").unwrap_or(false),
            });
        }
        out.push(Action {
            id: id.into(),
            title: title.into(),
            needs,
            confirm,
            params,
        });
    }
    Ok(out)
}

impl Action {
    /// Overi argumenty volani: kazdy argument je deklarovany parametr
    /// spravneho typu, zadny dvakrat, povinne nechybi.
    pub fn check(&self, args: &[Arg]) -> Result<()> {
        for (i, (name, v)) in args.iter().enumerate() {
            let p = self.params.iter().find(|p| &p.name == name).ok_or(Error::InvalidArgs)?;
            if !v.matches(&p.ty) || args[..i].iter().any(|(n, _)| n == name) {
                return Err(Error::InvalidArgs);
            }
        }
        if self.params.iter().any(|p| p.required && !args.iter().any(|(n, _)| n == &p.name)) {
            return Err(Error::InvalidArgs);
        }
        Ok(())
    }

    pub fn encode(&self, w: &mut Writer) {
        w.str(&self.id).str(&self.title).bool(self.confirm);
        w.u32(self.needs.len() as u32);
        for n in &self.needs {
            w.str(n);
        }
        w.u32(self.params.len() as u32);
        for p in &self.params {
            w.str(&p.name).u8(p.ty.code()).bool(p.required);
            if let ParamType::Choice(c) = &p.ty {
                w.u32(c.len() as u32);
                for x in c {
                    w.str(x);
                }
            }
        }
    }

    pub fn decode(r: &mut Reader<'_>, msg_len: usize) -> Result<Self> {
        let id = r.str()?;
        let title = r.str()?;
        let confirm = r.bool()?;
        let n = crate::read_count(r, 2, msg_len)?;
        let mut needs = Vec::with_capacity(n);
        for _ in 0..n {
            needs.push(r.str()?);
        }
        let n = crate::read_count(r, 4, msg_len)?;
        let mut params = Vec::with_capacity(n);
        for _ in 0..n {
            let name = r.str()?;
            let ty = match r.u8()? {
                1 => ParamType::Text,
                2 => ParamType::Int,
                3 => ParamType::Bool,
                4 => ParamType::Path,
                5 => ParamType::Choice(Vec::new()),
                6 => ParamType::Paths,
                _ => return Err(Error::InvalidArgs),
            };
            let required = r.bool()?;
            let ty = if let ParamType::Choice(_) = ty {
                let k = crate::read_count(r, 2, msg_len)?;
                let mut c = Vec::with_capacity(k);
                for _ in 0..k {
                    c.push(r.str()?);
                }
                ParamType::Choice(c)
            } else {
                ty
            };
            params.push(Param { name, ty, required });
        }
        Ok(Self {
            id,
            title,
            needs,
            confirm,
            params,
        })
    }
}

/// Zakoduje argumenty (pocet + jmeno a hodnota).
pub fn encode_args(w: &mut Writer, args: &[Arg]) {
    w.u32(args.len() as u32);
    for (n, v) in args {
        w.str(n);
        v.encode(w);
    }
}

pub fn decode_args(r: &mut Reader<'_>, msg_len: usize) -> Result<Vec<Arg>> {
    let n = crate::read_count(r, 4, msg_len)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let name = r.str()?;
        out.push((name, Value::decode(r)?));
    }
    Ok(out)
}
