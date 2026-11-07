//! Pristup k souborum z prikazu: cesty, kontrola manifestu `needs`,
//! zapis pres koncept radku (klony FS bokem, nanecisto do konce radku).

use alloc::string::String;
use alloc::vec::Vec;

use super::{Flow, Interp, R};
use crate::error::{ShError, code};
use crate::host::{DirEntry, Host, Meta, NodeKind};
use crate::span::Span;
use crate::value::Value;
use crate::vfs::path::{self, resolve, within};

fn sys(e: abi::Error, what: String) -> Flow {
    Flow::Err(ShError::sys(e, what))
}

impl Interp {
    /// Absolutni cesta.
    pub(crate) fn abs(&self, p: &str) -> String {
        resolve(&self.cwd, &self.home, p)
    }

    fn check_read(&self, p: &str) -> R<()> {
        if let Some(n) = &self.needs
            && !n.read.iter().chain(&n.write).any(|r| within(p, r))
        {
            return Err(Flow::Err(
                ShError::new(code::NEEDS, alloc::format!("skript nema pravo cist {p}"))
                    .hint("pridejte cestu do manifestu: needs { read: [\"...\"] }"),
            ));
        }
        Ok(())
    }

    fn check_write(&self, p: &str) -> R<()> {
        if let Some(n) = &self.needs
            && !n.write.iter().any(|r| within(p, r))
        {
            return Err(Flow::Err(
                ShError::new(code::NEEDS, alloc::format!("skript nema pravo zapisovat do {p}"))
                    .hint("pridejte cestu do manifestu: needs { write: [\"...\"] }"),
            ));
        }
        Ok(())
    }

    pub(crate) fn fs_stat(&mut self, host: &mut dyn Host, p: &str) -> R<Meta> {
        self.check_read(p)?;
        self.draft.stat(host.fs(), p).map_err(|e| sys(e, alloc::format!("{p} nejde otevrit")))
    }

    pub(crate) fn fs_exists(&mut self, host: &mut dyn Host, p: &str) -> R<bool> {
        self.check_read(p)?;
        Ok(self.draft.stat(host.fs(), p).is_ok())
    }

    pub(crate) fn fs_list(&mut self, host: &mut dyn Host, p: &str) -> R<Vec<DirEntry>> {
        self.check_read(p)?;
        self.draft.list(host.fs(), p).map_err(|e| sys(e, alloc::format!("slozku {p} nejde vypsat")))
    }

    /// Jen jmena a druhy (levnejsi nez `fs_list`).
    pub(crate) fn fs_list_names(&mut self, host: &mut dyn Host, p: &str) -> R<Vec<DirEntry>> {
        self.check_read(p)?;
        self.draft.list_names(host.fs(), p).map_err(|e| sys(e, alloc::format!("slozku {p} nejde vypsat")))
    }

    pub(crate) fn fs_read(&mut self, host: &mut dyn Host, p: &str) -> R<Vec<u8>> {
        self.check_read(p)?;
        self.draft.read(host.fs(), p).map_err(|e| sys(e, alloc::format!("{p} nejde precist")))
    }

    pub(crate) fn fs_write(&mut self, host: &mut dyn Host, p: &str, data: Vec<u8>) -> R<()> {
        self.check_write(p)?;
        let r = self.draft.write(host.fs(), p, &data);
        self.write_err(r, alloc::format!("do {p} nejde zapsat"))
    }

    /// Chyba zapisu; kdyz za ni muze nepovedeny klon bokem, s radou.
    fn write_err<T>(&mut self, r: crate::host::FsResult<T>, what: String) -> R<T> {
        let failed = core::mem::take(&mut self.draft.aside_failed);
        r.map_err(|e| match sys(e, what) {
            Flow::Err(err) if failed => Flow::Err(
                err.hint("zmeny nanecisto tu nejdou (FS nezalozil klon bokem, napr. chybi zapisovatelny /tmp)"),
            ),
            f => f,
        })
    }

    pub(crate) fn fs_mkdir(&mut self, host: &mut dyn Host, p: &str) -> R<()> {
        self.check_write(p)?;
        let r = self.draft.mkdir(host.fs(), p);
        self.write_err(r, alloc::format!("slozku {p} nejde vytvorit"))
    }

