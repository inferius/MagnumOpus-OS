//! Vzdaleny pristup SSH (`docs/cs/ssh.md`): `sshd` (sluzba pro cely
//! pocitac, zapnuti a vypnuti je ukon spravy), `ssh-key` (klice SSH
//! vlastniho uctu) a `remote-access` (povoleni vzdaleneho pristupu uctu).
//! Vse dela hostitel (init a sluzba uctu); heslo se pta pres `Host::ask`
//! a nikde se nevypisuje.

use alloc::string::String;
use alloc::vec::Vec;

use super::super::call::Args;
use super::super::{Flow, Interp, R};
use crate::error::{ShError, code};
use crate::host::Host;
use crate::value::Value;

pub(super) fn call(it: &mut Interp, host: &mut dyn Host, name: &str, a: Args, input: Value) -> R<Value> {
    match name {
        "sshd" => sshd(host, &a),
        "ssh-key" => ssh_key(it, host, &a, input),
        _ => remote_access(host, &a),
    }
}

fn word(a: &Args, i: usize) -> Option<String> {
    match a.pos(i) {
        Some(Value::Str(s)) => Some(s.clone()),
        Some(Value::Int(n)) => Some(alloc::format!("{n}")),
        _ => None,
    }
}

fn sys(e: abi::Error, what: &str, a: &Args) -> Flow {
    let err = match e {
        abi::Error::NotSupported => ShError::new(e.name(), alloc::format!("{what}: vzdálený přístup tu není (konzole, test)")),
        abi::Error::AccessDenied if what.starts_with("sshd") => ShError::new(e.name(), alloc::format!("{what}: úkon správy nebyl potvrzen"))
            .hint("zapnout a vypnout smí jen člověk s rolí správy u počítače (systémový dialog)"),
        _ => ShError::sys(e, what),
    };
    Flow::Err(err.at(a.span))
}

/// Text "klic\thodnota" -> zaznam.
fn record(text: &str) -> Value {
    let fields: Vec<(String, Value)> = text
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, v)| {
            let v = match v {
                "on" | "yes" => Value::Bool(true),
                "off" | "no" => Value::Bool(false),
                _ => match v.parse::<i64>() {
                    Ok(n) => Value::Int(n),
                    Err(_) => Value::str(v),
                },
            };
            (String::from(k), v)
        })
        .collect();
    let refs: Vec<(&str, Value)> = fields.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    Value::record(&refs)
}

fn sshd(host: &mut dyn Host, a: &Args) -> R<Value> {
    let op = match word(a, 0).as_deref() {
        None | Some("status") => "sshd-status",
        Some("enable") => "sshd-enable",
        Some("disable") => "sshd-disable",
        Some(_) => return Err(a.err(code::ARGS, "sshd: status, enable nebo disable")),
    };
    let text = host.ssh(op, "", "").map_err(|e| sys(e, &alloc::format!("sshd {}", op.trim_start_matches("sshd-")), a))?;
    if op == "sshd-status" { Ok(record(&text)) } else { Ok(Value::Str(text)) }
}

/// Heslo uctu (dotaz s maskovanim); bez odpovedi = zruseno.
fn password(host: &mut dyn Host, a: &Args, why: &str) -> R<String> {
    host.ask(&alloc::format!("Heslo vašeho účtu ({why})"), true, "")
        .filter(|p| !p.is_empty())
        .ok_or_else(|| Flow::Err(ShError::new(code::REJECTED, alloc::format!("{why}: zrušeno, bez hesla nic nezměním")).at(a.span)))
}

