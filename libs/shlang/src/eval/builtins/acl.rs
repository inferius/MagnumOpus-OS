//! Prikaz `acl`: opravneni vlastnich souboru a slozek (popis opravneni
//! FS, `crate::acl`). Vypis, pridani a odebrani grantu; prava vynucuje FS
//! (menit smi vlastnik nebo kdo ma Spravovat). Zmena jde primo do FS
//! (ne do konceptu radku) a `undo` ji nevraci.

use alloc::string::String;
use alloc::vec::Vec;

use super::super::call::Args;
use super::super::{Flow, Interp, R};
use crate::acl::{self, Account, Acl, grant};
use crate::error::{ShError, code};
use crate::host::Host;
use crate::value::Value;

const USAGE: &str = "acl <cesta>, acl add <kdo> <view|run|edit|manage> <cesta> [--inherit], acl remove <kdo> <cesta>";

pub(super) fn call(it: &mut Interp, host: &mut dyn Host, a: Args) -> R<Value> {
    match a.pos(0) {
        Some(Value::Str(s)) if s == "add" && a.pos.len() > 1 => change(it, host, &a, true),
        Some(Value::Str(s)) if s == "remove" && a.pos.len() > 1 => change(it, host, &a, false),
        Some(_) if a.pos.len() == 1 => {
            let p = path(it, &a, 0)?;
            it.check_read(&p)?;
            show(host, &p).map_err(|e| fail(e, &p, false, &a))
        }
        _ => Err(a.err(code::ARGS, alloc::format!("acl: {USAGE}"))),
    }
}

fn path(it: &Interp, a: &Args, i: usize) -> R<String> {
    match a.pos(i) {
        Some(Value::Str(s)) => Ok(it.abs(s)),
        Some(Value::File(f)) => Ok(it.abs(&f.path)),
        Some(other) => Err(a.err(code::TYPE, alloc::format!("acl: cesta je text, dostal jsem {}", other.type_name()))),
        None => Err(a.err(code::ARGS, alloc::format!("acl: chybí cesta ({USAGE})"))),
    }
}

/// Chyba FS jednou vetou podle toho, co se delalo.
fn fail(e: abi::Error, p: &str, write: bool, a: &Args) -> Flow {
    let err = match e {
        abi::Error::AccessDenied if write => ShError::new(
            e.name(),
            alloc::format!("oprávnění {p} nejde změnit: smí je měnit jen vlastník nebo kdo má právo Spravovat"),
        )
        .hint("vlastníka a vaše právo ukáže acl <cesta>"),
        abi::Error::NotFound => ShError::new(e.name(), alloc::format!("{p} neexistuje nebo k ní nemáte žádné právo")),
        abi::Error::NotSupported => ShError::new(e.name(), alloc::format!("FS u {p} oprávnění nevede")),
        _ if write => ShError::sys(e, alloc::format!("oprávnění {p} nejde změnit")),
        _ => ShError::sys(e, alloc::format!("oprávnění {p} nejde zjistit")),
    };
    Flow::Err(err.at(a.span))
}

fn accounts(host: &mut dyn Host) -> Vec<Account> {
    host.accounts().unwrap_or_default()
}

/// Rozsah grantu: jen objekt, jen potomci, oboji.
fn scope(flags: u8) -> &'static str {
    match (flags & grant::THIS != 0, flags & grant::INHERIT != 0) {
        (true, true) => "this+children",
        (false, true) => "children",
        _ => "this",
    }
}

/// Radek grantu. `from`: `None` = vlastni grant, `Some(slozka)` =
/// zdedeny (prazdna slozka = mimo pripojene cesty relace, puvod nevime).
fn grant_row(who: u64, rights: u8, flags: u8, accounts: &[Account], from: Option<&str>) -> Value {
    let (level, label) = acl::level_of(rights);
    Value::record(&[
        ("who", Value::Str(acl::name_of(who, accounts))),
        ("kind", Value::str(acl::kind_of(who))),
        ("level", Value::Str(level)),
        ("label", Value::Str(label)),
        ("scope", Value::str(if from.is_some() { "inherited" } else { scope(flags) })),
        ("from", from.filter(|f| !f.is_empty()).map_or(Value::Nothing, Value::str)),
    ])
}

