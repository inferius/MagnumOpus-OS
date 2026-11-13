//! Prikazy nad soubory: ls, cd, pwd, open, save, rm, cp, mv, mkdir,
//! exists, df.

use alloc::vec::Vec;

use super::super::call::Args;
use super::super::{Flow, Interp, R};
use crate::error::{ShError, code};
use crate::host::{Host, NodeKind};
use crate::value::Value;
use crate::vfs::path;

pub(super) fn call(it: &mut Interp, host: &mut dyn Host, name: &str, a: Args, input: Value) -> R<Value> {
    match name {
        "ls" => ls(it, host, &a),
        "cd" => {
            if a.flag("history") {
                // Nejnovejsi prvni; cislo = argument pro `cd -N`.
                let rows = it
                    .dirs_back
                    .iter()
                    .rev()
                    .enumerate()
                    .map(|(i, d)| Value::record(&[("back", Value::Int(i as i64 + 1)), ("path", Value::str(d))]))
                    .collect();
                return Ok(Value::List(rows));
            }
            // `cd -` / `cd -N` zpet v historii slozek, `cd +` vpred.
            let nav = match a.pos(0) {
                Some(Value::Str(s)) if s == "-" => Some(-1),
                Some(Value::Str(s)) if s == "+" => Some(1),
                Some(Value::Int(n)) if *n < 0 => Some(*n),
                _ => None,
            };
            if let Some(n) = nav {
                return cd_nav(it, host, &a, n);
            }
            let p = match a.pos(0) {
                Some(v) => it.paths(v)?.into_iter().next().unwrap_or_else(|| it.home.clone()),
                None => it.home.clone(),
            };
            let m = it.fs_stat(host, &p)?;
            if m.kind != NodeKind::Dir {
                return Err(a.err(abi::Error::NotADirectory.name(), alloc::format!("{p} není složka")));
            }
            if p != it.cwd {
                let old = core::mem::replace(&mut it.cwd, p);
                it.dirs_back.retain(|d| *d != old);
                it.dirs_back.push(old);
                if it.dirs_back.len() > DIRS_MAX {
                    it.dirs_back.remove(0);
                }
                it.dirs_fwd.clear();
            }
            Ok(Value::Nothing)
        }
        "pwd" => Ok(Value::Str(it.cwd.clone())),
        "open" => {
            let p = one_path(it, &a)?;
            // Velky soubor se necte cely: hodnota "soubor" (obsah prectou
            // prikazy, ktere ho potrebuji); zobrazeni ukaze souhrn.
            let m = it.fs_stat(host, &p)?;
            if m.kind == NodeKind::File && m.size > OPEN_INLINE {
                let head = it.fs_read_range(host, &p, 0, 512)?;
                let kind = crate::filekind::kind(&head, false);
                return Ok(Value::File(crate::value::FileRef { path: p, size: m.size, kind }));
            }
            Ok(Value::Bytes(it.fs_read(host, &p)?))
        }
        "save" => {
            let p = one_path(it, &a)?;
            let data = match input {
                Value::Bytes(b) => b,
                Value::Str(s) => s.into_bytes(),
                Value::List(_) | Value::Record(_) => {
                    return Err(Flow::Err(
                        ShError::new(code::TYPE, alloc::format!("{} nejde uložit přímo do souboru", input.type_name()))
                            .at(a.span)
                            .hint("převeďte výslovně: | to text > soubor nebo | to json > soubor"),
                    ));
                }
                Value::Nothing => return Err(a.err(code::TYPE, "save nedostal žádná data (použijte rouru: příkaz | save soubor)")),
                other => other.to_text().into_bytes(),
            };
            let data = if a.flag("append") && it.fs_exists(host, &p)? {
                let mut old = it.fs_read(host, &p)?;
                old.extend_from_slice(&data);
                old
            } else {
                data
            };
            it.fs_write(host, &p, data)?;
            Ok(Value::Nothing)
        }
        "rm" => {
            if a.pos.is_empty() {
                return Err(a.err(code::ARGS, "rm: chybí co smazat"));
            }
            let recursive = a.flag("recursive");
            for v in &a.pos {
                for p in it.paths(v)? {
                    let m = it.fs_stat(host, &p)?;
                    if m.kind == NodeKind::Dir && !recursive && !it.fs_list_names(host, &p)?.is_empty() {
                        return Err(Flow::Err(
                            ShError::new(abi::Error::NotEmpty.name(), alloc::format!("{p} je složka s obsahem"))
                                .at(a.span)
                                .hint("i s obsahem: rm -r"),
                        ));
                    }
                    it.fs_remove(host, &p, recursive)?;
                }
            }
            Ok(Value::Nothing)
        }
        "cp" | "mv" => {
            let from = it.paths(a.pos(0).unwrap())?;
            let to = it.paths(a.pos(1).unwrap())?.into_iter().next().unwrap_or_default();
            let to_dir = matches!(it.fs_stat(host, &to), Ok(m) if m.kind == NodeKind::Dir);
            if from.len() > 1 && !to_dir {
                return Err(a.err(abi::Error::NotADirectory.name(), "víc souboru jde jen do složky"));
            }
            for f in from {
                let target = if to_dir { path::join(&to, path::file_name(&f)) } else { to.clone() };
                if path::within(&target, &f) {
                    let what = if name == "cp" { "kopírovat" } else { "přesunout" };
                    return Err(Flow::Err(
                        ShError::new(abi::Error::InvalidArgs.name(), alloc::format!("{f} nejde {what} sám do sebe"))
                            .at(a.span),
                    ));
                }
                if name == "cp" {
                    let m = it.fs_stat(host, &f)?;
                    if m.kind == NodeKind::Dir && !a.flag("recursive") {
                        return Err(Flow::Err(
                            ShError::new(abi::Error::IsADirectory.name(), alloc::format!("{f} je složka"))
                                .at(a.span)
                                .hint("složku kopíruje cp -r"),
                        ));
                    }
                    if it.fs_exists(host, &target)? && m.kind == NodeKind::File {
                        it.fs_remove(host, &target, false)?;
                    }
                    it.fs_copy(host, &f, &target)?;
                } else {
                    it.fs_rename(host, &f, &target)?;
                }
            }
            Ok(Value::Nothing)
        }
        "mkdir" => {
            for v in &a.pos {
                for p in it.paths(v)? {
                    if a.flag("parents") {
                        let mut cur = alloc::string::String::new();
                        for part in p.split('/').filter(|s| !s.is_empty()) {
                            cur.push('/');
                            cur.push_str(part);
                            if !it.fs_exists(host, &cur)? {
                                it.fs_mkdir(host, &cur)?;
                            }
                        }
                    } else {
                        it.fs_mkdir(host, &p)?;
                    }
                }
            }
            Ok(Value::Nothing)
        }
        "exists" => {
            let p = one_path(it, &a)?;
            Ok(Value::Bool(it.fs_exists(host, &p)?))
        }
        "df" => {
            let (total, free) = host.fs().space().map_err(|e| ShError::sys(e, "místo na FS nejde zjistit"))?;
            Ok(Value::record(&[
                ("total", Value::Size(total as i64)),
                ("used", Value::Size(total.saturating_sub(free) as i64)),
                ("free", Value::Size(free as i64)),
            ]))
        }
        _ => Err(a.err(code::UNKNOWN, "neznámý příkaz")),
    }
}

