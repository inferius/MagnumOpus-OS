//! DNSSEC (RFC 4033-4035, RFC 5155, RFC 6840): overeni podpisu a retezu.
//!
//! - [`Dnskey`], [`Ds`], [`Rrsig`]: zaznamy klicu, otisku a podpisu.
//! - [`verify_rrset`]: podpis RRSIG sady zaznamu klicem (kanonicky tvar
//!   RFC 4034 3.1.8.1, poradi 6.3, wildcard 5.3.2, casy se seriovou
//!   aritmetikou RFC 1982).
//! - [`denial`]: dukazy neexistence NSEC (RFC 4035 5.4) a NSEC3 (RFC 5155 8).
//! - [`chain`]: validator - retez DS od korenove kotvy, stav odpovedi.
//!
//! Algoritmy: 8 RSASHA256, 10 RSASHA512, 13 ECDSAP256SHA256,
//! 14 ECDSAP384SHA384, 15 ED25519 (RFC 8624: MUST a RECOMMENDED). Otisky DS:
//! 2 SHA-256, 4 SHA-384. Jine algoritmy = zona se bere jako nepodepsana
//! (RFC 4035 5.2), ne jako podvrzena.

pub mod chain;
pub mod denial;

use alloc::vec::Vec;

use sha2::{Digest, Sha256, Sha384, Sha512};
use signature::Verifier;
use signature::hazmat::PrehashVerifier;

use crate::message::Record;
use crate::name::{self, Name};
use crate::{Error, Result, rtype};

pub mod alg {
    pub const RSASHA256: u8 = 8;
    pub const RSASHA512: u8 = 10;
    pub const ECDSAP256SHA256: u8 = 13;
    pub const ECDSAP384SHA384: u8 = 14;
    pub const ED25519: u8 = 15;

    pub fn supported(a: u8) -> bool {
        matches!(a, RSASHA256 | RSASHA512 | ECDSAP256SHA256 | ECDSAP384SHA384 | ED25519)
    }
}

pub mod digest {
    pub const SHA256: u8 = 2;
    pub const SHA384: u8 = 4;

    pub fn supported(d: u8) -> bool {
        matches!(d, SHA256 | SHA384)
    }
}

/// Zaznam DNSKEY.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dnskey {
    pub owner: Name,
    pub flags: u16,
    pub protocol: u8,
    pub algorithm: u8,
    pub key: Vec<u8>,
    /// Cele RDATA (pro otisk DS a key tag).
    pub rdata: Vec<u8>,
}

/// Priznak ZONE (klic zony; bez nej se klic k podpisum nesmi pouzit).
pub const FLAG_ZONE: u16 = 0x0100;
/// Priznak REVOKE (RFC 5011): klic se nepouzije.
pub const FLAG_REVOKE: u16 = 0x0080;

impl Dnskey {
    pub fn parse(r: &Record) -> Result<Self> {
        let d = &r.rdata;
        if r.rtype != rtype::DNSKEY || d.len() < 5 {
            return Err(Error::Malformed("DNSKEY"));
        }
        Ok(Self {
            owner: r.name.clone(),
            flags: u16::from_be_bytes([d[0], d[1]]),
            protocol: d[2],
            algorithm: d[3],
            key: d[4..].to_vec(),
            rdata: d.clone(),
        })
    }

    /// Key tag (RFC 4034 dodatek B).
    pub fn key_tag(&self) -> u16 {
        let mut ac: u32 = 0;
        for (i, &b) in self.rdata.iter().enumerate() {
            ac += if i & 1 == 1 { b as u32 } else { (b as u32) << 8 };
        }
        ac += (ac >> 16) & 0xFFFF;
        (ac & 0xFFFF) as u16
    }

    /// Lze klic pouzit k overeni podpisu zony?
    pub fn usable(&self) -> bool {
        self.flags & FLAG_ZONE != 0 && self.flags & FLAG_REVOKE == 0 && self.protocol == 3
    }

    /// Otisk pro DS (RFC 4034 5.1.4): H(jmeno vlastnika kanonicky || RDATA).
    pub fn ds_digest(&self, digest_type: u8) -> Option<Vec<u8>> {
        let mut data = self.owner.to_wire(true);
        data.extend_from_slice(&self.rdata);
        match digest_type {
            digest::SHA256 => Some(Sha256::digest(&data).to_vec()),
            digest::SHA384 => Some(Sha384::digest(&data).to_vec()),
            _ => None,
        }
    }
}

/// Zaznam DS (otisk klice podrizene zony v nadrazene zone).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ds {
    pub key_tag: u16,
    pub algorithm: u8,
    pub digest_type: u8,
    pub digest: Vec<u8>,
}

