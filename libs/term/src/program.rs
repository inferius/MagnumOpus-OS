//! Program spusteny ze shellu (sluzba "exec" initu, `launchproto::exec`).
//!
//! Relace shellu program nespousti sama: prostredi (`Env::exec_start`)
//! posle pozadavek initu a vrati bezici program ([`Running`]). Vlakno
//! radku pak:
//!
//! - vystup (stdout a stderr) vypisuje po radcich do bloku, jak prichazi,
//! - vstup: data z roury (`echo x | prog`), jinak radky, ktere uzivatel
//!   napise do textove vyzvy behem behu (interaktivni stdin, napr. `nc`),
//! - Ctrl+C = ukonceni procesu,
//! - nenulovy navratovy kod = chyba `SH_EXIT`.
//!
//! Typovany kanal (program vraci hodnoty shellu misto bajtu) je
//! [navrzeno].

use alloc::string::String;
use alloc::vec::Vec;

use shlang::host::ExecNeeds;
use shlang::value::Value;

use crate::perm::{MountSpec, mounts};

/// Co poslat initu.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    /// Prazdne = cely FS s pravy uzivatele.
    pub mounts: Vec<MountSpec>,
    pub net: bool,
}

impl Request {
    /// Pozadavek z volani shellu: pojmenovane argumenty jako `--jmeno`
    /// (pravdivostni `true` bez hodnoty), cesty z naroku programu.
    pub fn new(program: &str, args: &[(Option<String>, Value)], needs: &ExecNeeds) -> Self {
        let mut out = Vec::new();
        for (name, v) in args {
            match (name, v) {
                (None, v) => out.push(v.to_text()),
                (Some(n), Value::Bool(true)) => out.push(alloc::format!("--{n}")),
                (Some(_), Value::Bool(false)) => {}
                (Some(n), v) => {
                    out.push(alloc::format!("--{n}"));
                    out.push(v.to_text());
                }
            }
        }
        let m = mounts(needs);
        // Plna prava bez klonu = cely FS uzivatele (bez zuzeni).
        let whole = m.len() == 1 && m[0].name == "/" && m[0].target == "/" && !m[0].read_only;
        Self {
            program: String::from(program),
            args: out,
            cwd: needs.cwd.clone(),
            mounts: if whole { Vec::new() } else { m },
            net: needs.net || needs.all,
        }
    }
}

/// Kus vystupu programu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub stderr: bool,
    pub data: Vec<u8>,
}

/// Bezici program (vlastni ho vlakno radku).
pub trait Running: Send {
    /// Dosly vystup (bez cekani).
    fn output(&mut self) -> Vec<Output>;
    /// Zapis do stdin; `false` = program stdin zavrel.
    fn write_stdin(&mut self, data: &[u8]) -> bool;
    fn close_stdin(&mut self);
    /// Navratovy kod, jakmile proces skoncil a vystup je precteny.
    fn exit_code(&mut self) -> Option<i64>;
    /// Ukonci proces (Ctrl+C).
    fn kill(&mut self);
    /// Obslouzi ridici kanal terminalu programu (`ttyproto`: rezim,
    /// velikost).
    fn poll_control(&mut self) {}
    /// Ma program zapnute ECHO? `false` = skryty vstup (heslo).
    fn echo(&self) -> bool {
        true
    }
    /// Systemova vyzva sluzby uctu, ktera ceka na zobrazeni.
    fn system_prompt(&mut self) -> Option<SystemPrompt> {
        None
    }
    /// Odpoved na systemovou vyzvu (jde primo sluzbe uctu, ne programu).
    fn answer_system(&mut self, _id: u32, _ok: bool, _text: String) {}
}

/// Stitek systemove vyzvy sluzby uctu (odznak v okne).
pub const SYSTEM_LABEL: &str = "Účty";

/// Systemova vyzva (sluzba uctu): heslo k odemceni klicu SSH nebo
/// potvrzeni klice pro novy server. Program ji nevidi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemPrompt {
    pub id: u32,
    pub question: String,
    /// Skryty text (heslo); `false` = ano / ne.
    pub secret: bool,
}

/// Rozdeli vystup na cele radky; zbytek zustane v `buf`.
pub fn lines(buf: &mut Vec<u8>, data: &[u8]) -> Vec<String> {
    buf.extend_from_slice(data);
    let mut out = Vec::new();
    while let Some(i) = buf.iter().position(|&b| b == b'\n') {
        let line: Vec<u8> = buf.drain(..=i).collect();
        let line = &line[..line.len() - 1];
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        out.push(String::from_utf8_lossy(line).into_owned());
    }
    out
}
