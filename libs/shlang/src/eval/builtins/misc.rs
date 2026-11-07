//! Cas, ulohy, undo, nahled zmen, historie, napoveda, skripty, chyby.

use alloc::string::String;
use alloc::vec::Vec;

use super::super::call::Args;
use super::super::{Flow, Interp, JobState, R};
use super::{int_arg, str_arg};
use crate::error::{ShError, code};
use crate::host::Host;
use crate::sig::{BUILTINS, builtin};
use crate::value::Value;
use crate::vfs::overlay::diff_value;

pub(super) fn call(it: &mut Interp, host: &mut dyn Host, name: &str, a: Args, _input: Value) -> R<Value> {
    match name {
        "now" => match host.now() {
            Some(t) => Ok(Value::Time(t)),
            None => Err(a.err(abi::Error::NotSupported.name(), "cas neni znamy")),
        },
        "sleep" => match a.pos(0) {
            Some(Value::Duration(d)) => {
                host.sleep(*d);
                if host.interrupted() {
                    return Err(Flow::Err(ShError::new(code::INTERRUPTED, "preruseno (Ctrl-C)")));
                }
                Ok(Value::Nothing)
            }
            _ => Err(a.err(code::TYPE, "sleep ceka dobu, napr. sleep 2s")),
        },
        "undo" => {
            if !it.overlay.is_empty() {
                return Err(a.err(code::ARGS, "undo patri na samostatny radek"));
            }
            match it.undo.undo(host.fs()) {
                Ok(e) => Ok(Value::record(&[("undone", Value::Str(e.line)), ("changes", diff_value(&e.diff))])),
                Err(abi::Error::NotFound) => Err(a.err(code::NO_UNDO, "neni co vratit")),
                Err(e) => Err(Flow::Err(ShError::sys(e, "vraceni se nepovedlo").at(a.span))),
            }
        }
        "preview" => preview(it, host, &a),
        "jobs" => Ok(Value::List(
            it.jobs
                .iter()
                .map(|j| {
                    let state = match &j.state {
                        JobState::Pending => "waiting",
                        JobState::Done(_) => "done",
                        JobState::Failed(_) => "failed",
                        JobState::Killed => "killed",
                    };
                    Value::record(&[
                        ("id", Value::Int(j.id as i64)),
                        ("state", Value::str(state)),
                        ("command", Value::Str(j.line.clone())),
                    ])
                })
                .collect(),
        )),
        "wait" => match int_arg(&a, 0, "id")? {
            Some(id) => it.run_job(host, id as u64).map_err(Flow::Err),
            None => {
                let ids: Vec<u64> = it.jobs.iter().filter(|j| j.state == JobState::Pending).map(|j| j.id).collect();
                let mut out = Vec::new();
                for id in ids {
                    out.push(it.run_job(host, id).map_err(Flow::Err)?);
                }
                Ok(Value::List(out))
            }
        },
        "kill" => {
            let id = int_arg(&a, 0, "id")?.unwrap_or(0) as u64;
            match it.jobs.iter_mut().find(|j| j.id == id) {
                Some(j) if j.state == JobState::Pending => {
                    j.state = JobState::Killed;
                    Ok(Value::Nothing)
                }
                Some(_) => Err(a.err(code::ARGS, alloc::format!("uloha {id} uz nebezi"))),
                None => Err(a.err(code::UNKNOWN, alloc::format!("uloha {id} neexistuje"))),
            }
        }
        "history" => Ok(it.history.to_value()),
        "explain" => {
            let c = match a.pos(0) {
                Some(Value::Error(e)) => e.code.clone(),
                Some(Value::Str(s)) => s.clone(),
                Some(Value::Int(n)) => abi::Error::from_code(*n).map(|e| e.name().into()).unwrap_or_default(),
                _ => return Err(a.err(code::ARGS, "explain ceka kod chyby (NOT_FOUND, SH_PARSE, -8) nebo hlasku")),
            };
            match crate::explain::explain(&c) {
                Some(t) => Ok(Value::Str(t)),
                None => Err(a.err(code::UNKNOWN, alloc::format!("kod ani hlasku '{c}' neznam"))),
            }
        }
        "help" => Ok(Value::Str(match a.pos(0) {
            Some(_) => {
                let n = str_arg(&a, 0, "prikaz")?;
                match builtin(&n) {
                    Some(s) => help_for(s),
                    None => return Err(a.err(code::UNKNOWN, alloc::format!("prikaz '{n}' neznam"))),
                }
            }
            None => {
                let mut s = String::from("Prikazy (kratke jmeno / plne jmeno):\n");
                for b in BUILTINS {
                    s.push_str(&alloc::format!("  {:<12} {:<18} {}\n", b.name, b.full, b.help));
                }
                s.push_str("\nPodrobne: help <prikaz>. Zapis C: fmt --style c.\n");
                s
            }
        })),
        "run" => {
            let v = a.pos(0).ok_or_else(|| a.err(code::ARGS, "chybi skript"))?;
            let p = it.paths(v)?.into_iter().next().unwrap_or_default();
            let data = it.fs_read(host, &p)?;
            let src = String::from_utf8(data).map_err(|_| a.err(code::FORMAT, "skript neni text v UTF-8"))?;
            it.script_inner(host, &src, Some(&p)).map_err(Flow::Err)?;
            Ok(Value::Nothing)
        }
        "error" => {
            let msg = str_arg(&a, 0, "zprava")?;
            let c = match a.named("code") {
                Some(Value::Str(c)) => c.clone(),
                _ => code::USER.into(),
            };
            Err(Flow::Err(ShError::new(&c, msg).at(a.span)))
        }
        "exit" => {
            it.exit = true;
            Ok(Value::Nothing)
        }
        _ => Err(a.err(code::UNKNOWN, alloc::format!("neznamy prikaz '{name}'"))),
    }
}

