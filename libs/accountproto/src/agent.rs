//! Klice klienta SSH a podpis za klienta (`docs/cs/ssh.md`, Klient).
//!
//! - Kanal podpisu (program `ssh`, `scp`, `sftp` -> accountd): vydava ho
//!   init jen systemovemu programu z initfs (id aplikace z manifestu) a jen
//!   pro jeho relaci. Soukromy klic program nikdy nema: dostane verejne
//!   klice ([`AgentReq::Identities`]) a podpis ([`AgentReq::Sign`]).
//! - Systemova vyzva (accountd -> shell relace): odemceni klicu heslem
//!   uctu a potvrzeni klice pro novy server. Zobrazuje ji systemova cast
//!   shellu se stitkem "Účty", odpoved jde primo accountd; program ssh heslo
//!   uctu nikdy nevidi.
//!
//! Zpravy kanalu podpisu: `op u32` a parametry; odpoved jako ostatni
//! kanaly accountd (`reply_ok` / `reply_err`).

use alloc::string::String;
use alloc::vec::Vec;

use wire::{Reader, Writer};

use crate::{Error, Result, Secret, get_blob, get_secret, put_blob, put_secret};

pub mod op {
    pub const IDENTITIES: u32 = 1;
    pub const SIGN: u32 = 2;
}

/// Nejvic identit ve vypisu.
const MAX_IDENTITIES: usize = 64;
/// Nejdelsi text vyzvy.
pub const MAX_QUESTION: usize = 1024;

/// Pozadavek programu ssh na kanalu podpisu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentReq {
    /// Verejne klice klienta uctu.
    Identities,
    /// Podpis dat prihlaseni (RFC 4252) klicem `id` pro server `host`
    /// (`host` nebo `[host]:port`; potvrzeni a audit).
    Sign { id: u32, host: String, data: Vec<u8> },
}

impl AgentReq {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            AgentReq::Identities => Writer::op(op::IDENTITIES).finish(),
            AgentReq::Sign { id, host, data } => {
                let mut w = Writer::op(op::SIGN);
                w.u32(*id).str(host);
                put_blob(&mut w, data);
                w.finish()
            }
        }
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let m = match r.u32()? {
            op::IDENTITIES => AgentReq::Identities,
            op::SIGN => AgentReq::Sign { id: r.u32()?, host: r.str()?, data: get_blob(&mut r)? },
            _ => return Err(Error::NotSupported),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(m)
    }
}

/// Identita klienta (verejny klic).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub id: u32,
    /// Blob protokolu SSH (`ssh::keys::PublicKey::blob`).
    pub blob: Vec<u8>,
    pub comment: String,
}

pub fn encode_identities(list: &[Identity]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(list.len() as u32);
    for i in list {
        w.u32(i.id);
        put_blob(&mut w, &i.blob);
        w.str(&i.comment);
    }
    w.finish()
}

pub fn decode_identities(b: &[u8]) -> Result<Vec<Identity>> {
    let mut r = Reader::new(b);
    let n = r.u32()? as usize;
    if n > MAX_IDENTITIES {
        return Err(Error::InvalidArgs);
    }
    let v = (0..n).map(|_| Ok(Identity { id: r.u32()?, blob: get_blob(&mut r)?, comment: r.str()? })).collect::<Result<Vec<_>>>()?;
    if !r.is_empty() {
        return Err(Error::InvalidArgs);
    }
    Ok(v)
}

/// Odpoved na vytvoreni nebo import klice: cislo, radek verejneho klice
/// (`ssh-ed25519 AAAA... komentar`) a otisk.
pub fn encode_created(id: u32, line: &str, fingerprint: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(id).str(line).str(fingerprint);
    w.finish()
}

pub fn decode_created(b: &[u8]) -> Result<(u32, String, String)> {
    let mut r = Reader::new(b);
    let v = (r.u32()?, r.str()?, r.str()?);
    if !r.is_empty() {
        return Err(Error::InvalidArgs);
    }
    Ok(v)
}

/// Systemova vyzva (accountd -> shell relace).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prompt {
    /// Skryty text (heslo uctu k odemceni klicu).
    Secret { id: u32, question: String },
    /// Ano / ne (pouzit klic pro novy server).
    Confirm { id: u32, question: String },
}

impl Prompt {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Prompt::Secret { id, question } => {
                w.u8(1).u32(*id).str(question);
            }
            Prompt::Confirm { id, question } => {
                w.u8(2).u32(*id).str(question);
            }
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let kind = r.u8()?;
        let (id, question) = (r.u32()?, r.str()?);
        if question.len() > MAX_QUESTION || !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        match kind {
            1 => Ok(Prompt::Secret { id, question }),
            2 => Ok(Prompt::Confirm { id, question }),
            _ => Err(Error::InvalidArgs),
        }
    }

    pub fn id(&self) -> u32 {
        match self {
            Prompt::Secret { id, .. } | Prompt::Confirm { id, .. } => *id,
        }
    }
}

/// Odpoved shellu na vyzvu: `ok = false` = odmitnuto nebo zruseno;
/// `text` jen u [`Prompt::Secret`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptAnswer {
    pub id: u32,
    pub ok: bool,
    pub text: Secret,
}

impl PromptAnswer {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(self.id).u8(self.ok as u8);
        put_secret(&mut w, &self.text);
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let a = PromptAnswer { id: r.u32()?, ok: r.u8()? != 0, text: get_secret(&mut r)? };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(a)
    }
}
