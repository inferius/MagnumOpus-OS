//! Verejne klice SSH uzivatelu: rozbor (radek `authorized_keys`, blob
//! protokolu), otisk SHA256 a overeni podpisu prihlaseni.
//!
//! Podporovane typy (v1): `ssh-ed25519` a `sk-ssh-ed25519@openssh.com`
//! (klic FIDO u klienta, OpenSSH `PROTOCOL.u2f`). RSA a ECDSA se odmitnou
//! s `NOT_SUPPORTED` (`docs/cs/ssh.md`). Podpis overuje sluzba uctu, ne
//! proces spojeni: ten cte data ze site a nesmi umet prihlaseni zfalsovat.

use alloc::string::String;
use alloc::vec::Vec;

use abi::{Error, Result};
use base64ct::{Base64, Base64Unpadded, Encoding};
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

/// Jmeno typu `ssh-ed25519`.
pub const ED25519: &str = "ssh-ed25519";
/// Jmeno typu klice FIDO (`sk-ssh-ed25519@openssh.com`).
pub const SK_ED25519: &str = "sk-ssh-ed25519@openssh.com";
/// Priznak podpisu klice FIDO: uzivatel byl pritomen (dotek).
pub const SK_USER_PRESENT: u8 = 0x01;
/// Priznak podpisu klice FIDO: uzivatel overen (PIN, biometrie).
pub const SK_USER_VERIFIED: u8 = 0x04;
/// Nejdelsi blob klice (vic nikdo legitimni neposle).
const MAX_BLOB: usize = 1024;

/// Typ klice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Algo {
    Ed25519,
    /// Klic FIDO: `application` je retezec aplikace (obvykle `ssh:`).
    SkEd25519 { application: String },
}

/// Verejny klic uzivatele.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKey {
    pub algo: Algo,
    pub key: [u8; 32],
    /// Komentar z radku `authorized_keys` (napr. `alice@pc`).
    pub comment: String,
}

/// Cteni retezcu protokolu SSH (`u32` delka big-endian + bajty).
struct Wire<'a>(&'a [u8]);

impl<'a> Wire<'a> {
    fn bytes(&mut self) -> Result<&'a [u8]> {
        let (len, rest) = self.0.split_first_chunk::<4>().ok_or(Error::InvalidArgs)?;
        let len = u32::from_be_bytes(*len) as usize;
        if len > rest.len() {
            return Err(Error::InvalidArgs);
        }
        let (v, rest) = rest.split_at(len);
        self.0 = rest;
        Ok(v)
    }

    fn str(&mut self) -> Result<&'a str> {
        core::str::from_utf8(self.bytes()?).map_err(|_| Error::InvalidArgs)
    }

    fn u8(&mut self) -> Result<u8> {
        let (&b, rest) = self.0.split_first().ok_or(Error::InvalidArgs)?;
        self.0 = rest;
        Ok(b)
    }

    fn u32(&mut self) -> Result<u32> {
        let (v, rest) = self.0.split_first_chunk::<4>().ok_or(Error::InvalidArgs)?;
        self.0 = rest;
        Ok(u32::from_be_bytes(*v))
    }

    fn end(&self) -> Result<()> {
        if self.0.is_empty() { Ok(()) } else { Err(Error::InvalidArgs) }
    }
}

fn put_str(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(&(s.len() as u32).to_be_bytes());
    out.extend_from_slice(s);
}

fn key32(b: &[u8]) -> Result<[u8; 32]> {
    b.try_into().map_err(|_| Error::InvalidArgs)
}

impl PublicKey {
    /// Klic z blobu protokolu (`string typ, ...`), jak ho posle klient.
    pub fn from_blob(blob: &[u8]) -> Result<Self> {
        if blob.len() > MAX_BLOB {
            return Err(Error::InvalidArgs);
        }
        let mut r = Wire(blob);
        let algo = r.str()?;
        let k = match algo {
            ED25519 => PublicKey { algo: Algo::Ed25519, key: key32(r.bytes()?)?, comment: String::new() },
            SK_ED25519 => {
                let key = key32(r.bytes()?)?;
                let application = String::from(r.str()?);
                PublicKey { algo: Algo::SkEd25519 { application }, key, comment: String::new() }
            }
            "ssh-rsa" | "rsa-sha2-256" | "rsa-sha2-512" | "ecdsa-sha2-nistp256" | "ecdsa-sha2-nistp384"
            | "ecdsa-sha2-nistp521" | "sk-ecdsa-sha2-nistp256@openssh.com" => return Err(Error::NotSupported),
            _ => return Err(Error::InvalidArgs),
        };
        r.end()?;
        // Klic musi byt platny bod krivky (jinak overeni nema smysl).
        VerifyingKey::from_bytes(&k.key).map_err(|_| Error::InvalidArgs)?;
        Ok(k)
    }

