//! Popis nastroju (strojove citelny) a overeni vstupu proti schematu.
//!
//! Schema je podmnozina JSON Schema, kterou poskytovatele modelu znaji:
//! `type: object` s `properties` a `required`; vlastnosti typu `string`,
//! `integer`, `boolean`, `array` (s `items`) a `object`; `enum` u retezcu;
//! `maxLength` u retezcu. Vstup od modelu se overi **pred** volanim
//! nastroje - neplatny vstup se vrati modelu jako chyba nastroje.

use alloc::string::String;
use alloc::vec::Vec;

use crate::json::Value;
use crate::{Error, Result};

/// Druh ucinku nastroje: podle nej pravidla rozhoduji o potvrzeni.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Jen cteni (soubory, stav systemu).
    Read,
    /// Zapis do souboroveho systemu.
    Write,
    /// Spusteni programu nebo prikazu.
    Exec,
    /// Sit.
    Net,
    /// Zmena nastaveni systemu.
    Settings,
    /// Jen navrh pro uzivatele (nic nemeni).
    Propose,
}

impl Effect {
    pub fn code(self) -> u8 {
        match self {
            Effect::Read => 1,
            Effect::Write => 2,
            Effect::Exec => 3,
            Effect::Net => 4,
            Effect::Settings => 5,
            Effect::Propose => 6,
        }
    }

    pub fn from_code(c: u8) -> Result<Self> {
        Ok(match c {
            1 => Effect::Read,
            2 => Effect::Write,
            3 => Effect::Exec,
            4 => Effect::Net,
            5 => Effect::Settings,
            6 => Effect::Propose,
            _ => return Err(Error::InvalidArgs),
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Effect::Read => "read",
            Effect::Write => "write",
            Effect::Exec => "exec",
            Effect::Net => "net",
            Effect::Settings => "settings",
            Effect::Propose => "propose",
        }
    }
}

/// Popis jednoho nastroje.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    /// `skupina.akce` (a-z, 0-9, `.`, `_`), nejvys 64 znaku.
    pub name: String,
    /// Popis pro model (k cemu nastroj je, co vraci).
    pub description: String,
    /// Schema vstupu (objekt JSON Schema).
    pub schema: Value,
    pub effect: Effect,
}

/// Plati jmeno nastroje? (poskytovatele maji omezeni znaku a delky)
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_')
        && !name.starts_with('.')
}

impl ToolSpec {
    pub fn new(name: &str, description: &str, effect: Effect, schema: Value) -> Self {
        Self { name: String::from(name), description: String::from(description), schema, effect }
    }

    /// Jmeno pro poskytovatele, ktery tecku nepovoli (Claude: `^[a-zA-Z0-9_-]{1,64}$`).
    pub fn wire_name(&self) -> String {
        wire_name(&self.name)
    }

    pub fn put(&self, w: &mut wire::Writer) {
        w.str(&self.name).str(&self.description).u8(self.effect.code()).str(&self.schema.to_json());
    }

    pub fn get(r: &mut wire::Reader) -> Result<Self> {
        let name = r.str()?;
        if !valid_name(&name) {
            return Err(Error::InvalidArgs);
        }
        let description = r.str()?;
        let effect = Effect::from_code(r.u8()?)?;
        let schema = crate::json::parse(&r.str()?).map_err(|_| Error::InvalidArgs)?;
        if schema.get("type").and_then(Value::as_str) != Some("object") {
            return Err(Error::InvalidArgs);
        }
        Ok(Self { name, description, schema, effect })
    }
}

/// `fs.read` -> `fs__read` (zpet [`from_wire_name`]).
pub fn wire_name(name: &str) -> String {
    name.replace('.', "__")
}

pub fn from_wire_name(name: &str) -> String {
    name.replace("__", ".")
}

/// Stavitel schematu objektu.
pub struct Schema {
    props: Vec<(String, Value)>,
    required: Vec<Value>,
}