    pub(crate) fn fs_remove(&mut self, host: &mut dyn Host, p: &str, recursive: bool) -> R<()> {
        self.check_write(p)?;
        let fs = host.fs();
        let r = if recursive { self.draft.remove_all(fs, p) } else { self.draft.remove(fs, p) };
        r.map_err(|e| {
            let f = sys(e, alloc::format!("{p} nejde smazat"));
            match (e, f) {
                (abi::Error::NotEmpty, Flow::Err(err)) => Flow::Err(err.hint("slozku i s obsahem smaze rm -r")),
                (_, f) => f,
            }
        })
    }

    pub(crate) fn fs_copy(&mut self, host: &mut dyn Host, from: &str, to: &str) -> R<()> {
        self.check_read(from)?;
        self.check_write(to)?;
        let r = self.draft.copy(host.fs(), from, to);
        self.write_err(r, alloc::format!("{from} nejde zkopirovat do {to}"))
    }

    pub(crate) fn fs_rename(&mut self, host: &mut dyn Host, from: &str, to: &str) -> R<()> {
        self.check_write(from)?;
        self.check_write(to)?;
        let r = self.draft.rename(host.fs(), from, to);
        self.write_err(r, alloc::format!("{from} nejde presunout do {to}"))
    }

    /// Zaznam souboru (radek vystupu `ls`).
    pub(crate) fn file_record(full: &str, name: &str, m: &Meta) -> Value {
        Value::record(&[
            ("name", Value::str(name)),
            ("path", Value::str(full)),
            ("kind", Value::str(if m.dataset { "dataset" } else { m.kind.name() })),
            ("size", Value::Size(m.size as i64)),
            ("mtime", m.mtime.map(Value::Time).unwrap_or(Value::Null)),
            ("ext", Value::str(if m.kind == NodeKind::File { path::ext(name) } else { "" })),
            // Prava relace (`rwx`); prazdne = FS je nehlasi.
            ("access", Value::str(m.access.map(crate::host::access::text).unwrap_or_default())),
        ])
    }

    /// Rozvine vzor souboru (`*.txt`, `src/*.rs`) na seznam cest. Vzor jen
    /// v posledni casti cesty; nic nenalezeno = chyba (ne tichy text).
    pub(crate) fn glob(&mut self, host: &mut dyn Host, pat: &str, span: Span) -> R<Value> {
        let (dir, name_pat) = match pat.rfind('/') {
            Some(i) => (&pat[..i.max(1)], &pat[i + 1..]),
            None => (".", pat),
        };
        if dir.contains(['*', '?', '[']) {
            return Err(Flow::Err(
                ShError::new(code::ARGS, "vzor smi byt jen v posledni casti cesty").at(span).hint("napr. src/*.rs"),
            ));
        }
        let abs_dir = self.abs(dir);
        let mut out = Vec::new();
        let show_hidden = name_pat.starts_with('.');
        for e in self.fs_list_names(host, &abs_dir)? {
            if (!show_hidden && e.name.starts_with('.')) || !path::glob_match(name_pat, &e.name) {
                continue;
            }
            let shown = if dir == "." { e.name.clone() } else { path::join(dir, &e.name) };
            out.push(Value::Str(shown));
        }
        if out.is_empty() {
            return Err(Flow::Err(
                ShError::new(code::NO_MATCH, alloc::format!("vzor '{pat}' nenasel zadny soubor")).at(span),
            ));
        }
        out.sort_by(|a, b| a.to_text().to_lowercase().cmp(&b.to_text().to_lowercase()));
        Ok(Value::List(out))
    }

    /// Hodnota argumentu jako seznam cest (text nebo seznam textu).
    pub(crate) fn paths(&self, v: &Value) -> R<Vec<String>> {
        match v {
            Value::Str(s) => Ok(alloc::vec![self.abs(s)]),
            Value::List(items) => {
                let mut out = Vec::new();
                for x in items {
                    out.extend(self.paths(x)?);
                }
                Ok(out)
            }
            Value::Record(_) => match v.field("path") {
                Some(Value::Str(p)) => Ok(alloc::vec![self.abs(&p)]),
                _ => Err(Flow::Err(ShError::new(code::TYPE, "zaznam nema pole path"))),
            },
            other => Err(Flow::Err(ShError::new(
                code::TYPE,
                alloc::format!("cekam cestu, dostal jsem {}", other.type_name()),
            ))),
        }
    }
}