/// Vypis: vlastnik, vase uroven, dedeni a granty (vlastni, pak zdedene
/// s puvodem od korene; jedna operace FS `ACL_VIEW`).
fn show(host: &mut dyn Host, p: &str) -> Result<Value, abi::Error> {
    let view = host.fs().acl_view(p)?;
    let accounts = accounts(host);
    let perms = &view.perms;
    let protected = perms.acl.as_ref().is_some_and(|a| a.protected);
    let mut rows: Vec<Value> =
        perms.acl.iter().flat_map(|a| a.grants.iter()).map(|g| grant_row(g.who, g.rights, g.flags, &accounts, None)).collect();
    rows.extend(view.inherited.iter().map(|g| grant_row(g.who, g.rights, grant::INHERIT, &accounts, Some(&g.from))));
    let (level, label) = acl::level_of(perms.rights);
    Ok(Value::record(&[
        ("path", Value::str(p)),
        ("owner", Value::Str(acl::name_of(perms.owner, &accounts))),
        ("your_level", Value::Str(level)),
        ("your_label", Value::Str(label)),
        ("inherits", Value::Bool(!protected)),
        ("grants", Value::List(rows)),
    ]))
}

/// Cislo prijemce z argumentu (jmeno, `everyone`, `users`, cislo).
fn who(host: &mut dyn Host, a: &Args, i: usize) -> R<u64> {
    let unknown = |n: &str| {
        a.err(
            code::ARGS,
            alloc::format!("acl: neznámý uživatel nebo skupina '{n}' (zadejte jméno účtu, everyone, users nebo číslo)"),
        )
    };
    match a.pos(i) {
        Some(Value::Int(n)) if *n >= 0 => Ok(*n as u64),
        Some(Value::Str(s)) => {
            let accounts = accounts(host);
            acl::who_of(s, &accounts).ok_or_else(|| unknown(s))
        }
        Some(other) => Err(unknown(&other.to_text())),
        None => Err(a.err(code::ARGS, alloc::format!("acl: chybí kdo ({USAGE})"))),
    }
}

fn change(it: &mut Interp, host: &mut dyn Host, a: &Args, add: bool) -> R<Value> {
    let cmd = if add { "acl add" } else { "acl remove" };
    if !it.draft.is_empty() {
        return Err(a.err(code::ARGS, alloc::format!("{cmd} patří na samostatný řádek (oprávnění se mění hned, ne nanečisto)")));
    }
    let who = who(host, a, 1)?;
    let (rights, p) = if add {
        let level = match a.pos(2) {
            Some(Value::Str(s)) => s.clone(),
            _ => return Err(a.err(code::ARGS, alloc::format!("acl add: chybí úroveň view, run, edit nebo manage ({USAGE})"))),
        };
        let r = acl::level_rights(&level)
            .ok_or_else(|| a.err(code::ARGS, alloc::format!("acl add: neznámá úroveň '{level}' (view, run, edit, manage)")))?;
        (r, path(it, a, 3)?)
    } else {
        (0, path(it, a, 2)?)
    };
    if a.pos.len() > if add { 4 } else { 3 } {
        return Err(a.err(code::ARGS, alloc::format!("{cmd}: moc argumentů ({USAGE})")));
    }
    it.check_write(&p)?;
    let cur = host.fs().acl_view(&p).map_err(|e| fail(e, &p, false, a))?.perms;
    let next: Option<Acl> = if add {
        let flags = if a.flag("inherit") { grant::THIS | grant::INHERIT } else { grant::THIS };
        Some(acl::with_grant(cur.acl, who, rights, flags))
    } else {
        let name = acl::name_of(who, &accounts(host));
        let acl = acl::without_grant(cur.acl, who)
            .ok_or_else(|| a.err(code::ARGS, alloc::format!("acl remove: {name} nemá na {p} vlastní grant (zděděný se odebírá na předkovi)")))?;
        // Prazdny popis, ktery dedi, je totez co zadny.
        (acl.protected || !acl.grants.is_empty()).then_some(acl)
    };
    host.fs().set_acl(&p, next.as_ref()).map_err(|e| fail(e, &p, true, a))?;
    show(host, &p).map_err(|e| fail(e, &p, false, a))
}