    /// Klic z radku `authorized_keys` / `*.pub`: `typ base64 [komentar]`.
    /// Volby pred typem (`from=...`, `command=...`) se nepodporuji
    /// (`INVALID_ARGS`), aby nevznikl dojem, ze plati.
    pub fn parse_line(line: &str) -> Result<Self> {
        let line = line.trim_end_matches(['\r', '\n']).trim();
        let mut it = line.splitn(3, char::is_whitespace);
        let (Some(algo), Some(b64)) = (it.next(), it.next()) else { return Err(Error::InvalidArgs) };
        let blob = Base64::decode_vec(b64).map_err(|_| Error::InvalidArgs)?;
        let mut k = Self::from_blob(&blob)?;
        if k.algo_name() != algo {
            return Err(Error::InvalidArgs);
        }
        k.comment = String::from(it.next().unwrap_or("").trim());
        Ok(k)
    }

    pub fn algo_name(&self) -> &'static str {
        match self.algo {
            Algo::Ed25519 => ED25519,
            Algo::SkEd25519 { .. } => SK_ED25519,
        }
    }

    /// Blob protokolu (to, co posle klient a co se uklada).
    pub fn blob(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_str(&mut out, self.algo_name().as_bytes());
        put_str(&mut out, &self.key);
        if let Algo::SkEd25519 { application } = &self.algo {
            put_str(&mut out, application.as_bytes());
        }
        out
    }

    /// Radek ve tvaru `authorized_keys`.
    pub fn line(&self) -> String {
        let mut s = alloc::format!("{} {}", self.algo_name(), Base64::encode_string(&self.blob()));
        if !self.comment.is_empty() {
            s.push(' ');
            s.push_str(&self.comment);
        }
        s
    }

    /// Otisk jako v OpenSSH: `SHA256:` + base64 bez doplneni.
    pub fn fingerprint(&self) -> String {
        fingerprint(&self.blob())
    }

    /// Overi podpis `sig` (blob protokolu: `string typ, string podpis`,
    /// u FIDO navic `byte priznaky, uint32 citac`) nad daty `data`.
    /// Klic FIDO musi mit priznak pritomnosti uzivatele; `need_uv` navic
    /// vyzaduje overeni uzivatele (PIN klice).
    pub fn verify(&self, data: &[u8], sig: &[u8], need_uv: bool) -> Result<()> {
        let mut r = Wire(sig);
        let algo = r.str()?;
        if algo != self.algo_name() {
            return Err(Error::AccessDenied);
        }
        let raw: [u8; 64] = r.bytes()?.try_into().map_err(|_| Error::InvalidArgs)?;
        let vk = VerifyingKey::from_bytes(&self.key).map_err(|_| Error::InvalidArgs)?;
        let s = Signature::from_bytes(&raw);
        let ok = match &self.algo {
            Algo::Ed25519 => {
                r.end()?;
                vk.verify_strict(data, &s).is_ok()
            }
            Algo::SkEd25519 { application } => {
                let flags = r.u8()?;
                let counter = r.u32()?;
                r.end()?;
                if flags & SK_USER_PRESENT == 0 || (need_uv && flags & SK_USER_VERIFIED == 0) {
                    return Err(Error::AccessDenied);
                }
                // PROTOCOL.u2f: podepisuje se sha256(aplikace) || priznaky
                // || citac || sha256(data).
                let mut m = Vec::with_capacity(32 + 1 + 4 + 32);
                m.extend_from_slice(&Sha256::digest(application.as_bytes()));
                m.push(flags);
                m.extend_from_slice(&counter.to_be_bytes());
                m.extend_from_slice(&Sha256::digest(data));
                vk.verify_strict(&m, &s).is_ok()
            }
        };
        if ok { Ok(()) } else { Err(Error::AccessDenied) }
    }
}

