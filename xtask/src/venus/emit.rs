//! Generovani Rustu: typy a konstanty, struktury (kodovani, castecne
//! kodovani, cteni) a retezce pNext. Prikazy jsou v `emit_cmd.rs`.

use std::collections::BTreeSet;
use std::fmt::Write;

use super::model::{Base, Field, Plan, Prim, Shape, len_tokens, snake};
use super::registry::Registry;

/// Typ prvku v Rustu (struktura s `<'a>`, kdyz ma odkazy).
pub fn elem_ty(r: &Registry, p: &Plan, b: &Base, lt: &str) -> String {
    match b {
        Base::Struct(s) if p.needs_lifetime(r, s) => format!("{s}<{lt}>"),
        _ => b.rust().to_string(),
    }
}

/// Typ clenu struktury.
fn field_ty(r: &Registry, p: &Plan, f: &Field) -> String {
    let t = elem_ty(r, p, &f.base, "'a");
    match &f.shape {
        Shape::Value => t,
        Shape::Static(n) => format!("[{t}; {n}]"),
        Shape::Ptr => format!("Option<&'a {t}>"),
        Shape::Dyn(_) => format!("&'a [{t}]"),
        Shape::Str => "Option<&'a str>".into(),
        Shape::StrArray(_) => "&'a [&'a str]".into(),
    }
}

fn zero(b: &Base) -> String {
    match b {
        Base::Prim { prim: Prim::F32 | Prim::F64, .. } => "0.0".into(),
        Base::Prim { .. } | Base::Void => "0".into(),
        Base::Handle(h) => format!("{h}(0)"),
        Base::Struct(s) | Base::Union(s) => format!("{s}::default()"),
    }
}

fn field_default(f: &Field) -> String {
    match &f.shape {
        Shape::Value => zero(&f.base),
        Shape::Static(n) => format!("[{}; {n}]", zero(&f.base)),
        Shape::Ptr | Shape::Str => "None".into(),
        Shape::Dyn(_) | Shape::StrArray(_) => "&[]".into(),
    }
}

/// Kodovani jedne hodnoty; `x` je vyraz typu `&T`.
pub fn enc_ref(b: &Base, x: &str) -> String {
    match b {
        Base::Prim { prim, .. } => format!("e.{}(*{x});", prim.rust()),
        Base::Handle(_) => format!("e.u64({x}.0);"),
        Base::Struct(_) | Base::Union(_) => format!("{x}.encode(e);"),
        Base::Void => format!("e.u8(*{x});"),
    }
}

/// Kodovani jedne hodnoty; `x` je vyraz typu `T` (Copy).
pub fn enc_val(b: &Base, x: &str) -> String {
    match b {
        Base::Prim { prim, .. } => format!("e.{}({x});", prim.rust()),
        Base::Handle(_) => format!("e.u64({x}.0);"),
        Base::Struct(_) | Base::Union(_) => format!("{x}.encode(e);"),
        Base::Void => format!("e.u8({x});"),
    }
}

/// Castecne kodovani jedne hodnoty (handle = id, struktura = sType/pNext).
pub fn enc_partial_ref(b: &Base, x: &str) -> String {
    match b {
        Base::Handle(_) => format!("e.u64({x}.0);"),
        Base::Struct(_) => format!("{x}.encode_partial(e);"),
        _ => String::new(),
    }
}

/// Kodovani prvku pole; `s` je vyraz typu `&[T]`.
pub fn enc_slice(b: &Base, s: &str) -> String {
    match b {
        Base::Prim { prim, .. } => format!("e.{}_array({s});", prim.rust()),
        Base::Void => format!("e.u8_array({s});"),
        _ => format!("for x in {s} {{ {} }}", enc_ref(b, "x")),
    }
}

/// Cteni jedne hodnoty (vyraz typu `T`).
pub fn dec(b: &Base) -> String {
    match b {
        Base::Prim { prim, .. } => format!("d.{}()?", prim.rust()),
        Base::Handle(h) => format!("{h}(d.u64()?)"),
        Base::Struct(s) => format!("{s}::decode(d)?"),
        // Unie ve vystupu odmitne uz `Plan::build`.
        Base::Union(u) => format!("{u}::default()"),
        Base::Void => "d.u8()?".into(),
    }
}