/// Napoveda k prikazu: popis, obe jmena, pouziti, prepinace, vstup/vystup.
pub fn help_for(s: &crate::sig::Sig) -> String {
    let mut out = alloc::format!("{} ({}): {}\n  pouziti: {}\n", s.name, s.full, s.help, crate::parse::usage(s));
    for f in s.flags {
        let short = f.short.map(|c| alloc::format!("-{c}, ")).unwrap_or_default();
        out.push_str(&alloc::format!("  {short}--{}: {}\n", f.long, f.help));
    }
    out.push_str(&alloc::format!("  zapis C: {}(...)\n", s.name.replace('-', "_")));
    out
}

/// `preview { ... }`: blok nanecisto, diff, prevzeti po potvrzeni.
fn preview(it: &mut Interp, host: &mut dyn Host, a: &Args) -> R<Value> {
    let body = a.pos(0).cloned().ok_or_else(|| a.err(code::ARGS, "preview: chybi blok"))?;
    let before = it.overlay.clone();
    it.preview_depth += 1;
    let r = it.call_value(host, &body, Vec::new());
    it.preview_depth -= 1;
    if let Err(e) = r {
        it.overlay = before;
        return Err(e);
    }
    let fs = host.fs();
    let old = before.changes(fs).map_err(|e| ShError::sys(e, "zmeny nejde spocitat"))?;
    let new = it.overlay.changes(fs).map_err(|e| ShError::sys(e, "zmeny nejde spocitat"))?;
    let diff: Vec<_> = new.into_iter().filter(|d| !old.contains(d)).collect();
    let table = diff_value(&diff);
    if diff.is_empty() {
        return Ok(Value::record(&[("applied", Value::Bool(false)), ("changes", table)]));
    }
    let ok = a.flag("yes") || host.confirm(&alloc::format!("Nahled: {} zmen. Prevzit?", diff.len()), &table);
    if !ok {
        it.overlay = before;
        return Err(Flow::Err(
            ShError::new(code::REJECTED, "zmeny nebyly prevzaty (nic se nezmenilo)").at(a.span),
        ));
    }
    it.confirmed = true;
    Ok(Value::record(&[("applied", Value::Bool(true)), ("changes", table)]))
}
