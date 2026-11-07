//! Model pro generovani: jak se clen nebo parametr koduje (zakladni typ
//! a tvar), ktere typy jsou potreba (uzaver od zvolenych prikazu) a v jakem
//! smeru (vstup = kodovani, vystup = castecne kodovani + cteni).
//!
//! Pravidla odpovidaji generatoru venus-protocol (`vn_protocol.py`):
//! nekonstantni ukazatel parametru je vystup; vystup, ktery je delkou
//! jineho parametru, je vstup i vystup (pocet u vkEnumerate*); vystupni
//! handle a struktury jsou "castecne" (handle nese id od klienta,
//! struktura sType a pNext).

use std::collections::{BTreeMap, BTreeSet};

use super::registry::{Category, Member, Registry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prim {
    U8,
    U16,
    U32,
    U64,
    I32,
    I64,
    F32,
    F64,
}

impl Prim {
    pub fn from_c(name: &str) -> Option<Prim> {
        Some(match name {
            "uint8_t" | "char" => Prim::U8,
            "uint16_t" => Prim::U16,
            "uint32_t" => Prim::U32,
            "uint64_t" | "size_t" => Prim::U64,
            "int32_t" => Prim::I32,
            "int64_t" => Prim::I64,
            "float" => Prim::F32,
            "double" => Prim::F64,
            _ => return None,
        })
    }

    /// Typ v Rustu a jmeno metod `Encoder`/`Decoder`.
    pub fn rust(self) -> &'static str {
        match self {
            Prim::U8 => "u8",
            Prim::U16 => "u16",
            Prim::U32 => "u32",
            Prim::U64 => "u64",
            Prim::I32 => "i32",
            Prim::I64 => "i64",
            Prim::F32 => "f32",
            Prim::F64 => "f64",
        }
    }

    /// Bajtu na prvek v poli.
    pub fn size(self) -> usize {
        match self {
            Prim::U8 => 1,
            Prim::U16 => 2,
            Prim::U32 | Prim::I32 | Prim::F32 => 4,
            Prim::U64 | Prim::I64 | Prim::F64 => 8,
        }
    }
}

/// Zakladni typ clenu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Base {
    /// Skalar; `rust` = jmeno typu v Rustu (alias jako `VkBool32` nebo
    /// primo `u32`), `char` = pole znaku.
    Prim { prim: Prim, rust: String, char: bool },
    Handle(String),
    Struct(String),
    /// `void` (jen jako blob s delkou).
    Void,
}

impl Base {
    pub fn rust(&self) -> &str {
        match self {
            Base::Prim { rust, .. } => rust,
            Base::Handle(n) | Base::Struct(n) => n,
            Base::Void => "u8",
        }
    }
}

/// Tvar clenu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shape {
    Value,
    /// Pevne pole (pocet prvku).
    Static(usize),
    /// Ukazatel na jednu hodnotu.
    Ptr,
    /// Pole s delkou (vyraz v C nad ostatnimi cleny).
    Dyn(String),
    /// Retezec zakonceny NUL.
    Str,
    /// Pole retezcu s delkou.
    StrArray(String),
}

#[derive(Debug, Clone)]
pub struct Field {
    pub m: Member,
    pub base: Base,
    pub shape: Shape,
}

/// Prevod camelCase na snake_case (`pNext` -> `p_next`, `vendorID` -> `vendor_id`).
pub fn snake(name: &str) -> String {
    let c: Vec<char> = name.chars().collect();
    let mut s = String::new();
    for (i, &ch) in c.iter().enumerate() {
        if ch.is_ascii_uppercase() {
            let prev = if i > 0 { Some(c[i - 1]) } else { None };
            let next = c.get(i + 1).copied();
            let split = match prev {
                Some(p) if p.is_ascii_lowercase() || p.is_ascii_digit() => true,
                Some(p) if p.is_ascii_uppercase() => next.is_some_and(|n| n.is_ascii_lowercase()),
                _ => false,
            };
            if split {
                s.push('_');
            }
            s.push(ch.to_ascii_lowercase());
        } else {
            s.push(ch);
        }
    }
    const KEYWORDS: &[&str] = &[
        "type", "ref", "match", "fn", "mod", "use", "loop", "move", "box", "impl", "in", "as", "where", "struct",
        "enum", "trait", "self", "super", "crate", "const", "static", "let", "if", "else", "for", "while", "return",
        "break", "continue", "pub", "unsafe", "dyn", "async", "await", "yield", "true", "false",
    ];
    if KEYWORDS.contains(&s.as_str()) { format!("r#{s}") } else { s }
}

/// Jmeno prikazu bez `vk` ve snake_case (`vkCreateInstance` -> `create_instance`).
pub fn cmd_snake(name: &str) -> String {
    snake(name.strip_prefix("vk").unwrap_or(name))
}