/// Nejmensi velikost prvku pole v proudu (ochrana `array_size`).
pub fn min_elem(b: &Base) -> usize {
    match b {
        Base::Prim { prim, .. } => prim.size(),
        Base::Handle(_) => 8,
        Base::Struct(_) | Base::Union(_) => 4,
        Base::Void => 1,
    }
}

/// Vyraz delky v kontextu struktury (`self.<clen>`).
fn struct_len(fields: &[Field], expr: &str, owner: &str) -> Result<String, String> {
    let mut out = String::new();
    for t in len_tokens(expr) {
        if t.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            let f = fields
                .iter()
                .find(|f| f.m.name == t)
                .ok_or_else(|| format!("{owner}: delka {expr}: neznamy clen {t}"))?;
            if f.shape != Shape::Value {
                return Err(format!("{owner}: delka {expr}: {t} neni hodnota"));
            }
            write!(out, "(self.{} as usize)", snake(&t)).unwrap();
        } else {
            out.push_str(&t);
        }
    }
    Ok(strip_parens(out))
}

/// `(x as usize)` -> `x as usize` (jinak varovani unused_parens).
pub fn strip_parens(s: String) -> String {
    let Some(inner) = s.strip_prefix('(').and_then(|x| x.strip_suffix(')')) else {
        return s;
    };
    // Vnejsi zavorky musi patrit k sobe: hloubka uvnitr nesmi klesnout pod 0.
    let mut depth = 0i32;
    for c in inner.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if depth < 0 {
            return s;
        }
    }
    inner.to_string()
}

/// Je clen delkou jineho clenu (pak se i ve vystupu koduje cely)?
fn is_len_target(fields: &[Field], name: &str) -> bool {
    fields.iter().any(|f| match &f.shape {
        Shape::Dyn(l) | Shape::StrArray(l) => len_tokens(l).iter().any(|t| t == name),
        _ => false,
    })
}

fn enc_field(fields: &[Field], f: &Field, owner: &str) -> Result<String, String> {
    let n = snake(&f.m.name);
    Ok(match &f.shape {
        Shape::Value => enc_val(&f.base, &format!("self.{n}")),
        Shape::Static(c) => {
            let s = match f.base {
                Base::Prim { .. } => enc_slice(&f.base, &format!("&self.{n}")),
                _ => enc_slice(&f.base, &format!("self.{n}.iter()")),
            };
            format!("e.array_size({c});\n        {s}")
        }
        Shape::Ptr => format!(
            "match self.{n} {{\n            Some(x) => {{\n                e.pointer(true);\n                {}\n            }}\n            None => e.pointer(false),\n        }}",
            enc_ref(&f.base, "x")
        ),
        Shape::Dyn(l) => {
            let len = struct_len(fields, l, owner)?;
            format!("{{\n            let s = e.dyn_array(self.{n}, {len});\n            {}\n        }}", enc_slice(&f.base, "s"))
        }
        Shape::Str => format!("e.opt_string(self.{n});"),
        Shape::StrArray(l) => {
            let len = struct_len(fields, l, owner)?;
            format!(
                "{{\n            let s = e.dyn_array(self.{n}, {len});\n            for x in s {{\n                e.string(x);\n            }}\n        }}"
            )
        }
    })
}

fn enc_partial_field(fields: &[Field], f: &Field, owner: &str) -> Result<String, String> {
    if is_len_target(fields, &f.m.name) {
        return enc_field(fields, f, owner);
    }
    let n = snake(&f.m.name);
    if !matches!(f.base, Base::Handle(_) | Base::Struct(_)) {
        return Ok(String::new());
    }
    Ok(match &f.shape {
        Shape::Value => enc_partial_ref(&f.base, &format!("self.{n}")),
        Shape::Static(c) => {
            format!("e.array_size({c});\n        for x in self.{n}.iter() {{ {} }}", enc_partial_ref(&f.base, "x"))
        }
        _ => return Err(format!("{owner}.{}: vystup s ukazatelem", f.m.name)),
    })
}

