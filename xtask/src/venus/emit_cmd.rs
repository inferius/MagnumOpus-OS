//! Generovani prikazu: funkce, ktera prikaz zakoduje do proudu, struktura
//! odpovedi a jeji cteni.
//!
//! Parametry (pravidla `vn_protocol.py`):
//! - vstup: hodnota, `&T` / `Option<&T>`, pole `&[T]`, retezec,
//! - vstup i vystup (pocet u vkEnumerate*): `Option<u32>`; v odpovedi
//!   prijde novy pocet,
//! - vystupni handle: id od klienta (renderer pod nim objekt vytvori),
//! - vystupni struktura: castecne kodovani; kdyz nezavisi na datech
//!   (zadne id handlu), parametr v Rustu chybi a koduje se castecne
//!   kodovani vychozi hodnoty (sType, NULL pNext, velikosti poli),
//! - vystupni pole skalaru nebo struktur bez dat: `bool` (chci pole),
//!   delka podle parametru delky.

use std::fmt::Write;

use super::emit::{dec, elem_ty, enc_partial_ref, enc_ref, enc_slice, enc_val, min_elem, strip_parens};
use super::model::{Base, Field, Plan, Shape, cmd_camel, cmd_snake, is_out_param, len_tokens, snake};
use super::registry::Registry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `VkAllocationCallbacks`: vzdy NULL, v Rustu chybi.
    Alloc,
    In,
    InOutCount,
    OutPartialPtr,
    OutEmptyPtr,
    OutPartialDyn,
    OutFlagDyn,
}

struct Param {
    f: Field,
    kind: Kind,
    name: String,
}

fn classify(r: &Registry, p: &Plan, all: &[Field], f: &Field) -> Result<Kind, String> {
    if f.m.ty == "VkAllocationCallbacks" {
        return Ok(Kind::Alloc);
    }
    if !is_out_param(&f.m) {
        return Ok(Kind::In);
    }
    let is_len = all.iter().any(|o| match &o.shape {
        Shape::Dyn(l) | Shape::StrArray(l) => len_tokens(l).iter().any(|t| t == &f.m.name),
        _ => false,
    });
    if is_len {
        if f.shape != Shape::Ptr || !matches!(f.base, Base::Prim { .. }) {
            return Err(format!("{}: pocet neni skalar", f.m.name));
        }
        return Ok(Kind::InOutCount);
    }
    let partial_data = match &f.base {
        Base::Handle(_) => true,
        Base::Struct(s) => p.partial_has_data(r, s),
        _ => false,
    };
    Ok(match (&f.shape, partial_data) {
        (Shape::Ptr, true) => Kind::OutPartialPtr,
        (Shape::Ptr, false) => Kind::OutEmptyPtr,
        (Shape::Dyn(_), true) => Kind::OutPartialDyn,
        (Shape::Dyn(_), false) => Kind::OutFlagDyn,
        _ => return Err(format!("{}: vystup tohoto tvaru neni podporovany", f.m.name)),
    })
}

/// Vyraz delky nad parametry prikazu.
fn cmd_len(params: &[Param], expr: &str, cmd: &str) -> Result<String, String> {
    let mut out = String::new();
    for t in len_tokens(expr) {
        if !t.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            out.push_str(&t);
            continue;
        }
        let (head, member) = match t.split_once("->") {
            Some((a, b)) => (a, Some(b)),
            None => (t.as_str(), None),
        };
        let prm = params
            .iter()
            .find(|x| x.f.m.name == head)
            .ok_or_else(|| format!("{cmd}: delka {expr}: neznamy parametr {head}"))?;
        let n = &prm.name;
        let s = match (prm.kind, member, &prm.f.shape) {
            (Kind::InOutCount, None, _) => format!("({n}.unwrap_or(0) as usize)"),
            (Kind::In, None, Shape::Value) => format!("({n} as usize)"),
            (Kind::In, Some(m), Shape::Ptr) if prm.f.m.optional => {
                format!("{n}.map_or(0, |x| x.{} as usize)", snake(m))
            }
            (Kind::In, Some(m), Shape::Ptr) => format!("({n}.{} as usize)", snake(m)),
            _ => return Err(format!("{cmd}: delka {expr}: nepodporovany tvar {head}")),
        };
        out.push_str(&s);
    }
    Ok(strip_parens(out))
}

