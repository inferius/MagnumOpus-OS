//! Registr Vulkanu: typy, hodnoty enumu, konstanty a prikazy z `vk.xml`
//! a XML Venusu (`VK_EXT_command_serialization.xml` s cisly prikazu,
//! `VK_MESA_venus_protocol.xml` s prikazy transportu).
//!
//! Bere jen API `vulkan` (prvky s `api="vulkansc"` vynecha) a hodnoty
//! z rozsireni, ktera nejsou `supported="disabled"`.

use std::collections::{BTreeMap, HashMap};

use super::xml::{Element, Node};

/// Clen struktury nebo parametr prikazu.
#[derive(Debug, Clone)]
pub struct Member {
    pub name: String,
    /// Zakladni typ (`uint32_t`, `VkFormat`, `VkApplicationInfo`, ...).
    pub ty: String,
    /// Pocet `*`.
    pub ptr: u8,
    /// `const` pred typem (data, na ktera ukazatel miri, jsou jen ke cteni).
    pub konst: bool,
    /// Velikost pevneho pole (cislo nebo jmeno konstanty).
    pub array: Option<String>,
    /// Polozky atributu `len` (nebo `altlen`, kdyz `len` je latexmath).
    pub len: Vec<String>,
    /// Ukazatel smi byt NULL (prvni polozka `optional`).
    pub optional: bool,
    /// `values` u `sType` (hodnota VkStructureType).
    pub values: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Category {
    Struct,
    Union,
    /// `dispatchable` = VK_DEFINE_HANDLE.
    Handle,
    Enum,
    /// Typedef na `VkFlags` / `VkFlags64`; `bits` = jmeno FlagBits.
    Bitmask { flags64: bool, bits: Option<String> },
    /// Typedef na jiny typ (`VkBool32` -> `uint32_t`); `None` = neprusvitny.
    Basetype(Option<String>),
    Funcpointer,
    Other,
}

#[derive(Debug, Clone)]
pub struct TypeDef {
    pub category: Category,
    pub members: Vec<Member>,
    /// Clen, ktery generator neumi (napr. vicerozmerne pole): chyba az
    /// pri pouziti typu.
    pub error: Option<String>,
    /// `structextends` (rodice, do jejichz retezce pNext typ patri).
    pub extends: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Command {
    /// Navratovy typ (`void`, `VkResult`, ...).
    pub ret: String,
    pub params: Vec<Member>,
}

#[derive(Debug, Default)]
pub struct Registry {
    pub types: HashMap<String, TypeDef>,
    /// Alias typu -> cil.
    pub type_alias: HashMap<String, String>,
    /// Hodnoty enumu: jmeno typu -> (jmeno hodnoty -> hodnota), poradi podle hodnoty.
    pub enums: HashMap<String, BTreeMap<String, i64>>,
    /// 64bitove enumy (FlagBits2).
    pub enums64: Vec<String>,
    /// Konstanty API: jmeno -> (C typ, hodnota v Rustu).
    pub constants: BTreeMap<String, (String, String)>,
    pub commands: HashMap<String, Command>,
    pub command_alias: HashMap<String, String>,
}

fn api_ok(e: &Element) -> bool {
    e.attr("api").is_none_or(|a| a.split(',').any(|x| x == "vulkan"))
}

/// Rozebere `<member>` nebo `<param>`.
pub fn parse_member(e: &Element) -> Result<Member, String> {
    let name = e.child_text("name").ok_or("clen bez jmena")?;
    let ty = e.child_text("type").ok_or_else(|| format!("clen {name} bez typu"))?;
    // Text pred typem, mezi typem a jmenem a za jmenem (podle poradi uzlu).
    let (mut pre, mut post, mut after) = (String::new(), String::new(), String::new());
    let mut stage = 0;
    for c in &e.children {
        match c {
            Node::Elem(x) if x.name == "type" && stage == 0 => stage = 1,
            Node::Elem(x) if x.name == "name" => stage = 2,
            Node::Elem(x) if x.name == "comment" => {}
            Node::Elem(x) => {
                if stage == 2 {
                    after.push_str(&x.text_without(""));
                }
            }
            Node::Text(t) => match stage {
                0 => pre.push_str(t),
                1 => post.push_str(t),
                _ => after.push_str(t),
            },
        }
    }
    let ptr = post.matches('*').count() as u8;
    let konst = pre.contains("const");
    let after = after.trim();
    let array = if let Some(rest) = after.strip_prefix('[') {
        let (size, tail) = rest.split_once(']').ok_or_else(|| format!("clen {name}: pole bez ]"))?;
        if tail.trim_start().starts_with('[') {
            return Err(format!("clen {name}: vicerozmerne pole neni podporovane"));
        }
        Some(size.trim().to_string())
    } else {
        None
    };
    let len_attr = e.attr("len").unwrap_or("");
    let len_attr = if len_attr.starts_with("latexmath") { e.attr("altlen").unwrap_or("") } else { len_attr };
    let len = if len_attr.is_empty() { Vec::new() } else { len_attr.split(',').map(|s| s.trim().to_string()).collect() };
    let optional = e.attr("optional").and_then(|o| o.split(',').next()) == Some("true");
    Ok(Member {
        name,
        ty,
        ptr,
        konst,
        array,
        len,
        optional,
        values: e.attr("values").map(str::to_string),
    })
}

/// Hodnota konstanty API jako vyraz v Rustu (`(~0U)` -> `u32::MAX`).
fn constant_value(ty: &str, v: &str) -> Option<String> {
    let v = v.trim();
    Some(match (ty, v) {
        ("uint32_t", "(~0U)") => "u32::MAX".into(),
        ("uint32_t", "(~1U)") => "u32::MAX - 1".into(),
        ("uint32_t", "(~2U)") => "u32::MAX - 2".into(),
        ("uint64_t", "(~0ULL)") => "u64::MAX".into(),
        ("float", f) => {
            let f = f.trim_end_matches(['F', 'f']);
            f.parse::<f32>().ok()?;
            format!("{f}_f32")
        }
        (_, n) => {
            n.parse::<i64>().ok()?;
            n.to_string()
        }
    })
}

impl Registry {
    /// Nacte vsechny dokumenty (v poradi: vk.xml, pak XML Venusu).
    pub fn load(docs: &[Element]) -> Result<Self, String> {
        let mut r = Registry::default();
        // Hodnoty s aliasem se dopocitaji az nakonec.
        let mut aliases: Vec<(String, String, String)> = Vec::new();
        for d in docs {
            for types in d.elems("types") {
                for t in types.elems("type") {
                    if api_ok(t) {
                        r.add_type(t)?;
                    }
                }
            }
            for en in d.elems("enums") {
                r.add_enums(en, &mut aliases)?;
            }
            for cmds in d.elems("commands") {
                for c in cmds.elems("command") {
                    if api_ok(c) {
                        r.add_command(c)?;
                    }
                }
            }
            for feat in d.elems("feature") {
                if !api_ok(feat) {
                    continue;
                }
                for req in feat.elems("require") {
                    r.add_required(req, None, &mut aliases)?;
                }
            }
            for exts in d.elems("extensions") {
                for ext in exts.elems("extension") {
                    if ext.attr("supported") == Some("disabled") {
                        continue;
                    }
                    let number = ext.attr("number").and_then(|n| n.parse::<i64>().ok());
                    for req in ext.elems("require") {
                        if api_ok(req) {
                            r.add_required(req, number, &mut aliases)?;
                        }
                    }
                }
            }
        }
        // Aliasy hodnot (opakovane, alias muze mirit na alias).
        for _ in 0..4 {
            for (en, name, target) in &aliases {
                let v = r.enums.get(en).and_then(|m| m.get(target)).copied();
                if let Some(v) = v {
                    r.enums.entry(en.clone()).or_default().entry(name.clone()).or_insert(v);
                }
            }
        }
        Ok(r)
    }

