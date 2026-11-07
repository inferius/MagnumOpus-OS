//! Opravneni: prava, granty, popisy opravneni a vypocet ucinnych prav.
//!
//! Model (viz `docs/cs/souborovy-system.md`, kapitola Opravneni):
//!
//! - **Kdo:** `Principal` = uzivatel a jeho skupiny. Identitu prideluje ten,
//!   kdo zaklada relaci (init, spravce sezeni) - FS ji jen dostane. Uzivatel
//!   0 je system: na nej se kontroly nevztahuji.
//! - **Vlastnik** objektu (`Inode::owner`) ma vzdy vsechna prava.
//! - **Granty:** "kdo -> prava" v popisu opravneni (`Acl`). Jen povoleni,
//!   zadne zakazy. Grant plati pro objekt sam (`grant::THIS`) a/nebo se
//!   dedi do potomku (`grant::INHERIT`).
//! - **Dedicnost:** objekt dostane zdedene granty rodice a k nim sve vlastni.
//!   Popis s `acl_flags::PROTECTED` dedicnost prerusi (soukroma slozka uvnitr
//!   sdilene - bez zakazu).
//! - **Popisy se sdileji:** jeden zaznam `ACL` v datasetu, objekty na nej
//!   odkazuji cislem (`Inode::acl`, 0 = jen zdedit) a zaznam pocita odkazy.

use alloc::vec::Vec;

use abi::{Error, Result};

use crate::layout::u64_at;

/// Prava (bity).
pub mod rights {
    /// Vypsat jmena v adresari.
    pub const LIST: u8 = 1 << 0;
    /// Projit adresarem k polozce se znamym jmenem.
    pub const TRAVERSE: u8 = 1 << 1;
    pub const READ: u8 = 1 << 2;
    pub const WRITE: u8 = 1 << 3;
    /// Vytvorit polozku v adresari.
    pub const CREATE: u8 = 1 << 4;
    /// Smazat nebo presunout objekt.
    pub const DELETE: u8 = 1 << 5;
    pub const EXECUTE: u8 = 1 << 6;
    /// Menit opravneni, snapshoty a datasety.
    pub const MANAGE: u8 = 1 << 7;

    /// Uzivatelske urovne.
    pub const VIEW: u8 = LIST | TRAVERSE | READ;
    pub const EDIT: u8 = VIEW | WRITE | CREATE | DELETE;
    pub const RUN: u8 = VIEW | EXECUTE;
    pub const ALL: u8 = 0xFF;
}

/// Priznaky grantu.
pub mod grant {
    /// Plati pro objekt, na kterem je popis.
    pub const THIS: u8 = 1 << 0;
    /// Dedi se do vsech potomku (a jejich potomku).
    pub const INHERIT: u8 = 1 << 1;
    pub const BOTH: u8 = THIS | INHERIT;
}

/// Priznaky popisu opravneni.
pub mod acl_flags {
    /// Nededit granty od rodice.
    pub const PROTECTED: u32 = 1 << 0;
}

/// Uzivatel system (kontroly se nedelaji).
pub const SYSTEM: u64 = 0;
/// Specialni prijemce grantu: kazdy uzivatel krome systemu.
pub const EVERYONE: u64 = u64::MAX;
/// Nejvic grantu v jednom popisu.
pub const MAX_GRANTS: usize = 256;

/// Kdo se pta: uzivatel a jeho skupiny.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Principal {
    pub user: u64,
    pub groups: Vec<u64>,
}

impl Principal {
    pub fn system() -> Self {
        Self::default()
    }

    pub fn user(user: u64, groups: &[u64]) -> Self {
        Self { user, groups: groups.to_vec() }
    }

    pub fn is_system(&self) -> bool {
        self.user == SYSTEM
    }

    /// Plati grant pro tohoto uzivatele?
    pub fn matches(&self, who: u64) -> bool {
        who == self.user || (who == EVERYONE && !self.is_system()) || self.groups.contains(&who)
    }
}

/// Jeden grant: komu, jaka prava, pro co (`grant::*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Grant {
    pub who: u64,
    pub rights: u8,
    pub flags: u8,
}

/// Popis opravneni (bez poctu odkazu - ten je jen na disku).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Acl {
    pub flags: u32,
    pub grants: Vec<Grant>,
}