/// Jmeno prikazu bez `vk` (`vkCreateInstance` -> `CreateInstance`).
pub fn cmd_camel(name: &str) -> String {
    name.strip_prefix("vk").unwrap_or(name).to_string()
}

impl Registry {
    /// Zakladni typ podle jmena v C.
    pub fn base(&self, name: &str) -> Result<Base, String> {
        if name == "void" {
            return Ok(Base::Void);
        }
        if let Some(p) = Prim::from_c(name) {
            return Ok(Base::Prim { prim: p, rust: p.rust().into(), char: name == "char" });
        }
        let canon = self.canonical(name).to_string();
        let t = self.ty(name).ok_or_else(|| format!("neznamy typ {name}"))?;
        Ok(match &t.category {
            Category::Struct => Base::Struct(canon),
            Category::Handle => Base::Handle(canon),
            Category::Enum => {
                let p = if self.enums64.contains(&canon) {
                    Prim::U64
                } else if canon.contains("FlagBits") {
                    Prim::U32
                } else {
                    Prim::I32
                };
                Base::Prim { prim: p, rust: canon, char: false }
            }
            Category::Bitmask { flags64, .. } => {
                Base::Prim { prim: if *flags64 { Prim::U64 } else { Prim::U32 }, rust: canon, char: false }
            }
            Category::Basetype(Some(target)) => {
                let p = Prim::from_c(target)
                    .or_else(|| match self.base(target) {
                        Ok(Base::Prim { prim, .. }) => Some(prim),
                        _ => None,
                    })
                    .ok_or_else(|| format!("basetype {name} -> {target} neni skalar"))?;
                Base::Prim { prim: p, rust: canon, char: false }
            }
            c => return Err(format!("typ {name} ({c:?}) neni podporovany")),
        })
    }

    /// Zakladni typ a tvar clenu.
    pub fn field(&self, m: &Member) -> Result<Field, String> {
        let base = self.base(&m.ty).map_err(|e| format!("{}: {e}", m.name))?;
        let first_len = m.len.first().cloned().unwrap_or_default();
        let shape = if let Some(size) = &m.array {
            if m.ptr != 0 {
                return Err(format!("{}: pole ukazatelu", m.name));
            }
            Shape::Static(self.array_size(size).ok_or_else(|| format!("{}: velikost pole {size}", m.name))?)
        } else {
            match m.ptr {
                0 => Shape::Value,
                1 if m.ty == "char" && first_len == "null-terminated" => Shape::Str,
                1 if !first_len.is_empty() && first_len != "null-terminated" => Shape::Dyn(first_len),
                1 if base != Base::Void => Shape::Ptr,
                2 if m.ty == "char" && m.len.get(1).map(String::as_str) == Some("null-terminated") => {
                    Shape::StrArray(first_len)
                }
                _ => return Err(format!("{}: ukazatel na {} neni podporovany", m.name, m.ty)),
            }
        };
        if base == Base::Void && !matches!(shape, Shape::Dyn(_)) {
            return Err(format!("{}: void bez delky", m.name));
        }
        Ok(Field { m: m.clone(), base, shape })
    }

    /// Cleny struktury bez `sType` a `pNext` (ty se koduji zvlast).
    pub fn struct_fields(&self, name: &str) -> Result<Vec<Field>, String> {
        let t = self.ty(name).ok_or_else(|| format!("neznamy typ {name}"))?;
        if let Some(e) = &t.error {
            return Err(e.clone());
        }
        t.members
            .iter()
            .filter(|m| m.name != "sType" && m.name != "pNext")
            .map(|m| self.field(m).map_err(|e| format!("{name}.{e}")))
            .collect()
    }

    /// Hodnota `sType` struktury (pokud ma `sType`).
    pub fn s_type(&self, name: &str) -> Option<String> {
        self.ty(name)?.members.iter().find(|m| m.name == "sType").and_then(|m| m.values.clone())
    }
}

/// Co se ma vygenerovat.
#[derive(Debug, Default)]
pub struct Plan {
    pub commands: Vec<String>,
    /// Struktury pro vstup (kodovani).
    pub input: BTreeSet<String>,
    /// Struktury pro vystup (castecne kodovani a cteni).
    pub output: BTreeSet<String>,
    pub handles: BTreeSet<String>,
    /// Skalarni aliasy (enum, bitmask, basetype): jmeno -> Prim.
    pub scalars: BTreeMap<String, Prim>,
    /// Retezce pNext: rodic -> struktury, ktere smi byt v jeho retezci.
    pub chain: BTreeMap<String, Vec<String>>,
    /// Struktury v nejakem retezci pNext.
    pub chained: BTreeSet<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    In,
    Out,
}