fn one_path(it: &Interp, a: &Args) -> R<alloc::string::String> {
    let v = a.pos(0).ok_or_else(|| a.err(code::ARGS, "chybí cesta"))?;
    let ps = it.paths(v)?;
    if ps.len() != 1 {
        return Err(a.err(code::ARGS, "čekám jednu cestu"));
    }
    Ok(ps.into_iter().next().unwrap())
}

fn ls(it: &mut Interp, host: &mut dyn Host, a: &Args) -> R<Value> {
    let targets = match a.pos(0) {
        Some(v) => it.paths(v)?,
        None => alloc::vec![it.cwd.clone()],
    };
    let mut out = Vec::new();
    for t in targets {
        let m = it.fs_stat(host, &t)?;
        if m.kind == NodeKind::Dir {
            list_dir(it, host, &t, a.flag("all"), a.flag("recursive"), &mut out)?;
        } else {
            out.push(Interp::file_record(&t, path::file_name(&t), &m));
        }
    }
    Ok(Value::List(out))
}

fn list_dir(it: &mut Interp, host: &mut dyn Host, dir: &str, all: bool, rec: bool, out: &mut Vec<Value>) -> R<()> {
    let mut entries = it.fs_list(host, dir)?;
    entries.sort_by(|x, y| x.name.to_lowercase().cmp(&y.name.to_lowercase()));
    for e in entries {
        if !all && e.name.starts_with('.') {
            continue;
        }
        if host.interrupted() {
            return Err(Flow::Err(ShError::new(code::INTERRUPTED, "přerušeno (Ctrl-C)")));
        }
        let full = path::join(dir, &e.name);
        out.push(Interp::file_record(&full, &e.name, &e.meta));
        if rec && e.meta.kind == NodeKind::Dir {
            list_dir(it, host, &full, all, rec, out)?;
        }
    }
    Ok(())
}

/// Nejvic slozek v historii relace.
const DIRS_MAX: usize = 50;

/// `cd -N` (n < 0, o N zpet) nebo `cd +` (n > 0, vpred) v historii slozek.
fn cd_nav(it: &mut Interp, host: &mut dyn Host, a: &Args, n: i64) -> R<Value> {
    let (from, what) = if n < 0 { (&it.dirs_back, "zpět") } else { (&it.dirs_fwd, "vpřed") };
    let k = n.unsigned_abs() as usize;
    if k == 0 || k > from.len() {
        return Err(a.err(code::ARGS, alloc::format!("v historii složek není kam jít {what} ({} složek)", from.len())));
    }
    let target = from[from.len() - k].clone();
    let m = it.fs_stat(host, &target)?;
    if m.kind != NodeKind::Dir {
        return Err(a.err(abi::Error::NotADirectory.name(), alloc::format!("{target} není složka")));
    }
    // Presun: aktualni a preskocene slozky na druhou stranu historie.
    let cur = core::mem::replace(&mut it.cwd, target);
    let (from, to) = if n < 0 { (&mut it.dirs_back, &mut it.dirs_fwd) } else { (&mut it.dirs_fwd, &mut it.dirs_back) };
    to.push(cur);
    for _ in 1..k {
        if let Some(d) = from.pop() {
            to.push(d);
        }
    }
    from.pop();
    Ok(Value::Nothing)
}

/// Do teto velikosti `open` cte soubor hned (bajty), vetsi jako "soubor".
const OPEN_INLINE: u64 = 64 * 1024;
