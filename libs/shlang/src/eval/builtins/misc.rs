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
use crate::vfs::diff::diff_value;

pub(super) fn call(it: &mut Interp, host: &mut dyn Host, name: &str, a: Args, _input: Value) -> R<Value> {
    match name {
        "now" => match host.now() {
            Some(t) => Ok(Value::Time(t)),
            None => Err(a.err(abi::Error::NotSupported.name(), "čas není známý")),
        },
        "sleep" => match a.pos(0) {
            Some(Value::Duration(d)) => {
                host.sleep(*d);
                if host.interrupted() {
                    return Err(Flow::Err(ShError::new(code::INTERRUPTED, "přerušeno (Ctrl-C)")));
                }
                Ok(Value::Nothing)
            }
            _ => Err(a.err(code::TYPE, "sleep čeká dobu, např. sleep 2s")),
        },
        "undo" => {
            if !it.draft.is_empty() {
                return Err(a.err(code::ARGS, "undo patří na samostatný řádek"));
            }
            match it.undo.undo(host.fs()) {
                Ok(e) => Ok(Value::record(&[("undone", Value::Str(e.line)), ("changes", diff_value(&e.diff))])),
                Err(abi::Error::NotFound) => Err(a.err(code::NO_UNDO, "není co vrátit")),
                Err(e) => Err(Flow::Err(ShError::sys(e, "vrácení se nepovedlo").at(a.span))),
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
        "tasks" => {
            use crate::host::TasksOp;
            let action = match a.pos(0) {
                None => None,
                Some(_) => Some(str_arg(&a, 0, "akce")?),
            };
            let id = || -> R<u64> {
                int_arg(&a, 1, "id")?.map(|v| v as u64).ok_or_else(|| a.err(code::ARGS, "tasks: chybí číslo úlohy"))
            };
            let op = match action.as_deref() {
                None | Some("list") => TasksOp::List,
                Some("cancel") => TasksOp::Cancel(id()?),
                Some("pause") => TasksOp::Pause(id()?),
                Some("resume") => TasksOp::Resume(id()?),
                Some("log") => {
                    let n = match a.named("lines") {
                        Some(Value::Int(n)) => (*n).clamp(1, 1000) as u32,
                        _ => 20,
                    };
                    TasksOp::Log(id()?, n)
                }
                Some(other) => {
                    return Err(Flow::Err(
                        ShError::new(code::ARGS, alloc::format!("tasks: neznámá akce '{other}'"))
                            .at(a.span)
                            .hint("tasks, tasks cancel|pause|resume|log <id>"),
                    ));
                }
            };
            match host.tasks(op) {
                Some(r) => r.map_err(|e| Flow::Err(e.at(a.span))),
                None => Err(Flow::Err(ShError::sys(abi::Error::NotSupported, "přehled úloh tu není (služba taskd neběží)").at(a.span))),
            }
        }
        "cancel" => {
            let v = a.pos(0).cloned().ok_or_else(|| a.err(code::ARGS, "cancel: chybí úloha"))?;
            it.cancel_value(host, &v, a.span)
        }
        // Rizeni uloh terminalu: `fg` obsluhuje relace (radek ve vlakne);
        // sem dojde jen mimo terminal (skript, test).
        "fg" => Err(a.err(code::ARGS, "fg funguje jen v terminálu (řádky na pozadí: Ctrl+Z, příkaz &)")),
        "kill" => {
            let id = int_arg(&a, 0, "id")?.unwrap_or(0) as u64;
            match it.jobs.iter_mut().find(|j| j.id == id) {
                Some(j) if j.state == JobState::Pending => {
                    j.state = JobState::Killed;
                    Ok(Value::Nothing)
                }
                Some(_) => Err(a.err(code::ARGS, alloc::format!("úloha {id} už neběží"))),
                None => Err(a.err(code::UNKNOWN, alloc::format!("úloha {id} neexistuje"))),
            }
        }
        "history" => Ok(it.history.to_value()),
        "explain" => {
            let c = match a.pos(0) {
                Some(Value::Error(e)) => e.code.clone(),
                Some(Value::Str(s)) => s.clone(),
                Some(Value::Int(n)) => abi::Error::from_code(*n).map(|e| e.name().into()).unwrap_or_default(),
                _ => return Err(a.err(code::ARGS, "explain čeká kód chyby (NOT_FOUND, SH_PARSE, -8) nebo hlášku")),
            };
            match crate::explain::explain(&c) {
                Some(t) => Ok(Value::Str(t)),
                None => Err(a.err(code::UNKNOWN, alloc::format!("kód ani hlášku '{c}' neznám"))),
            }
        }
        "help" => Ok(Value::Str(match a.pos(0) {
            Some(_) => {
                let n = str_arg(&a, 0, "příkaz")?;
                match builtin(&n) {
                    Some(s) => crate::doc::help_text(s),
                    None => return Err(a.err(code::UNKNOWN, alloc::format!("příkaz '{n}' neznám"))),
                }
            }
            None => help_overview(),
        })),
        "run" => {
            let v = a.pos(0).ok_or_else(|| a.err(code::ARGS, "chybí skript"))?;
            let p = it.paths(v)?.into_iter().next().unwrap_or_default();
            // Skript ma priponu .mos (MagnumOpus Shell); jiny soubor (log,
            // text) by se rozbil na prvnim radku chybou, ktera ukazuje
            // do prikazoveho radku - radsi rict, o co jde.
            if !p.to_lowercase().ends_with(".mos") {
                return Err(a.err(
                    code::ARGS,
                    alloc::format!("{p} není skript shellu: skripty mají příponu .mos a hlavičku `lang 1`; obsah ukáže open {p} | from text"),
                ));
            }
            let data = it.fs_read(host, &p)?;
            let src = String::from_utf8(data).map_err(|_| a.err(code::FORMAT, "skript není text v UTF-8"))?;
            it.script_inner(host, &src, Some(&p)).map_err(Flow::Err)?;
            Ok(Value::Nothing)
        }
        "error" => {
            let msg = str_arg(&a, 0, "zpráva")?;
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
        "shutdown" | "reboot" => {
            let reboot = name == "reboot";
            let question = if reboot { "Restartovat počítač?" } else { "Vypnout počítač?" };
            if !a.flag("yes") && !host.confirm(question, &Value::Nothing) {
                return Err(Flow::Err(ShError::new(code::REJECTED, "zrušeno, počítač běží dál").at(a.span)));
            }
            match host.power(reboot) {
                Ok(()) => Ok(Value::Nothing),
                Err(e) => Err(Flow::Err(
                    ShError::sys(e, if reboot { "restart se nepovedl" } else { "vypnutí se nepovedlo" }).at(a.span),
                )),
            }
        }
        "loglevel" => {
            let target = match a.pos(0) {
                Some(Value::Str(s)) => s.clone(),
                None => String::new(),
                Some(_) => return Err(a.err(code::ARGS, "loglevel: modul je text")),
            };
            let level = match a.pos(1) {
                Some(Value::Str(s)) => s.clone(),
                None if target.is_empty() => String::new(),
                _ => return Err(a.err(code::ARGS, "loglevel: chybí úroveň (error, warn, info, debug, trace, vychozi)")),
            };
            match host.log_level(&target, &level) {
                Ok(text) => Ok(Value::Str(text)),
                Err(e) => Err(Flow::Err(ShError::sys(e, "nastavení úrovně logu se nepovedlo").at(a.span))),
            }
        }
        "report" => match host.report() {
            Ok(text) => Ok(Value::Str(text)),
            Err(e) => Err(Flow::Err(ShError::sys(e, "balíček pro vývojáře se nepovedl").at(a.span))),
        },
        _ => Err(a.err(code::UNKNOWN, alloc::format!("neznámý příkaz '{name}'"))),
    }
}

/// Prehled `help`: prikazy po skupinach, na radek jmeno a kratky popis
/// v zarovnanem sloupci (dlouhy popis okno zalomi s odsazenim pod
/// popisem). Plne jmeno a prepinace az v `help <prikaz>`.
pub fn help_overview() -> String {
    let w = BUILTINS.iter().map(|b| b.name.chars().count()).max().unwrap_or(8);
    let mut s = String::from("Příkazy podle skupin (podrobně: help <příkaz>):\n");
    let mut seen: Vec<&str> = Vec::new();
    for (title, names) in crate::sig::GROUPS {
        s.push_str(&alloc::format!("\n{title}\n"));
        for n in *names {
            if let Some(b) = builtin(n) {
                seen.push(b.name);
                s.push_str(&alloc::format!("  {:<w$}  {}\n", b.name, b.help));
            }
        }
    }
    // Prikaz bez skupiny (pridany pozdeji) nesmi zmizet.
    let rest: Vec<_> = BUILTINS.iter().filter(|b| !seen.contains(&b.name)).collect();
    if !rest.is_empty() {
        s.push_str("\nOstatní\n");
        for b in rest {
            s.push_str(&alloc::format!("  {:<w$}  {}\n", b.name, b.help));
        }
    }
    s.push_str("\nZápis C (volání funkcí): v okně terminálu Ctrl+F převede rozepsaný řádek mezi zápisy B a C.\n");
    s
}

/// `preview { ... }`: blok nanecisto (dalsi vrstva klonu), diff,
/// prevzeti do konceptu radku po potvrzeni.
fn preview(it: &mut Interp, host: &mut dyn Host, a: &Args) -> R<Value> {
    let body = a.pos(0).cloned().ok_or_else(|| a.err(code::ARGS, "preview: chybí blok"))?;
    it.draft.push();
    let depth = it.draft.depth();
    let r = it.call_value(host, &body, Vec::new());
    if let Err(e) = r {
        // Vnorene vrstvy (preruseny vnitrni preview) i tuhle zahodit.
        while it.draft.depth() >= depth {
            it.draft.discard_top(host.fs());
        }
        return Err(e);
    }
    let diff = match it.draft.changes(host.fs()) {
        Ok(d) => d,
        Err(e) => {
            it.draft.discard_top(host.fs());
            return Err(Flow::Err(ShError::sys(e, "změny nejde spočítat")));
        }
    };
    let table = diff_value(&diff);
    if diff.is_empty() {
        it.draft.discard_top(host.fs());
        return Ok(Value::record(&[("applied", Value::Bool(false)), ("changes", table)]));
    }
    let ok = a.flag("yes") || host.confirm(&alloc::format!("Náhled: {} změn. Převzít?", diff.len()), &table);
    if !ok {
        it.draft.discard_top(host.fs());
        return Err(Flow::Err(
            ShError::new(code::REJECTED, "změny nebyly převzaty (nic se nezměnilo)").at(a.span),
        ));
    }
    let applied = it.draft.apply_top(host.fs()).map_err(|e| Flow::Err(ShError::sys(e, "změny nejde převzít")))?;
    if !applied.conflicts.is_empty() || !applied.complete {
        let more = if applied.complete { "" } else { " a další" };
        return Err(Flow::Err(ShError::new(
            code::CONFLICT,
            alloc::format!(
                "{} změn náhledu se nepřevzalo: {}{more}",
                applied.conflicts.len(),
                applied.conflicts.join(", ")
            ),
        )));
    }
    it.confirmed = true;
    Ok(Value::record(&[("applied", Value::Bool(true)), ("changes", table)]))
}