    fn add_type(&mut self, t: &Element) -> Result<(), String> {
        let name = match t.attr("name") {
            Some(n) => n.to_string(),
            None => match t.child_text("name") {
                Some(n) => n,
                None => return Ok(()),
            },
        };
        if let Some(a) = t.attr("alias") {
            self.type_alias.insert(name, a.to_string());
            return Ok(());
        }
        let cat = t.attr("category").unwrap_or("");
        let category = match cat {
            "struct" => Category::Struct,
            "union" => Category::Union,
            "handle" => Category::Handle,
            "enum" => Category::Enum,
            "bitmask" => Category::Bitmask {
                flags64: t.child_text("type").as_deref() == Some("VkFlags64"),
                bits: t.attr("requires").or(t.attr("bitvalues")).map(str::to_string),
            },
            "basetype" => Category::Basetype(t.child_text("type")),
            "funcpointer" => Category::Funcpointer,
            _ => Category::Other,
        };
        let mut members = Vec::new();
        let mut error = None;
        if matches!(category, Category::Struct | Category::Union) {
            for m in t.elems("member") {
                if api_ok(m) {
                    match parse_member(m) {
                        Ok(m) => members.push(m),
                        Err(e) => error = Some(format!("{name}: {e}")),
                    }
                }
            }
        }
        let def = TypeDef {
            category,
            members,
            error,
            extends: t
                .attr("structextends")
                .map(|s| s.split(',').map(str::to_string).collect())
                .unwrap_or_default(),
        };
        // Prvni definice vyhrava (vk.xml je prvni).
        self.types.entry(name).or_insert(def);
        Ok(())
    }