impl Plan {
    /// Uzaver typu od prikazu `commands` a struktur `extra` (clenu retezcu pNext).
    pub fn build(r: &Registry, commands: &[String], extra: &[String]) -> Result<Plan, String> {
        let mut p = Plan::default();
        for name in commands {
            let c = r.command(name).ok_or_else(|| format!("neznamy prikaz {name}"))?;
            r.command_type(name).ok_or_else(|| format!("{name}: chybi VK_COMMAND_TYPE_{name}_EXT"))?;
            p.commands.push(name.clone());
            let base = r.base(&c.ret)?;
            p.walk_base(r, &base, Dir::Out)?;
            for prm in &c.params {
                if prm.ty == "VkAllocationCallbacks" {
                    continue;
                }
                let f = r.field(prm).map_err(|e| format!("{name}: {e}"))?;
                let dir = if is_out_param(prm) { Dir::Out } else { Dir::In };
                p.walk_base(r, &f.base, dir)?;
            }
        }
        for s in extra {
            let canon = r.canonical(s).to_string();
            let t = r.ty(&canon).ok_or_else(|| format!("neznama struktura {s}"))?;
            p.walk_base(r, &Base::Struct(canon.clone()), Dir::In)?;
            for parent in &t.extends {
                p.chain.entry(r.canonical(parent).to_string()).or_default().push(canon.clone());
            }
            p.chained.insert(canon);
        }
        // Retezec jen u rodicu, ktere se opravdu koduji.
        p.chain.retain(|k, _| p.input.contains(k));
        Ok(p)
    }

    fn walk_base(&mut self, r: &Registry, b: &Base, dir: Dir) -> Result<(), String> {
        match b {
            Base::Prim { prim, rust, .. } => {
                if rust != prim.rust() {
                    self.scalars.insert(rust.clone(), *prim);
                    // FlagBits k bitmasce (kvuli hodnotam).
                    if let Some(t) = r.ty(rust)
                        && let Category::Bitmask { bits: Some(bits), .. } = &t.category
                        && r.ty(bits).is_some()
                    {
                        let bb = r.base(bits)?;
                        self.walk_base(r, &bb, dir)?;
                    }
                }
            }
            Base::Handle(h) => {
                self.handles.insert(h.clone());
            }
            Base::Struct(s) => {
                let set = if dir == Dir::In { &mut self.input } else { &mut self.output };
                if !set.insert(s.clone()) {
                    return Ok(());
                }
                if r.ty(s).is_some_and(|t| t.category != Category::Struct) {
                    return Err(format!("{s} neni struktura"));
                }
                for f in r.struct_fields(s)? {
                    if dir == Dir::Out && !matches!(f.shape, Shape::Value | Shape::Static(_)) {
                        return Err(format!("vystupni struktura {s}.{} ma ukazatel (neni podporovano)", f.m.name));
                    }
                    self.walk_base(r, &f.base, dir)?;
                }
            }
            Base::Void => {}
        }
        Ok(())
    }

    /// Potrebuje struktura parametr zivotnosti `'a` (ma odkazy)?
    pub fn needs_lifetime(&self, r: &Registry, name: &str) -> bool {
        if self.chain.contains_key(name) {
            return true;
        }
        let Ok(fields) = r.struct_fields(name) else {
            return false;
        };
        fields.iter().any(|f| match (&f.shape, &f.base) {
            (Shape::Value | Shape::Static(_), Base::Struct(s)) => self.needs_lifetime(r, s),
            (Shape::Value | Shape::Static(_), _) => false,
            _ => true,
        })
    }

    /// Zavisi castecne kodovani struktury na datech klienta (id handlu)?
    /// Kdyz ne, koduje se castecne kodovani vychozi hodnoty (sType, NULL
    /// pNext, velikosti pevnych poli) a parametr v Rustu neni potreba.
    pub fn partial_has_data(&self, r: &Registry, name: &str) -> bool {
        let Ok(fields) = r.struct_fields(name) else {
            return true;
        };
        fields.iter().any(|f| match &f.base {
            Base::Handle(_) => true,
            Base::Struct(s) => self.partial_has_data(r, s),
            _ => false,
        })
    }
}

/// Nekonstantni ukazatel = vystup (u parametru prikazu).
pub fn is_out_param(m: &Member) -> bool {
    m.ptr > 0 && !m.konst
}

/// Identifikatory ve vyrazu delky (`(rasterizationSamples + 31) / 32`,
/// `pAllocateInfo->descriptorSetCount`).
pub fn len_tokens(expr: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let b: Vec<char> = expr.chars().collect();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_alphanumeric() || c == '_' {
            cur.push(c);
        } else if c == '-' && b.get(i + 1) == Some(&'>') && !cur.is_empty() {
            cur.push_str("->");
            i += 1;
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            if !c.is_whitespace() {
                out.push(c.to_string());
            }
        }
        i += 1;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}