fn dec_field(f: &Field) -> String {
    let n = snake(&f.m.name);
    match &f.shape {
        Shape::Static(c) => {
            let body = match &f.base {
                Base::Prim { char: true, .. } => format!("d.char_array(&mut v.{n})?;"),
                Base::Prim { prim, .. } => format!("d.{}_array(&mut v.{n})?;", prim.rust()),
                b => format!("for x in v.{n}.iter_mut() {{ *x = {}; }}", dec(b)),
            };
            format!("d.fixed_array({c})?;\n        {body}")
        }
        _ => format!("v.{n} = {};", dec(&f.base)),
    }
}

/// Jedna struktura: definice, Default a metody podle smeru.
pub fn emit_struct(r: &Registry, p: &Plan, name: &str) -> Result<String, String> {
    let fields = r.struct_fields(name)?;
    let lt = p.needs_lifetime(r, name);
    let generics = if lt { "<'a>" } else { "" };
    let anon = if lt { "<'_>" } else { "" };
    let input = p.input.contains(name);
    let output = p.output.contains(name);
    let chain = p.chain.get(name);
    let s_type = r.s_type(name);
    let mut o = String::new();
    let dir = match (input, output) {
        (true, true) => "vstup i vystup",
        (true, false) => "vstup",
        _ => "vystup",
    };
    writeln!(o, "/// `{name}` ({dir}).").unwrap();
    writeln!(o, "#[derive(Debug, Clone, Copy)]").unwrap();
    writeln!(o, "pub struct {name}{generics} {{").unwrap();
    if chain.is_some() {
        writeln!(o, "    /// Retezec pNext (jen struktury, ktere `{name}` povoluje).").unwrap();
        writeln!(o, "    pub p_next: &'a [Next<'a>],").unwrap();
    }
    for f in &fields {
        writeln!(o, "    pub {}: {},", snake(&f.m.name), field_ty(r, p, f)).unwrap();
    }
    writeln!(o, "}}\n").unwrap();
    writeln!(o, "impl Default for {name}{anon} {{").unwrap();
    writeln!(o, "    fn default() -> Self {{").unwrap();
    writeln!(o, "        Self {{").unwrap();
    if chain.is_some() {
        writeln!(o, "            p_next: &[],").unwrap();
    }
    for f in &fields {
        writeln!(o, "            {}: {},", snake(&f.m.name), field_default(f)).unwrap();
    }
    writeln!(o, "        }}\n    }}\n}}\n").unwrap();
    writeln!(o, "impl{generics} {name}{generics} {{").unwrap();
    if let Some(st) = &s_type {
        writeln!(o, "    pub const S_TYPE: VkStructureType = {st};\n").unwrap();
    }
    // Vystupni struktury maji plne kodovani jen v testech (odpoved jako
    // od rendereru, zpatky pres `decode`).
    let test_only = if input { "" } else { "    #[cfg(test)]
" };
    {
        o.push_str(test_only);
        writeln!(o, "    pub fn encode(&self, e: &mut Encoder) {{").unwrap();
        if s_type.is_some() {
            writeln!(o, "        e.i32(Self::S_TYPE);").unwrap();
            match chain {
                Some(list) => {
                    let allowed: Vec<String> = list.iter().map(|c| format!("{c}::S_TYPE")).collect();
                    writeln!(o, "        encode_chain(e, self.p_next, &[{}]);", allowed.join(", ")).unwrap();
                }
                None => writeln!(o, "        e.pointer(false);").unwrap(),
            }
        }
        writeln!(o, "        self.encode_self(e);\n    }}\n").unwrap();
        writeln!(o, "    /// Cleny bez sType a pNext.").unwrap();
        o.push_str(test_only);
        writeln!(o, "    pub fn encode_self(&self, e: &mut Encoder) {{").unwrap();
        for f in &fields {
            writeln!(o, "        {}", enc_field(&fields, f, name)?).unwrap();
        }
        writeln!(o, "    }}\n").unwrap();
    }
    if output {
        writeln!(o, "    /// Castecne kodovani vystupu (sType, pNext, id handlu).").unwrap();
        writeln!(o, "    pub fn encode_partial(&self, e: &mut Encoder) {{").unwrap();
        let mut body = String::new();
        if s_type.is_some() {
            body.push_str("        e.i32(Self::S_TYPE);\n        e.pointer(false);\n");
        }
        for f in &fields {
            let s = enc_partial_field(&fields, f, name)?;
            if !s.is_empty() {
                writeln!(body, "        {s}").unwrap();
            }
        }
        if body.is_empty() {
            body.push_str("        let _ = e;\n");
        }
        o.push_str(&body);
        writeln!(o, "    }}\n").unwrap();
        writeln!(o, "    pub fn decode(d: &mut Decoder) -> Result<Self> {{").unwrap();
        writeln!(o, "        let mut v = Self::default();").unwrap();
        if s_type.is_some() {
            writeln!(o, "        if d.i32()? != Self::S_TYPE {{\n            return Err(CsError::Unexpected);\n        }}").unwrap();
            writeln!(o, "        d.no_chain()?;").unwrap();
        }
        for f in &fields {
            writeln!(o, "        {}", dec_field(f)).unwrap();
        }
        writeln!(o, "        Ok(v)\n    }}").unwrap();
    }
    writeln!(o, "}}").unwrap();
    Ok(o)
}

/// Velbloudi jmeno varianty unie (`float32` -> `Float32`).
fn variant(name: &str) -> String {
    let mut c = name.chars();
    match c.next() {
        Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
        None => String::new(),
    }
}

/// Unie: vycet s variantou pro kazdy clen; kodovani = index clenu (u32)
/// a clen (renderer prijme kterykoli, `vn_decode_*` prepina podle tagu).
pub fn emit_union(r: &Registry, name: &str) -> Result<String, String> {
    let fields = r.struct_fields(name)?;
    let first = fields.first().ok_or_else(|| format!("prazdna unie {name}"))?;
    let mut o = String::new();
    writeln!(o, "/// Unie `{name}` (vstup): varianta = clen.").unwrap();
    writeln!(o, "#[derive(Debug, Clone, Copy)]").unwrap();
    writeln!(o, "pub enum {name} {{").unwrap();
    for f in &fields {
        let t = match &f.shape {
            Shape::Static(n) => format!("[{}; {n}]", f.base.rust()),
            _ => f.base.rust().to_string(),
        };
        writeln!(o, "    {}({t}),", variant(&f.m.name)).unwrap();
    }
    writeln!(o, "}}
").unwrap();
    writeln!(o, "impl Default for {name} {{").unwrap();
    writeln!(o, "    fn default() -> Self {{").unwrap();
    let z = match &first.shape {
        Shape::Static(n) => format!("[{}; {n}]", zero(&first.base)),
        _ => zero(&first.base),
    };
    writeln!(o, "        Self::{}({z})
    }}
}}
", variant(&first.m.name)).unwrap();
    writeln!(o, "impl {name} {{").unwrap();
    writeln!(o, "    pub fn encode(&self, e: &mut Encoder) {{").unwrap();
    writeln!(o, "        match self {{").unwrap();
    for (i, f) in fields.iter().enumerate() {
        let body = match &f.shape {
            Shape::Static(n) => {
                let arr = match f.base {
                    Base::Prim { .. } => enc_slice(&f.base, "&x[..]"),
                    _ => enc_slice(&f.base, "x.iter()"),
                };
                format!("e.array_size({n});
                {arr}")
            }
            _ => enc_ref(&f.base, "x"),
        };
        writeln!(
            o,
            "            Self::{}(x) => {{
                e.u32({i});
                {body}
            }}",
            variant(&f.m.name)
        )
        .unwrap();
    }
    writeln!(o, "        }}
    }}
}}").unwrap();
    Ok(o)
}

