//! Akce k potvrzeni: presny popis, nahled a provedeni u klienta.
//!
//! Zapis mimo pracovni prostor (a spusteni, sit, nastaveni) `agentd`
//! neprovadi - nema na nej capability. Posle klientovi [`Action`]
//! s nahledem; klient se zepta uzivatele a pri "ano" akci provede
//! **svymi pravy** pres [`apply`]. Tak je jedno misto, kde se akce
//! vykonava, pro vsechny klienty (shell, testy, pozdeji GUI).

use alloc::string::String;
use alloc::vec::Vec;

use wire::{Reader, Writer};

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Zapsat cely obsah souboru (vytvori ho, kdyz neni).
    FsWrite { path: String, content: Vec<u8> },
    FsMkdir { path: String },
    /// Presun do kose (nikdy trvale smazani).
    FsTrash { path: String },
    FsRename { from: String, to: String },
    /// Spustit prikaz (shell, prekladac). Klient bez spoustece odmitne.
    Run { command: String },
    /// Jina akce nastroje sluzby (jmeno a vstup JSON) - klient ji preda sluzbe.
    Tool { name: String, input: String },
}

impl Action {
    pub fn kind(&self) -> &'static str {
        match self {
            Action::FsWrite { .. } => "zapis",
            Action::FsMkdir { .. } => "nova slozka",
            Action::FsTrash { .. } => "do kose",
            Action::FsRename { .. } => "prejmenovani",
            Action::Run { .. } => "spusteni",
            Action::Tool { .. } => "nastroj",
        }
    }

    pub fn put(&self, w: &mut Writer) {
        match self {
            Action::FsWrite { path, content } => {
                w.u8(1).str(path).u32(content.len() as u32).bytes(content);
            }
            Action::FsMkdir { path } => {
                w.u8(2).str(path);
            }
            Action::FsTrash { path } => {
                w.u8(3).str(path);
            }
            Action::FsRename { from, to } => {
                w.u8(4).str(from).str(to);
            }
            Action::Run { command } => {
                w.u8(5).str(command);
            }
            Action::Tool { name, input } => {
                w.u8(6).str(name).str(input);
            }
        }
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        Ok(match r.u8()? {
            1 => {
                let path = r.str()?;
                let n = r.u32()? as usize;
                Action::FsWrite { path, content: r.take(n)?.to_vec() }
            }
            2 => Action::FsMkdir { path: r.str()? },
            3 => Action::FsTrash { path: r.str()? },
            4 => Action::FsRename { from: r.str()?, to: r.str()? },
            5 => Action::Run { command: r.str()? },
            6 => Action::Tool { name: r.str()?, input: r.str()? },
            _ => return Err(Error::InvalidArgs),
        })
    }

    /// Kratky popis (audit, titulek dotazu).
    pub fn summary(&self) -> String {
        match self {
            Action::FsWrite { path, content } => alloc::format!("zapis {path} ({} B)", content.len()),
            Action::FsMkdir { path } => alloc::format!("nova slozka {path}"),
            Action::FsTrash { path } => alloc::format!("do kose {path}"),
            Action::FsRename { from, to } => alloc::format!("prejmenovat {from} -> {to}"),
            Action::Run { command } => alloc::format!("spustit: {command}"),
            Action::Tool { name, .. } => alloc::format!("nastroj {name}"),
        }
    }
}

/// Nejvic radku nahledu (zbytek se zkrati).
pub const PREVIEW_LINES: usize = 200;

/// Nahled zapisu: rozdil stareho a noveho obsahu po radcich (`-`, `+`,
/// ` ` kontext). `old = None` = novy soubor. Binarni obsah = jen velikosti.
pub fn preview_write(path: &str, old: Option<&[u8]>, new: &[u8]) -> String {
    let mut out = alloc::format!("--- {path}\n");
    let (Ok(new_t), Ok(old_t)) = (core::str::from_utf8(new), core::str::from_utf8(old.unwrap_or(b""))) else {
        out.push_str(&alloc::format!("binarni obsah: {} B -> {} B\n", old.map_or(0, <[u8]>::len), new.len()));
        return out;
    };
    if old.is_none() {
        out.push_str("(novy soubor)\n");
    }
    out.push_str(&diff_lines(old_t, new_t));
    out
}