fn arg_ty(r: &Registry, p: &Plan, prm: &Param) -> Option<String> {
    let t = elem_ty(r, p, &prm.f.base, "'_");
    let opt = prm.f.m.optional;
    Some(match prm.kind {
        Kind::Alloc | Kind::OutEmptyPtr => return None,
        Kind::In => match &prm.f.shape {
            Shape::Value => t,
            Shape::Ptr if opt => format!("Option<&{t}>"),
            Shape::Ptr => format!("&{t}"),
            Shape::Dyn(_) => format!("&[{t}]"),
            Shape::Str if opt => "Option<&str>".into(),
            Shape::Str => "&str".into(),
            Shape::StrArray(_) => "&[&str]".into(),
            Shape::Static(n) => format!("&[{t}; {n}]"),
        },
        Kind::InOutCount => format!("Option<{t}>"),
        Kind::OutPartialPtr => match (&prm.f.base, opt) {
            (Base::Handle(_), true) => format!("Option<{t}>"),
            (Base::Handle(_), false) => t,
            (_, true) => format!("Option<&{t}>"),
            (_, false) => format!("&{t}"),
        },
        Kind::OutPartialDyn => format!("Option<&[{t}]>"),
        Kind::OutFlagDyn => "bool".into(),
    })
}

fn encode_param(params: &[Param], prm: &Param, cmd: &str) -> Result<String, String> {
    let n = &prm.name;
    let b = &prm.f.base;
    let opt = prm.f.m.optional;
    let some = |inner: String| {
        format!(
            "match {n} {{\n        Some(x) => {{\n            e.pointer(true);\n            {inner}\n        }}\n        None => e.pointer(false),\n    }}"
        )
    };
    Ok(match prm.kind {
        Kind::Alloc => "e.pointer(false);".into(),
        Kind::In => match &prm.f.shape {
            Shape::Value => enc_val(b, n),
            Shape::Ptr if opt => some(enc_ref(b, "x")),
            Shape::Ptr => format!("e.pointer(true);\n    {}", enc_ref(b, n)),
            Shape::Dyn(l) => {
                let len = cmd_len(params, l, cmd)?;
                format!("{{\n        let s = e.dyn_array({n}, {len});\n        {}\n    }}", enc_slice(b, "s"))
            }
            Shape::Str if opt => format!("e.opt_string({n});"),
            Shape::Str => format!("e.string({n});"),
            Shape::StrArray(l) => {
                let len = cmd_len(params, l, cmd)?;
                format!("{{\n        let s = e.dyn_array({n}, {len});\n        for x in s {{\n            e.string(x);\n        }}\n    }}")
            }
            Shape::Static(c) => format!("e.array_size({c});\n    {}", enc_slice(b, &format!("{n}.iter()"))),
        },
        Kind::InOutCount => some(enc_val(b, "x")),
        Kind::OutPartialPtr => match (b, opt) {
            (Base::Handle(_), true) => some(enc_val(b, "x")),
            (_, true) => some(enc_partial_ref(b, "x")),
            _ => format!("e.pointer(true);\n    {}", enc_partial_ref(b, n)),
        },
        Kind::OutEmptyPtr => match b {
            Base::Struct(s) => format!("e.pointer(true);
    {s}::default().encode_partial(e);"),
            _ => "e.pointer(true);".into(),
        },
        Kind::OutPartialDyn => {
            let Shape::Dyn(l) = &prm.f.shape else { unreachable!() };
            let len = cmd_len(params, l, cmd)?;
            format!(
                "match {n} {{\n        Some(s) => {{\n            let s = e.dyn_array(s, {len});\n            for x in s {{ {} }}\n        }}\n        None => e.array_size(0),\n    }}",
                enc_partial_ref(b, "x")
            )
        }
        Kind::OutFlagDyn => {
            let Shape::Dyn(l) = &prm.f.shape else { unreachable!() };
            let len = cmd_len(params, l, cmd)?;
            match b {
                Base::Struct(s) => format!(
                    "{{
        let n = if {n} {{ {len} }} else {{ 0 }};
        e.array_size(n);
        for _ in 0..n {{
            {s}::default().encode_partial(e);
        }}
    }}"
                ),
                _ => format!("e.array_size(if {n} {{ {len} }} else {{ 0 }});"),
            }
        }
    })
}

