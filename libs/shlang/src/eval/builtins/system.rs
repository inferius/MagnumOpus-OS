//! Volby systemu pres hostitele: vzhled (`theme`) a povoleni aplikaci
//! (`permissions`). Jmena prikazu a slova voleb jsou anglicky, hlaseni
//! cesky; do ulozeneho tvaru (`theme::Pref`, `accountproto::perm`)
//! prevadi hostitel nebo tenhle modul.

use alloc::string::String;

use super::super::call::Args;
use super::super::{Flow, R};
use crate::error::{ShError, code};
use crate::host::Host;
use crate::value::Value;

pub(super) fn call(host: &mut dyn Host, name: &str, a: Args) -> R<Value> {
    match name {
        "theme" => theme(host, &a),
        _ => permissions(host, &a),
    }
}

fn word(a: &Args, i: usize, cmd: &str) -> R<Option<String>> {
    match a.pos(i) {
        Some(Value::Str(s)) => Ok(Some(s.clone())),
        None => Ok(None),
        Some(other) => Err(a.err(code::TYPE, alloc::format!("{cmd}: čekám slovo, dostal jsem {}", other.type_name()))),
    }
}

/// Slovo prikazu -> slovo ulozeneho tvaru vzhledu (`theme::Pref::apply`).
fn on_off(v: &str) -> Option<&'static str> {
    match v {
        "on" => Some("ano"),
        "off" => Some("ne"),
        _ => None,
    }
}

const THEME_USAGE: &str = "theme: light, dark, auto, nebo contrast|reduce-motion on|off";

fn theme(host: &mut dyn Host, a: &Args) -> R<Value> {
    let change = match (word(a, 0, "theme")?.as_deref(), word(a, 1, "theme")?.as_deref()) {
        (None, _) => String::new(),
        (Some("light"), None) => String::from("schema svetly"),
        (Some("dark"), None) => String::from("schema tmavy"),
        (Some("auto"), None) => String::from("schema auto"),
        (Some(k @ ("contrast" | "reduce-motion")), Some(v)) => {
            let v = on_off(v).ok_or_else(|| a.err(code::ARGS, alloc::format!("theme {k}: on nebo off")))?;
            alloc::format!("{} {v}", if k == "contrast" { "kontrast" } else { "pohyb" })
        }
        _ => return Err(a.err(code::ARGS, THEME_USAGE)),
    };
    match host.appearance(a.flag("system"), &change) {
        Ok(words) => {
            let mut it = words.split_whitespace();
            let scheme = match it.next() {
                Some("svetly") => Value::str("light"),
                Some("tmavy") => Value::str("dark"),
                Some("auto") => Value::str("auto"),
                _ => Value::Nothing,
            };
            let flag = |w: Option<&str>| match w {
                Some("ano") => Value::Bool(true),
                Some("ne") => Value::Bool(false),
                _ => Value::Nothing,
            };
            let contrast = flag(it.next());
            let motion = flag(it.next());
            Ok(Value::record(&[("scheme", scheme), ("contrast", contrast), ("reduce_motion", motion)]))
        }
        Err(abi::Error::InvalidArgs) => Err(a.err(code::ARGS, THEME_USAGE)),
        Err(e) => Err(Flow::Err(ShError::sys(e, "změna vzhledu se nepovedla").at(a.span))),
    }
}

/// Povoleni, ktera jde odvolat (slovo prikazu).
const PERMS: &[&str] = &["microphone"];

fn permissions(host: &mut dyn Host, a: &Args) -> R<Value> {
    let (app, perm) = match word(a, 0, "permissions")?.as_deref() {
        None => (String::new(), String::new()),
        Some("revoke") => {
            let app = word(a, 1, "permissions")?
                .ok_or_else(|| a.err(code::ARGS, "permissions revoke: chybí aplikace (identita z výpisu)"))?;
            let perm = word(a, 2, "permissions")?.unwrap_or_else(|| String::from("microphone"));
            if !PERMS.contains(&perm.as_str()) {
                return Err(a.err(code::ARGS, alloc::format!("permissions revoke: neznámé povolení '{perm}' (známá: microphone)")));
            }
            (app, perm)
        }
        Some(_) => {
            return Err(a.err(code::ARGS, "permissions: bez argumentu výpis, nebo `permissions revoke <aplikace> [microphone]`"));
        }
    };
    let text = host.permissions(&app, &perm).map_err(|e| Flow::Err(ShError::sys(e, "povolení aplikací").at(a.span)))?;
    if !app.is_empty() {
        return Ok(Value::Str(text));
    }
    // Vypis: radek `aplikace\tpovoleni\tallow|deny` -> tabulka.
    let rows = text
        .lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let (app, perm, d) = (f.next()?, f.next()?, f.next()?);
            Some(Value::record(&[("app", Value::str(app)), ("permission", Value::str(perm)), ("allowed", Value::Bool(d == "allow"))]))
        })
        .collect();
    Ok(Value::List(rows))
}