impl Schema {
    pub fn object() -> Self {
        Self { props: Vec::new(), required: Vec::new() }
    }

    fn prop(mut self, name: &str, ty: &str, desc: &str, required: bool) -> Self {
        let v = Value::object().with("type", ty.into()).with("description", desc.into());
        self.props.push((String::from(name), v));
        if required {
            self.required.push(name.into());
        }
        self
    }

    pub fn string(self, name: &str, desc: &str, required: bool) -> Self {
        self.prop(name, "string", desc, required)
    }

    pub fn integer(self, name: &str, desc: &str, required: bool) -> Self {
        self.prop(name, "integer", desc, required)
    }

    pub fn boolean(self, name: &str, desc: &str, required: bool) -> Self {
        self.prop(name, "boolean", desc, required)
    }

    /// Pole retezcu.
    pub fn strings(mut self, name: &str, desc: &str, required: bool) -> Self {
        let v = Value::object()
            .with("type", "array".into())
            .with("description", desc.into())
            .with("items", Value::object().with("type", "string".into()));
        self.props.push((String::from(name), v));
        if required {
            self.required.push(name.into());
        }
        self
    }

    pub fn build(self) -> Value {
        Value::object()
            .with("type", "object".into())
            .with("properties", Value::Object(self.props))
            .with("required", Value::Array(self.required))
            .with("additionalProperties", Value::Bool(false))
    }
}

/// Overi vstup `input` proti schematu; chyba = popis pro model.
pub fn validate(schema: &Value, input: &Value) -> core::result::Result<(), String> {
    check(schema, input, "vstup", 0)
}

fn check(schema: &Value, v: &Value, at: &str, depth: usize) -> core::result::Result<(), String> {
    if depth > 16 {
        return Err(alloc::format!("{at}: prilis hluboke vnoreni"));
    }
    let ty = schema.get("type").and_then(Value::as_str).unwrap_or("");
    let ok = match (ty, v) {
        ("", _) => true,
        ("object", Value::Object(_)) | ("string", Value::Str(_)) | ("boolean", Value::Bool(_)) => true,
        ("array", Value::Array(_)) => true,
        ("integer", Value::Int(_)) => true,
        ("integer", Value::Float(f)) => *f == (*f as i64) as f64,
        ("number", Value::Int(_) | Value::Float(_)) => true,
        _ => false,
    };
    if !ok {
        return Err(alloc::format!("{at}: ocekavan typ {ty}, je {}", v.type_name()));
    }
    match v {
        Value::Object(items) => {
            let props = schema.get("properties");
            if let Some(req) = schema.get("required").and_then(Value::as_array) {
                for r in req {
                    if let Some(name) = r.as_str()
                        && !items.iter().any(|(k, _)| k == name)
                    {
                        return Err(alloc::format!("{at}: chybi povinna polozka '{name}'"));
                    }
                }
            }
            let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
            for (k, x) in items {
                match props.and_then(|p| p.get(k)) {
                    Some(s) => check(s, x, &alloc::format!("{at}.{k}"), depth + 1)?,
                    None if closed => return Err(alloc::format!("{at}: neznama polozka '{k}'")),
                    None => {}
                }
            }
        }
        Value::Array(items) => {
            if let Some(s) = schema.get("items") {
                for (i, x) in items.iter().enumerate() {
                    check(s, x, &alloc::format!("{at}[{i}]"), depth + 1)?;
                }
            }
        }
        Value::Str(s) => {
            if let Some(max) = schema.get("maxLength").and_then(Value::as_i64)
                && s.chars().count() as i64 > max
            {
                return Err(alloc::format!("{at}: retezec delsi nez {max} znaku"));
            }
            if let Some(opts) = schema.get("enum").and_then(Value::as_array)
                && !opts.iter().any(|o| o.as_str() == Some(s))
            {
                return Err(alloc::format!("{at}: hodnota '{s}' neni mezi povolenymi"));
            }
        }
        _ => {}
    }
    Ok(())
}