fn ssh_key(it: &mut Interp, host: &mut dyn Host, a: &Args, input: Value) -> R<Value> {
    match word(a, 0).as_deref() {
        None | Some("list") => {
            let text = host.ssh("key-list", "", "").map_err(|e| sys(e, "ssh-key list", a))?;
            let rows = text
                .lines()
                .filter_map(|l| {
                    let f: Vec<&str> = l.split('\t').collect();
                    let [id, algo, fp, comment, added] = f.as_slice() else { return None };
                    Some(Value::record(&[
                        ("id", Value::Int(id.parse().ok()?)),
                        ("algo", Value::str(*algo)),
                        ("fingerprint", Value::str(*fp)),
                        ("comment", Value::str(*comment)),
                        ("added", Value::Time(added.parse::<i64>().unwrap_or(0).saturating_mul(1_000_000_000))),
                    ]))
                })
                .collect();
            Ok(Value::List(rows))
        }
        Some("add") => {
            // Radek klice: ze souboru (`ssh-key add ~/.ssh/id_ed25519.pub`),
            // z roury (`-`) nebo primo jako text.
            let src = word(a, 1).ok_or_else(|| a.err(code::ARGS, "ssh-key add: chybí soubor s klíčem (*.pub) nebo -"))?;
            let text = if src == "-" {
                match input {
                    Value::Str(s) => s,
                    Value::Bytes(b) => String::from_utf8_lossy(&b).into_owned(),
                    _ => return Err(a.err(code::ARGS, "ssh-key add -: čekám klíč na vstupu (text)")),
                }
            } else if src.starts_with("ssh-") || src.starts_with("sk-") {
                src
            } else {
                let p = it.abs(&src);
                let data = it.fs_read(host, &p)?;
                String::from_utf8_lossy(&data).into_owned()
            };
            let line = text
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty() && !l.starts_with('#'))
                .ok_or_else(|| a.err(code::ARGS, "ssh-key add: v souboru není žádný klíč"))?;
            if line.contains("PRIVATE KEY") {
                return Err(a.err(code::ARGS, "ssh-key add: tohle je soukromý klíč; přidejte veřejný (*.pub)"));
            }
            let pw = password(host, a, "přidání klíče SSH")?;
            let text = host.ssh("key-add", line, &pw).map_err(|e| match e {
                abi::Error::AlreadyExists => a.err(e.name(), "ssh-key add: tenhle klíč už účet má"),
                abi::Error::NotSupported => a.err(e.name(), "ssh-key add: podporované jsou klíče ed25519 (i FIDO sk-ssh-ed25519); RSA a ECDSA zatím ne"),
                abi::Error::InvalidArgs => a.err(e.name(), "ssh-key add: neplatný řádek klíče (čekám `typ base64 [komentář]`)"),
                abi::Error::AccessDenied => a.err(e.name(), "ssh-key add: špatné heslo"),
                _ => sys(e, "ssh-key add", a),
            })?;
            let mut f = text.trim().split('\t');
            let id = f.next().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
            let fp = f.next().unwrap_or("");
            Ok(Value::record(&[("id", Value::Int(id)), ("fingerprint", Value::str(fp))]))
        }
        Some("remove") => {
            let id = word(a, 1).ok_or_else(|| a.err(code::ARGS, "ssh-key remove: chybí číslo klíče (ssh-key list)"))?;
            host.ssh("key-remove", &id, "").map_err(|e| match e {
                abi::Error::NotFound => a.err(e.name(), alloc::format!("ssh-key remove: klíč {id} účet nemá")),
                _ => sys(e, "ssh-key remove", a),
            })?;
            Ok(Value::Nothing)
        }
        Some(_) => Err(a.err(code::ARGS, "ssh-key: list, add <soubor|->, remove <číslo>")),
    }
}

fn remote_access(host: &mut dyn Host, a: &Args) -> R<Value> {
    let text = match word(a, 0).as_deref() {
        None => host.ssh("access", "", "").map_err(|e| sys(e, "remote-access", a))?,
        Some("on") => {
            let pw = password(host, a, "povolení vzdáleného přístupu")?;
            host.ssh("access-on", "", &pw).map_err(|e| match e {
                abi::Error::AccessDenied => a.err(e.name(), "remote-access on: špatné heslo"),
                _ => sys(e, "remote-access on", a),
            })?
        }
        Some("off") => host.ssh("access-off", "", "").map_err(|e| sys(e, "remote-access off", a))?,
        Some(_) => return Err(a.err(code::ARGS, "remote-access: on nebo off, bez argumentu stav")),
    };
    Ok(Value::record(&[("remote_access", Value::Bool(text.trim() == "on"))]))
}