/// Jeden prikaz: konstanta, kodovani, odpoved.
pub fn emit_command(r: &Registry, p: &Plan, name: &str) -> Result<String, String> {
    let c = r.command(name).ok_or_else(|| format!("neznamy prikaz {name}"))?;
    let fields: Vec<Field> = c
        .params
        .iter()
        .map(|m| {
            if m.ty == "VkAllocationCallbacks" {
                Ok(Field { m: m.clone(), base: Base::Void, shape: Shape::Ptr })
            } else {
                r.field(m).map_err(|e| format!("{name}: {e}"))
            }
        })
        .collect::<Result<_, _>>()?;
    let mut params = Vec::new();
    for f in &fields {
        let kind = classify(r, p, &fields, f).map_err(|e| format!("{name}: {e}"))?;
        params.push(Param { f: f.clone(), kind, name: snake(&f.m.name) });
    }
    let fname = cmd_snake(name);
    let camel = cmd_camel(name);
    let ty_const = format!("VK_COMMAND_TYPE_{name}_EXT");
    let mut o = String::new();
    // Kodovani.
    let mut args = vec!["e: &mut Encoder".to_string(), "flags: VkCommandFlagsEXT".to_string()];
    for prm in &params {
        if let Some(t) = arg_ty(r, p, prm) {
            args.push(format!("{}: {t}", prm.name));
        }
    }
    writeln!(o, "/// `{name}`: zakoduje prikaz (`flags` s `VK_COMMAND_GENERATE_REPLY_BIT_EXT` = chci odpoved).").unwrap();
    writeln!(o, "pub fn {fname}({}) {{", args.join(", ")).unwrap();
    writeln!(o, "    e.i32({ty_const});\n    e.u32(flags);").unwrap();
    for prm in &params {
        writeln!(o, "    {}", encode_param(&params, prm, name)?).unwrap();
    }
    writeln!(o, "}}\n").unwrap();
    // Odpoved.
    let ret = if c.ret == "void" { None } else { Some(r.base(&c.ret)?) };
    let outs: Vec<&Param> = params
        .iter()
        .filter(|x| !matches!(x.kind, Kind::Alloc | Kind::In))
        .collect();
    writeln!(o, "/// Odpoved na `{name}`.").unwrap();
    writeln!(o, "#[derive(Debug, Clone, Default)]").unwrap();
    writeln!(o, "pub struct {camel}Reply {{").unwrap();
    if let Some(b) = &ret {
        writeln!(o, "    pub ret: {},", b.rust()).unwrap();
    }
    for prm in &outs {
        let t = elem_ty(r, p, &prm.f.base, "'static");
        let t = match prm.f.shape {
            Shape::Dyn(_) => format!("Option<Vec<{t}>>"),
            _ => format!("Option<{t}>"),
        };
        writeln!(o, "    pub {}: {t},", prm.name).unwrap();
    }
    writeln!(o, "}}\n").unwrap();
    writeln!(o, "/// Precte odpoved na `{name}` (typ prikazu, navratova hodnota, vystupy).").unwrap();
    writeln!(o, "pub fn decode_{fname}_reply(d: &mut Decoder) -> Result<{camel}Reply> {{").unwrap();
    writeln!(o, "    if d.i32()? != {ty_const} {{\n        return Err(CsError::Unexpected);\n    }}").unwrap();
    let mutable = if ret.is_some() || !outs.is_empty() { "mut " } else { "" };
    writeln!(o, "    let {mutable}r = {camel}Reply::default();").unwrap();
    if let Some(b) = &ret {
        writeln!(o, "    r.ret = {};", dec(b)).unwrap();
    }
    for prm in &outs {
        let b = &prm.f.base;
        match prm.f.shape {
            Shape::Dyn(_) => {
                let read = match b {
                    Base::Prim { prim, .. } => {
                        format!("let mut v = alloc::vec![Default::default(); n];\n            d.{}_array(&mut v)?;", prim.rust())
                    }
                    Base::Void => "let mut v = alloc::vec![0u8; n];\n            d.u8_array(&mut v)?;".into(),
                    _ => format!(
                        "let mut v = Vec::with_capacity(n);\n            for _ in 0..n {{\n                v.push({});\n            }}",
                        dec(b)
                    ),
                };
                writeln!(
                    o,
                    "    r.{} = {{\n        let n = d.array_size({})?;\n        if n == 0 {{\n            None\n        }} else {{\n            {read}\n            Some(v)\n        }}\n    }};",
                    prm.name,
                    min_elem(b)
                )
                .unwrap();
            }
            _ => {
                writeln!(o, "    r.{} = if d.pointer()? {{ Some({}) }} else {{ None }};", prm.name, dec(b)).unwrap();
            }
        }
    }
    writeln!(o, "    Ok(r)\n}}").unwrap();
    Ok(o)
}