/// Rozdil radku (LCS); pro velke soubory jen souhrn.
pub fn diff_lines(old: &str, new: &str) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let mut out = String::new();
    if a.len().saturating_mul(b.len()) > 2_000_000 {
        out.push_str(&alloc::format!("(velky soubor: {} radku -> {} radku)\n", a.len(), b.len()));
        return out;
    }
    // Tabulka delek LCS od konce.
    let w = b.len() + 1;
    let mut t = alloc::vec![0u32; (a.len() + 1) * w];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            t[i * w + j] = if a[i] == b[j] { t[(i + 1) * w + j + 1] + 1 } else { t[(i + 1) * w + j].max(t[i * w + j + 1]) };
        }
    }
    let (mut i, mut j, mut lines) = (0, 0, 0);
    let mut push = |out: &mut String, sign: char, s: &str| {
        lines += 1;
        if lines <= PREVIEW_LINES {
            out.push(sign);
            out.push_str(s);
            out.push('\n');
        }
    };
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            push(&mut out, ' ', a[i]);
            i += 1;
            j += 1;
        } else if j < b.len() && (i == a.len() || t[i * w + j + 1] > t[(i + 1) * w + j]) {
            push(&mut out, '+', b[j]);
            j += 1;
        } else {
            push(&mut out, '-', a[i]);
            i += 1;
        }
    }
    if lines > PREVIEW_LINES {
        out.push_str(&alloc::format!("... (dalsich {} radku)\n", lines - PREVIEW_LINES));
    }
    out
}

/// Souborovy system klienta, nad kterym se potvrzena akce provede.
pub trait ActionFs {
    fn read(&mut self, path: &str) -> Result<Vec<u8>>;
    fn write(&mut self, path: &str, data: &[u8]) -> Result<()>;
    fn mkdir(&mut self, path: &str) -> Result<()>;
    fn trash(&mut self, path: &str) -> Result<()>;
    fn rename(&mut self, from: &str, to: &str) -> Result<()>;
}

/// Provede potvrzenou akci; vraci text vysledku pro model.
pub fn apply(fs: &mut dyn ActionFs, a: &Action) -> Result<String> {
    match a {
        Action::FsWrite { path, content } => {
            fs.write(path, content)?;
            Ok(alloc::format!("zapsano {} B do {path}", content.len()))
        }
        Action::FsMkdir { path } => fs.mkdir(path).map(|_| alloc::format!("vytvorena slozka {path}")),
        Action::FsTrash { path } => fs.trash(path).map(|_| alloc::format!("{path} presunuto do kose")),
        Action::FsRename { from, to } => fs.rename(from, to).map(|_| alloc::format!("{from} -> {to}")),
        Action::Run { .. } | Action::Tool { .. } => Err(Error::NotSupported),
    }
}

/// Jak klient na potvrzeni odpovedel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Uzivatel souhlasil a akce probehla (text vysledku).
    Done(String),
    /// Uzivatel odmitl (nebo klient nema UI).
    Declined,
    /// Uzivatel souhlasil, ale akce selhala.
    Failed(abi::Error, String),
}

impl Outcome {
    pub fn put(&self, w: &mut Writer) {
        match self {
            Outcome::Done(t) => {
                w.u8(1).str(t);
            }
            Outcome::Declined => {
                w.u8(2);
            }
            Outcome::Failed(e, t) => {
                w.u8(3).i32(e.code()).str(t);
            }
        }
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        Ok(match r.u8()? {
            1 => Outcome::Done(r.str()?),
            2 => Outcome::Declined,
            3 => {
                let e = abi::Error::from_code(i64::from(r.i32()?)).unwrap_or(abi::Error::IoError);
                Outcome::Failed(e, r.str()?)
            }
            _ => return Err(Error::InvalidArgs),
        })
    }
}