/// Verejny klic ed25519 ze soukromeho (32 B semeno; klic hostitele).
pub fn ed25519_public(seed: &[u8; 32]) -> PublicKey {
    let pk = ed25519_dalek::SigningKey::from_bytes(seed).verifying_key().to_bytes();
    PublicKey { algo: Algo::Ed25519, key: pk, comment: String::new() }
}

/// Otisk blobu klice (`SHA256:...`).
pub fn fingerprint(blob: &[u8]) -> String {
    alloc::format!("SHA256:{}", Base64Unpadded::encode_string(&Sha256::digest(blob)))
}

/// Podpis ed25519 seedem (sluzba uctu za klienta SSH; seed proces klienta
/// nikdy nema).
pub fn ed25519_sign(seed: &[u8; 32], data: &[u8]) -> [u8; 64] {
    use ed25519_dalek::Signer;
    ed25519_dalek::SigningKey::from_bytes(seed).sign(data).to_bytes()
}

/// Nejvetsi soubor soukromeho klice, ktery se cte.
pub const MAX_PRIVATE_TEXT: usize = 16 * 1024;

/// Soukromy klic OpenSSH (`-----BEGIN OPENSSH PRIVATE KEY-----`, format
/// `openssh-key-v1`): seed ed25519 a verejny klic s komentarem. Jen
/// nesifrovany klic ed25519; sifrovany (heslo k souboru) = `NOT_SUPPORTED`
/// (bcrypt-pbkdf zatim neni), jiny typ = `NOT_SUPPORTED`.
pub fn parse_private(text: &str) -> Result<([u8; 32], PublicKey)> {
    const BEGIN: &str = "-----BEGIN OPENSSH PRIVATE KEY-----";
    const END: &str = "-----END OPENSSH PRIVATE KEY-----";
    if text.len() > MAX_PRIVATE_TEXT {
        return Err(Error::InvalidArgs);
    }
    let start = text.find(BEGIN).ok_or(Error::InvalidArgs)? + BEGIN.len();
    let end = text[start..].find(END).ok_or(Error::InvalidArgs)? + start;
    let b64: String = text[start..end].chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let mut raw = Base64::decode_vec(&b64).map_err(|_| Error::InvalidArgs)?;
    let r = parse_private_raw(&raw);
    zeroize::Zeroize::zeroize(&mut raw);
    r
}

fn parse_private_raw(raw: &[u8]) -> Result<([u8; 32], PublicKey)> {
    const MAGIC: &[u8] = b"openssh-key-v1\0";
    let rest = raw.strip_prefix(MAGIC).ok_or(Error::InvalidArgs)?;
    let mut r = Wire(rest);
    let cipher = r.str()?;
    let _kdf = r.str()?;
    let _kdf_opts = r.bytes()?;
    if cipher != "none" {
        return Err(Error::NotSupported);
    }
    if r.u32()? != 1 {
        return Err(Error::NotSupported);
    }
    let public = PublicKey::from_blob(r.bytes()?)?;
    let mut p = Wire(r.bytes()?);
    r.end()?;
    if p.u32()? != p.u32()? {
        return Err(Error::InvalidArgs);
    }
    if p.str()? != ED25519 || public.algo != Algo::Ed25519 {
        return Err(Error::NotSupported);
    }
    let pk = key32(p.bytes()?)?;
    let sk = p.bytes()?;
    if sk.len() != 64 || pk != public.key || sk[32..] != pk {
        return Err(Error::InvalidArgs);
    }
    let seed = key32(&sk[..32])?;
    if ed25519_public(&seed).key != pk {
        return Err(Error::InvalidArgs);
    }
    let comment: String = p.str()?.chars().filter(|c| !c.is_control()).take(120).collect();
    // Vycpavka 1, 2, 3, ...
    for (i, b) in p.0.iter().enumerate() {
        if *b != (i + 1) as u8 {
            return Err(Error::InvalidArgs);
        }
    }
    Ok((seed, PublicKey { comment, ..public }))
}