/// Vycet struktur retezcu pNext a jeho kodovani.
pub fn emit_chain(r: &Registry, p: &Plan) -> Option<String> {
    if p.chained.is_empty() {
        return None;
    }
    let mut o = String::new();
    writeln!(o, "/// Struktura v retezci pNext.").unwrap();
    writeln!(o, "#[derive(Debug, Clone, Copy)]").unwrap();
    writeln!(o, "pub enum Next<'a> {{").unwrap();
    for c in &p.chained {
        writeln!(o, "    {c}(&'a {}),", elem_ty(r, p, &Base::Struct(c.clone()), "'a")).unwrap();
    }
    writeln!(o, "}}\n\nimpl Next<'_> {{").unwrap();
    writeln!(o, "    pub fn s_type(&self) -> VkStructureType {{\n        match self {{").unwrap();
    for c in &p.chained {
        writeln!(o, "            Next::{c}(_) => {c}::S_TYPE,").unwrap();
    }
    writeln!(o, "        }}\n    }}\n").unwrap();
    writeln!(o, "    pub fn encode_self(&self, e: &mut Encoder) {{\n        match self {{").unwrap();
    for c in &p.chained {
        writeln!(o, "            Next::{c}(x) => x.encode_self(e),").unwrap();
    }
    writeln!(o, "        }}\n    }}\n}}\n").unwrap();
    o.push_str(
        "/// Retezec pNext: ukazatel, sType, zbytek retezce, cleny (jako\n\
         /// `vn_encode_*_pnext`). Struktura mimo `allowed` = chyba.\n\
         pub fn encode_chain(e: &mut Encoder, next: &[Next<'_>], allowed: &[VkStructureType]) {\n\
         \x20   match next.split_first() {\n\
         \x20       None => e.pointer(false),\n\
         \x20       Some((n, rest)) => {\n\
         \x20           if !allowed.contains(&n.s_type()) {\n\
         \x20               e.fail(CsError::BadChain);\n\
         \x20           }\n\
         \x20           e.pointer(true);\n\
         \x20           e.i32(n.s_type());\n\
         \x20           encode_chain(e, rest, allowed);\n\
         \x20           n.encode_self(e);\n\
         \x20       }\n\
         \x20   }\n\
         }\n",
    );
    Some(o)
}