impl Ds {
    pub fn parse(r: &Record) -> Result<Self> {
        let d = &r.rdata;
        if r.rtype != rtype::DS || d.len() < 5 {
            return Err(Error::Malformed("DS"));
        }
        Ok(Self {
            key_tag: u16::from_be_bytes([d[0], d[1]]),
            algorithm: d[2],
            digest_type: d[3],
            digest: d[4..].to_vec(),
        })
    }

    /// Odpovida DS klici `k`?
    pub fn matches(&self, k: &Dnskey) -> bool {
        self.key_tag == k.key_tag()
            && self.algorithm == k.algorithm
            && k.ds_digest(self.digest_type).is_some_and(|d| d == self.digest)
    }
}

/// Zaznam RRSIG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rrsig {
    pub type_covered: u16,
    pub algorithm: u8,
    pub labels: u8,
    pub original_ttl: u32,
    pub expiration: u32,
    pub inception: u32,
    pub key_tag: u16,
    pub signer: Name,
    pub signature: Vec<u8>,
    /// RDATA bez podpisu, jmeno podepisujiciho kanonicky.
    signed_prefix: Vec<u8>,
}

impl Rrsig {
    pub fn parse(r: &Record) -> Result<Self> {
        let d = &r.rdata;
        if r.rtype != rtype::RRSIG || d.len() < 19 {
            return Err(Error::Malformed("RRSIG"));
        }
        let (signer, end) = name::read(d, 18)?;
        let mut signed_prefix = d[..18].to_vec();
        signer.write(&mut signed_prefix, true);
        Ok(Self {
            type_covered: u16::from_be_bytes([d[0], d[1]]),
            algorithm: d[2],
            labels: d[3],
            original_ttl: u32::from_be_bytes(d[4..8].try_into().unwrap()),
            expiration: u32::from_be_bytes(d[8..12].try_into().unwrap()),
            inception: u32::from_be_bytes(d[12..16].try_into().unwrap()),
            key_tag: u16::from_be_bytes([d[16], d[17]]),
            signer,
            signature: d[end..].to_vec(),
            signed_prefix,
        })
    }

    /// Plati podpis v case `now` (sekundy, seriova aritmetika RFC 1982)?
    pub fn in_validity(&self, now: u32) -> bool {
        let after = |a: u32, b: u32| (a.wrapping_sub(b) as i32) >= 0;
        after(now, self.inception) && after(self.expiration, now)
    }
}

/// Overi podpis `sig` bajtu `data` klicem `key`.
pub fn verify_signature(key: &Dnskey, data: &[u8], sig: &[u8]) -> bool {
    match key.algorithm {
        alg::RSASHA256 | alg::RSASHA512 => {
            let Some(pk) = rsa_key(&key.key) else {
                return false;
            };
            if key.algorithm == alg::RSASHA256 {
                let Ok(s) = rsa::pkcs1v15::Signature::try_from(sig) else {
                    return false;
                };
                rsa::pkcs1v15::VerifyingKey::<Sha256>::new(pk).verify(data, &s).is_ok()
            } else {
                let Ok(s) = rsa::pkcs1v15::Signature::try_from(sig) else {
                    return false;
                };
                rsa::pkcs1v15::VerifyingKey::<Sha512>::new(pk).verify(data, &s).is_ok()
            }
        }
        alg::ECDSAP256SHA256 => {
            if key.key.len() != 64 || sig.len() != 64 {
                return false;
            }
            let mut sec1 = Vec::from([4u8]);
            sec1.extend_from_slice(&key.key);
            let (Ok(vk), Ok(s)) = (p256::ecdsa::VerifyingKey::from_sec1_bytes(&sec1), p256::ecdsa::Signature::from_slice(sig))
            else {
                return false;
            };
            vk.verify_prehash(&Sha256::digest(data), &s).is_ok()
        }
        alg::ECDSAP384SHA384 => {
            if key.key.len() != 96 || sig.len() != 96 {
                return false;
            }
            let mut sec1 = Vec::from([4u8]);
            sec1.extend_from_slice(&key.key);
            let (Ok(vk), Ok(s)) = (p384::ecdsa::VerifyingKey::from_sec1_bytes(&sec1), p384::ecdsa::Signature::from_slice(sig))
            else {
                return false;
            };
            vk.verify_prehash(&Sha384::digest(data), &s).is_ok()
        }
        alg::ED25519 => {
            let (Ok(k), Ok(s)) = (<[u8; 32]>::try_from(key.key.as_slice()), ed25519_dalek::Signature::from_slice(sig)) else {
                return false;
            };
            ed25519_dalek::VerifyingKey::from_bytes(&k).is_ok_and(|vk| vk.verify_strict(data, &s).is_ok())
        }
        _ => false,
    }
}

