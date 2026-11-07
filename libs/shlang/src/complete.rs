//! Doplnovani podle typu (Tab v terminalu).
//!
//! Z podpisu prikazu vi, co na dane misto patri: jmeno prikazu, prepinac,
//! cesta (ze souboroveho systemu), promenna za `$`, nebo pole radku
//! (v podmince `where` a u `sort`/`select`/`group`/`sum` - z vystupu
//! predchoziho prikazu roury).

use alloc::string::String;
use alloc::vec::Vec;

use crate::host::{Fs, NodeKind};
use crate::sig::{Io, Kind, Sigs};
use crate::vfs::path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandKind {
    Command,
    Flag,
    File,
    Dir,
    Var,
    Field,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Text, ktery nahradi doplnovane slovo.
    pub text: String,
    pub kind: CandKind,
    pub help: String,
}

pub struct Ctx<'a> {
    pub sigs: &'a dyn Sigs,
    pub vars: Vec<String>,
    pub fns: Vec<String>,
    pub fs: &'a mut dyn Fs,
    pub cwd: String,
    pub home: String,
}

/// Casti prikazu pred kurzorem: predchozi cast roury a slova aktualni.
fn split_stage(prefix: &str) -> (Option<String>, Vec<String>, bool) {
    let mut stages: Vec<Vec<String>> = alloc::vec![Vec::new()];
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut chars = prefix.chars().peekable();
    let flush = |word: &mut String, stages: &mut Vec<Vec<String>>| {
        if !word.is_empty() {
            stages.last_mut().unwrap().push(core::mem::take(word));
        }
    };
    let mut trailing_space = false;
    while let Some(c) = chars.next() {
        trailing_space = false;
        if let Some(q) = quote {
            word.push(c);
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                quote = Some(c);
                word.push(c);
            }
            ' ' | '\t' => {
                flush(&mut word, &mut stages);
                trailing_space = true;
            }
            '|' | ';' | '(' | '{' | '&' | '\n' => {
                flush(&mut word, &mut stages);
                if c == '|' && chars.peek() == Some(&'|') {
                    chars.next();
                    stages.push(Vec::new());
                    stages.push(Vec::new());
                    continue;
                }
                // Novy prikaz (ne dalsi cast roury) po ; ( { && \n
                if c != '|' {
                    stages.push(Vec::new());
                }
                stages.push(Vec::new());
                trailing_space = true;
            }
            _ => word.push(c),
        }
    }
    if !word.is_empty() {
        stages.last_mut().unwrap().push(word);
    } else if trailing_space || stages.last().unwrap().is_empty() {
        stages.last_mut().unwrap().push(String::new());
    }
    let cur = stages.pop().unwrap_or_default();
    let prev = stages.last().and_then(|s| s.first().cloned());
    (prev, cur, trailing_space)
}

/// Doplni slovo pred kurzorem. Vrati pozici zacatku nahrazovaneho slova
/// a kandidaty.
pub fn complete(line: &str, cursor: usize, ctx: &mut Ctx) -> (usize, Vec<Candidate>) {
    let mut cursor = cursor.min(line.len());
    while !line.is_char_boundary(cursor) {
        cursor -= 1;
    }
    let prefix = &line[..cursor];
    let (prev, words, _) = split_stage(prefix);
    let word = words.last().cloned().unwrap_or_default();
    let start = cursor - word.len().min(cursor);
    let mut out = Vec::new();
    let lw = word.to_lowercase();
    // $promenna
    if let Some(v) = word.strip_prefix('$') {
        for name in &ctx.vars {
            if name.starts_with(v) {
                out.push(Candidate { text: alloc::format!("${name}"), kind: CandKind::Var, help: String::new() });
            }
        }
        return (start, out);
    }
    // jmeno prikazu
    if words.len() <= 1 {
        let mut names: Vec<String> = ctx.sigs.names().into_iter().map(String::from).collect();
        names.extend(ctx.fns.iter().cloned());
        names.sort();
        names.dedup();
        for n in names {
            if n.starts_with(word.as_str()) {
                let help = ctx.sigs.get(&n).map(|s| String::from(s.help)).unwrap_or_default();
                out.push(Candidate { text: n, kind: CandKind::Command, help });
            }
        }
        return (start, out);
    }
    let cmd = &words[0];
    let sig = ctx.sigs.get(cmd).copied();
    // prepinace
    if word.starts_with('-') {
        if let Some(s) = &sig {
            for f in s.flags {
                let long = alloc::format!("--{}", f.long);
                if long.starts_with(word.as_str()) {
                    out.push(Candidate { text: long, kind: CandKind::Flag, help: f.help.into() });
                } else if let Some(c) = f.short
                    && word == "-"
                {
                    out.push(Candidate { text: alloc::format!("-{c}"), kind: CandKind::Flag, help: f.help.into() });
                }
            }
        }
        return (start, out);
    }
    let npos = words[1..words.len() - 1].iter().filter(|w| !w.starts_with('-')).count();
    let kind = sig.as_ref().and_then(|s| s.pos_kind(npos));
    let wants_field = kind == Some(Kind::Cond) || matches!(cmd.as_str(), "sort" | "select" | "group" | "sum");
    if wants_field {
        let fields: &[&str] = prev
            .as_deref()
            .and_then(|p| ctx.sigs.get(p))
            .map(|s| match s.output {
                Io::Records(f) => f,
                _ => &[],
            })
            .unwrap_or(&[]);
        for f in fields {
            if f.starts_with(lw.as_str()) {
                out.push(Candidate { text: (*f).into(), kind: CandKind::Field, help: String::new() });
            }
        }
        if kind == Some(Kind::Cond) {
            for v in &ctx.vars {
                if v.starts_with(word.as_str()) && !fields.contains(&v.as_str()) {
                    out.push(Candidate { text: v.clone(), kind: CandKind::Var, help: String::new() });
                }
            }
        }
        return (start, out);
    }
    if matches!(kind, Some(Kind::Int | Kind::Duration | Kind::Closure | Kind::Block)) {
        return (start, out);
    }
    // cesta
    let (dir_part, name_part) = match word.rfind('/') {
        Some(i) => (&word[..=i], &word[i + 1..]),
        None => ("", word.as_str()),
    };
    let dir = if dir_part.is_empty() { ctx.cwd.clone() } else { path::resolve(&ctx.cwd, &ctx.home, dir_part) };
    if let Ok(entries) = ctx.fs.list_names(&dir) {
        let ln = name_part.to_lowercase();
        let mut cands: Vec<Candidate> = entries
            .into_iter()
            .filter(|e| e.name.to_lowercase().starts_with(&ln) && (!e.name.starts_with('.') || name_part.starts_with('.')))
            .map(|e| {
                let is_dir = e.meta.kind == NodeKind::Dir;
                let mut text = alloc::format!("{dir_part}{}", e.name);
                if is_dir {
                    text.push('/');
                }
                Candidate { text, kind: if is_dir { CandKind::Dir } else { CandKind::File }, help: String::new() }
            })
            .collect();
        cands.sort_by(|a, b| a.text.to_lowercase().cmp(&b.text.to_lowercase()));
        out.extend(cands);
    }
    (start, out)
}