fn fmt_value(prim: Prim, v: i64) -> String {
    match prim {
        Prim::I32 | Prim::I64 => v.to_string(),
        _ if v < 0 => v.to_string(),
        _ if v > 9 => format!("0x{v:x}"),
        _ => v.to_string(),
    }
}

/// Typy (aliasy skalaru, handly) a konstanty; vraci polozky po skupinach.
pub fn emit_types(r: &Registry, p: &Plan) -> Vec<String> {
    let mut items = Vec::new();
    let mut o = String::from("// Skalarni typy (enum = i32, bitmaska a FlagBits = u32/u64).\n");
    for (n, prim) in &p.scalars {
        writeln!(o, "pub type {n} = {};", prim.rust()).unwrap();
    }
    items.push(o);
    let mut o = String::from("// Handly: id objektu, ktere prideluje klient (u64).\n");
    for h in &p.handles {
        writeln!(o, "#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]").unwrap();
        writeln!(o, "#[repr(transparent)]").unwrap();
        writeln!(o, "pub struct {h}(pub u64);").unwrap();
    }
    items.push(o);
    let mut o = String::from("// Konstanty API.\n");
    for (n, (ty, v)) in &r.constants {
        let t = match ty.as_str() {
            "uint32_t" => "u32",
            "uint64_t" => "u64",
            "float" => "f32",
            _ => continue,
        };
        writeln!(o, "pub const {n}: {t} = {v};").unwrap();
    }
    items.push(o);
    // sType a typy prikazu jen pro vygenerovane struktury a prikazy.
    let s_types: BTreeSet<String> =
        p.input.iter().chain(p.output.iter()).filter_map(|s| r.s_type(s)).collect();
    let cmd_types: BTreeSet<String> = p.commands.iter().map(|c| format!("VK_COMMAND_TYPE_{c}_EXT")).collect();
    let mut seen = BTreeSet::new();
    for (n, prim) in &p.scalars {
        let Some(values) = r.enums.get(n) else { continue };
        let mut vals: Vec<(&String, &i64)> = values
            .iter()
            .filter(|(k, _)| match n.as_str() {
                "VkStructureType" => s_types.contains(*k),
                "VkCommandTypeEXT" => cmd_types.contains(*k),
                _ => true,
            })
            .collect();
        if vals.is_empty() {
            continue;
        }
        vals.sort_by_key(|(k, v)| (**v, (*k).clone()));
        let mut o = format!("// {n}\n");
        for (k, v) in vals {
            if seen.insert(k.clone()) {
                writeln!(o, "pub const {k}: {n} = {};", fmt_value(*prim, *v)).unwrap();
            }
        }
        items.push(o);
    }
    items
}