/// Verejny klic RSA (RFC 3110): delka exponentu, exponent, modul.
/// Nejmensi delka modulu 1024 bitu (RFC 8624 dovoluje overit i 1024).
fn rsa_key(k: &[u8]) -> Option<rsa::RsaPublicKey> {
    let (elen, rest) = match *k.first()? {
        0 => (u16::from_be_bytes([*k.get(1)?, *k.get(2)?]) as usize, k.get(3..)?),
        n => (n as usize, k.get(1..)?),
    };
    let (e, n) = (rest.get(..elen)?, rest.get(elen..)?);
    if n.len() * 8 < 1024 || n.len() > 512 {
        return None;
    }
    rsa::RsaPublicKey::new(rsa::BigUint::from_bytes_be(n), rsa::BigUint::from_bytes_be(e)).ok()
}

/// Overi sadu zaznamu `rrset` (stejne jmeno, typ, trida) podpisem `sig`
/// klicem `key` v case `now`. Vraci jmeno, ktere sadu "vytvorilo" pri
/// wildcardu (`Some(*.ce)`), jinak `None`.
pub fn verify_rrset(rrset: &[&Record], sig: &Rrsig, key: &Dnskey, now: u32) -> Result<Option<Name>> {
    let first = rrset.first().ok_or(Error::Bogus("prazdna sada"))?;
    if sig.type_covered != first.rtype || sig.algorithm != key.algorithm || sig.key_tag != key.key_tag() {
        return Err(Error::Bogus("podpis nepatri sade nebo klici"));
    }
    if !key.usable() || key.owner != sig.signer || !first.name.is_in(&sig.signer) {
        return Err(Error::Bogus("klic neni klicem zony podpisu"));
    }
    if !sig.in_validity(now) {
        return Err(Error::Bogus("podpis propadly nebo jeste neplatny"));
    }
    let owner_labels = first.name.labels.len() - usize::from(first.name.is_wildcard());
    if sig.labels as usize > owner_labels {
        return Err(Error::Bogus("pocet navesti podpisu"));
    }
    // Wildcard: vlastnik v podpisu je "*." + poslednich `labels` navesti.
    let (owner, wildcard) = if (sig.labels as usize) < owner_labels {
        let w = first.name.suffix(sig.labels as usize).wildcard_child();
        (w.clone(), Some(w))
    } else {
        (first.name.clone(), None)
    };
    let owner_wire = owner.to_wire(true);
    let mut rdatas: Vec<Vec<u8>> = rrset.iter().map(|r| r.canonical_rdata()).collect();
    rdatas.sort();
    rdatas.dedup();
    let mut data = sig.signed_prefix.clone();
    for rd in &rdatas {
        data.extend_from_slice(&owner_wire);
        data.extend_from_slice(&first.rtype.to_be_bytes());
        data.extend_from_slice(&first.class.to_be_bytes());
        data.extend_from_slice(&sig.original_ttl.to_be_bytes());
        data.extend_from_slice(&(rd.len() as u16).to_be_bytes());
        data.extend_from_slice(rd);
    }
    if verify_signature(key, &data, &sig.signature) {
        Ok(wildcard)
    } else {
        Err(Error::Bogus("podpis nesedi"))
    }
}

/// Sady zaznamu (jmeno, typ) ze sekce, bez RRSIG.
pub fn rrsets(records: &[Record]) -> Vec<Vec<&Record>> {
    let mut out: Vec<Vec<&Record>> = Vec::new();
    for r in records.iter().filter(|r| r.rtype != rtype::RRSIG && r.rtype != rtype::OPT) {
        match out.iter_mut().find(|s| s[0].rtype == r.rtype && s[0].name == r.name && s[0].class == r.class) {
            Some(s) => s.push(r),
            None => out.push(alloc::vec![r]),
        }
    }
    out
}

/// Podpisy RRSIG pro sadu (jmeno, typ) ze sekce.
pub fn signatures_for(records: &[Record], owner: &Name, covered: u16) -> Vec<Rrsig> {
    records
        .iter()
        .filter(|r| r.rtype == rtype::RRSIG && r.name == *owner)
        .filter_map(|r| Rrsig::parse(r).ok())
        .filter(|s| s.type_covered == covered)
        .collect()
}

/// Overi sadu nekterym podpisem nekterym z klicu. `Ok(wildcard)` jako
/// [`verify_rrset`].
pub fn verify_with_keys(rrset: &[&Record], sigs: &[Rrsig], keys: &[Dnskey], now: u32) -> Result<Option<Name>> {
    let mut last = Error::Bogus("chybi podpis");
    for s in sigs {
        for k in keys.iter().filter(|k| k.key_tag() == s.key_tag && k.algorithm == s.algorithm) {
            match verify_rrset(rrset, s, k, now) {
                Ok(w) => return Ok(w),
                Err(e) => last = e,
            }
        }
    }
    Err(last)
}