impl Acl {
    /// Serazeny a slouceny tvar (stejna prava = stejny popis = sdileny
    /// zaznam). Odmitne prazdna prava, neplatne priznaky a prilis mnoho
    /// grantu.
    pub fn normalized(&self) -> Result<Self> {
        if self.flags & !acl_flags::PROTECTED != 0 || self.grants.len() > MAX_GRANTS {
            return Err(Error::InvalidArgs);
        }
        let mut g: Vec<Grant> = Vec::with_capacity(self.grants.len());
        let mut src = self.grants.clone();
        src.sort_by_key(|x| (x.who, x.flags));
        for x in src {
            if x.rights == 0 || x.flags == 0 || x.flags & !grant::BOTH != 0 || x.who == SYSTEM {
                return Err(Error::InvalidArgs);
            }
            match g.last_mut() {
                Some(l) if l.who == x.who && l.flags == x.flags => l.rights |= x.rights,
                _ => g.push(x),
            }
        }
        Ok(Self { flags: self.flags, grants: g })
    }

    pub fn protected(&self) -> bool {
        self.flags & acl_flags::PROTECTED != 0
    }
}

/// Zaznam `ACL` na disku:
///
/// ```text
/// 0   priznaky u32      4   pocet grantu u16    6   rezerva u16
/// 8   pocet odkazu u64 (kolik objektu na popis ukazuje)
/// 16  granty: komu u64, prava u8, priznaky u8 (10 B kazdy)
/// ```
pub fn encode_acl(acl: &Acl, refs: u64) -> Vec<u8> {
    let mut b = alloc::vec![0u8; 16];
    b[0..4].copy_from_slice(&acl.flags.to_le_bytes());
    b[4..6].copy_from_slice(&(acl.grants.len() as u16).to_le_bytes());
    b[8..16].copy_from_slice(&refs.to_le_bytes());
    for g in &acl.grants {
        b.extend_from_slice(&g.who.to_le_bytes());
        b.push(g.rights);
        b.push(g.flags);
    }
    b
}

/// Opak `encode_acl`: (popis, pocet odkazu).
pub fn decode_acl(b: &[u8]) -> Result<(Acl, u64)> {
    if b.len() < 16 {
        return Err(Error::Corrupted);
    }
    let flags = u32::from_le_bytes(b[0..4].try_into().unwrap());
    let n = u16::from_le_bytes([b[4], b[5]]) as usize;
    if n > MAX_GRANTS || b.len() != 16 + n * 10 || flags & !acl_flags::PROTECTED != 0 {
        return Err(Error::Corrupted);
    }
    let refs = u64_at(b, 8);
    let grants = (0..n)
        .map(|i| {
            let o = 16 + i * 10;
            Grant { who: u64_at(b, o), rights: b[o + 8], flags: b[o + 9] }
        })
        .collect();
    Ok((Acl { flags, grants }, refs))
}

/// Otisk popisu pro hledani stejneho (klic indexu `ACL_INDEX`).
pub fn acl_hash(acl: &Acl) -> u64 {
    let b = encode_acl(acl, 0);
    // FNV-1a 64 (stabilni, na disku jen jako klic indexu).
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Granty, ktere sestupuji do potomku (pri pruchodu cestou od korene).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inherited {
    pub grants: Vec<Grant>,
}

impl Inherited {
    /// Prava `p` k objektu s vlastnikem `owner` a popisem `acl` (None =
    /// jen zdedeny), kdyz od rodice prisly granty `self`.
    pub fn rights_of(&self, p: &Principal, owner: u64, acl: Option<&Acl>) -> u8 {
        if p.is_system() || (owner != SYSTEM && owner == p.user) {
            return rights::ALL;
        }
        let mut r = 0;
        if !acl.is_some_and(Acl::protected) {
            for g in &self.grants {
                if p.matches(g.who) {
                    r |= g.rights;
                }
            }
        }
        if let Some(acl) = acl {
            for g in &acl.grants {
                if g.flags & grant::THIS != 0 && p.matches(g.who) {
                    r |= g.rights;
                }
            }
        }
        r
    }

    /// Granty pro potomky objektu s popisem `acl`.
    pub fn child(&self, acl: Option<&Acl>) -> Self {
        let Some(acl) = acl else { return self.clone() };
        let mut grants = if acl.protected() { Vec::new() } else { self.grants.clone() };
        for g in &acl.grants {
            if g.flags & grant::INHERIT != 0 {
                match grants.iter_mut().find(|x| x.who == g.who) {
                    Some(x) => x.rights |= g.rights,
                    None => grants.push(Grant { who: g.who, rights: g.rights, flags: grant::BOTH }),
                }
            }
        }
        Self { grants }
    }
}