    fn add_enums(&mut self, en: &Element, aliases: &mut Vec<(String, String, String)>) -> Result<(), String> {
        let Some(name) = en.attr("name") else {
            return Ok(());
        };
        if name == "API Constants" {
            for e in en.elems("enum") {
                let (Some(n), Some(ty), Some(v)) = (e.attr("name"), e.attr("type"), e.attr("value")) else {
                    continue;
                };
                if let Some(v) = constant_value(ty, v) {
                    self.constants.insert(n.to_string(), (ty.to_string(), v));
                }
            }
            return Ok(());
        }
        if en.attr("bitwidth") == Some("64") {
            self.enums64.push(name.to_string());
        }
        let map = self.enums.entry(name.to_string()).or_default();
        for e in en.elems("enum") {
            if !api_ok(e) {
                continue;
            }
            let Some(n) = e.attr("name") else { continue };
            if let Some(a) = e.attr("alias") {
                aliases.push((name.to_string(), n.to_string(), a.to_string()));
            } else if let Some(v) = e.attr("value") {
                let v = parse_int(v).ok_or_else(|| format!("{name}: hodnota {n} = {v}"))?;
                map.insert(n.to_string(), v);
            } else if let Some(b) = e.attr("bitpos") {
                let b: u32 = b.parse().map_err(|_| format!("{name}: bitpos {n}"))?;
                map.insert(n.to_string(), 1i64 << b);
            }
        }
        Ok(())
    }

    /// `<require>` z feature nebo rozsireni: rozsirujici hodnoty enumu.
    fn add_required(
        &mut self,
        req: &Element,
        ext_number: Option<i64>,
        aliases: &mut Vec<(String, String, String)>,
    ) -> Result<(), String> {
        for e in req.elems("enum") {
            if !api_ok(e) {
                continue;
            }
            let (Some(n), Some(ext)) = (e.attr("name"), e.attr("extends")) else {
                continue;
            };
            let v = if let Some(a) = e.attr("alias") {
                aliases.push((ext.to_string(), n.to_string(), a.to_string()));
                continue;
            } else if let Some(off) = e.attr("offset") {
                let off: i64 = off.parse().map_err(|_| format!("offset {n}"))?;
                let num = e
                    .attr("extnumber")
                    .and_then(|x| x.parse::<i64>().ok())
                    .or(ext_number)
                    .ok_or_else(|| format!("{n}: offset bez cisla rozsireni"))?;
                let v = 1_000_000_000 + (num - 1) * 1000 + off;
                if e.attr("dir") == Some("-") { -v } else { v }
            } else if let Some(v) = e.attr("value") {
                parse_int(v).ok_or_else(|| format!("{n} = {v}"))?
            } else if let Some(b) = e.attr("bitpos") {
                1i64 << b.parse::<u32>().map_err(|_| format!("bitpos {n}"))?
            } else {
                continue;
            };
            self.enums.entry(ext.to_string()).or_default().entry(n.to_string()).or_insert(v);
        }
        Ok(())
    }

    fn add_command(&mut self, c: &Element) -> Result<(), String> {
        if let (Some(n), Some(a)) = (c.attr("name"), c.attr("alias")) {
            self.command_alias.insert(n.to_string(), a.to_string());
            return Ok(());
        }
        let proto = c.elem("proto").ok_or("prikaz bez proto")?;
        let name = proto.child_text("name").ok_or("prikaz bez jmena")?;
        let ret = proto.child_text("type").ok_or_else(|| format!("{name}: navratovy typ"))?;
        let mut params = Vec::new();
        for p in c.elems("param") {
            if api_ok(p) {
                match parse_member(p) {
                    Ok(m) => params.push(m),
                    // Prikaz, ktery generator neumi, se nepouzije.
                    Err(_) => return Ok(()),
                }
            }
        }
        self.commands.entry(name).or_insert(Command { ret, params });
        Ok(())
    }

    /// Typ podle jmena (pres aliasy).
    pub fn ty(&self, name: &str) -> Option<&TypeDef> {
        let mut n = name;
        for _ in 0..4 {
            if let Some(t) = self.types.get(n) {
                return Some(t);
            }
            n = self.type_alias.get(n)?;
        }
        None
    }

    /// Kanonicke jmeno typu (alias -> cil).
    pub fn canonical<'a>(&'a self, name: &'a str) -> &'a str {
        let mut n = name;
        for _ in 0..4 {
            match self.type_alias.get(n) {
                Some(a) => n = a,
                None => break,
            }
        }
        n
    }

    pub fn command(&self, name: &str) -> Option<&Command> {
        let n = self.command_alias.get(name).map_or(name, |s| s.as_str());
        self.commands.get(n)
    }

    /// Cislo prikazu (`VK_COMMAND_TYPE_<jmeno>_EXT`).
    pub fn command_type(&self, name: &str) -> Option<i64> {
        self.enums.get("VkCommandTypeEXT")?.get(&format!("VK_COMMAND_TYPE_{name}_EXT")).copied()
    }

    /// Hodnota konstanty pro velikost pole.
    pub fn array_size(&self, size: &str) -> Option<usize> {
        if let Ok(n) = size.parse::<usize>() {
            return Some(n);
        }
        self.constants.get(size)?.1.parse().ok()
    }
}

fn parse_int(v: &str) -> Option<i64> {
    let v = v.trim();
    if let Some(h) = v.strip_prefix("0x") {
        return i64::from_str_radix(h, 16).ok();
    }
    v.parse().ok()
}
