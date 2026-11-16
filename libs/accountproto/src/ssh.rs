//! Vzdaleny pristup (SSH): vypis klicu uctu a odpoved overeni klicem
//! (`docs/cs/ssh.md`). Kanal `ssh` dostava jen sluzba sshd; samotny
//! podpis overuje accountd, sshd jen preda data.

use alloc::string::String;
use alloc::vec::Vec;

use wire::{Reader, Writer};

use crate::{Error, Result, get_blob, put_blob};

/// Delka listku vzdalene relace (nahodny, jednorazovy).
pub const TICKET: usize = 32;
/// Nejvic klicu ve vypisu.
const MAX_KEYS: usize = 64;

/// Klic SSH ve vypisu uctu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshKey {
    pub id: u32,
    pub algo: String,
    pub fingerprint: String,
    pub comment: String,
    /// Kdy pridan (sekundy od 1970).
    pub added: i64,
}

pub fn encode_keys(list: &[SshKey]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(list.len() as u32);
    for k in list {
        w.u32(k.id).str(&k.algo).str(&k.fingerprint).str(&k.comment).u64(k.added as u64);
    }
    w.finish()
}

pub fn decode_keys(b: &[u8]) -> Result<Vec<SshKey>> {
    let mut r = Reader::new(b);
    let n = r.u32()? as usize;
    if n > MAX_KEYS {
        return Err(Error::InvalidArgs);
    }
    let v = (0..n)
        .map(|_| Ok(SshKey { id: r.u32()?, algo: r.str()?, fingerprint: r.str()?, comment: r.str()?, added: r.u64()? as i64 }))
        .collect::<Result<Vec<_>>>()?;
    if !r.is_empty() {
        return Err(Error::InvalidArgs);
    }
    Ok(v)
}

/// Odpoved `SSH_AUTH`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshAuthReply {
    /// Prihlaseno: relace u initu a listek, kterym si ji sshd vyzvedne.
    Ok { uid: u64, session: u64, ticket: Vec<u8> },
    /// Odmitnuto (duvod se neprozradi); dalsi pokus za `retry_after_ms`.
    Denied { retry_after_ms: u64 },
    /// Klic sedi, ale domov je sifrovany a zamceny (klicem SSH nejde
    /// odemknout): nejdriv prihlaseni u pocitace.
    Locked,
}

impl SshAuthReply {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            SshAuthReply::Ok { uid, session, ticket } => {
                w.u8(0).u64(*uid).u64(*session);
                put_blob(&mut w, ticket);
            }
            SshAuthReply::Denied { retry_after_ms } => {
                w.u8(1).u64(*retry_after_ms);
            }
            SshAuthReply::Locked => {
                w.u8(2);
            }
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let v = match r.u8()? {
            0 => SshAuthReply::Ok { uid: r.u64()?, session: r.u64()?, ticket: get_blob(&mut r)? },
            1 => SshAuthReply::Denied { retry_after_ms: r.u64()? },
            2 => SshAuthReply::Locked,
            _ => return Err(Error::InvalidArgs),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(v)
    }
}
