//! Opravneni souboru pro prikaz `acl`: popis opravneni (granty), urovne
//! a jmena prijemcu.
//!
//! Bity prav i priznaky grantu jsou stejne jako v `fsproto::perm`
//! (`libs/fsproto/src/perm.rs`) a `fs::rights`; shell na protokolu
//! nezavisi, prevod dela hostitel (`user/shsess/src/fs.rs`). Cisla identit
//! jsou z `accountproto::ids` (`docs/cs/ucty.md`, Cisla identit).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Prava (bity jako `fsproto::perm::rights`).
pub mod rights {
    pub const LIST: u8 = 1 << 0;
    pub const TRAVERSE: u8 = 1 << 1;
    pub const READ: u8 = 1 << 2;
    pub const WRITE: u8 = 1 << 3;
    pub const CREATE: u8 = 1 << 4;
    pub const DELETE: u8 = 1 << 5;
    pub const EXECUTE: u8 = 1 << 6;
    pub const MANAGE: u8 = 1 << 7;
    pub const VIEW: u8 = LIST | TRAVERSE | READ;
    pub const EDIT: u8 = VIEW | WRITE | CREATE | DELETE;
    pub const RUN: u8 = VIEW | EXECUTE;
    pub const ALL: u8 = 0xFF;
}

/// Priznaky grantu (jako `fsproto::perm::grant`).
pub mod grant {
    /// Plati pro objekt sam.
    pub const THIS: u8 = 1;
    /// Dedi se na potomky (slozky).
    pub const INHERIT: u8 = 2;
}

/// Prijemce grantu: kazdy uzivatel (`fsproto::perm::EVERYONE`).
pub const EVERYONE: u64 = u64::MAX;
/// Skupina vsech lidi (`accountproto::ids::GROUP_USERS`).
pub const GROUP_USERS: u64 = 1_000_000;
/// Prvni cislo cloveka (`accountproto::ids::FIRST_USER`).
pub const FIRST_USER: u64 = 1000;

/// Grant popisu opravneni.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grant {
    pub who: u64,
    pub rights: u8,
    /// Bity [`grant`].
    pub flags: u8,
}

/// Vlastni popis opravneni objektu.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Acl {
    /// `true` = nededi od rodice (`fsproto::perm::ACL_PROTECTED`).
    pub protected: bool,
    pub grants: Vec<Grant>,
}

/// Opravneni objektu, jak je vraci FS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Perms {
    pub owner: u64,
    /// Ucinna prava volajiciho.
    pub rights: u8,
    /// `None` = objekt nema vlastni popis (jen zdedena prava).
    pub acl: Option<Acl>,
}

/// Ucet ze sluzby uctu (jmeno pro prikaz, zobrazovane jmeno).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub uid: u64,
    pub name: String,
    pub display: String,
}

/// Uroven prav v prikazu: slovo prikazu, popis pro cloveka, maska.
/// Vzestupne a stejne jako karta Opravneni v Souborech
/// (`filesview::props::Level`): Upravovat zahrnuje i spousteni.
pub const LEVELS: &[(&str, &str, u8)] = &[
    ("view", "Zobrazit", rights::VIEW),
    ("run", "Spouštět", rights::RUN),
    ("edit", "Upravovat", rights::EDIT | rights::EXECUTE),
    ("manage", "Spravovat", rights::ALL),
];

/// Maska podle slova urovne.
pub fn level_rights(word: &str) -> Option<u8> {
    LEVELS.iter().find(|l| l.0 == word).map(|l| l.2)
}

/// (slovo, popis) urovne masky; jina maska nez ctyri urovne = `custom`
/// s vypisem bitu (granty zalozene jinde, napr. aplikaci).
pub fn level_of(r: u8) -> (String, String) {
    if let Some(l) = LEVELS.iter().find(|l| l.2 == r) {
        return (l.0.into(), l.1.into());
    }
    const BITS: &[(u8, &str)] = &[
        (rights::LIST, "výpis"),
        (rights::TRAVERSE, "průchod"),
        (rights::READ, "čtení"),
        (rights::WRITE, "zápis"),
        (rights::CREATE, "vytváření"),
        (rights::DELETE, "mazání"),
        (rights::EXECUTE, "spouštění"),
        (rights::MANAGE, "správa"),
    ];
    let parts: Vec<&str> = BITS.iter().filter(|b| r & b.0 != 0).map(|b| b.1).collect();
    let label = if parts.is_empty() { String::from("nic") } else { parts.join(", ") };
    (String::from("custom"), format!("Vlastní ({label})"))
}

/// Druh prijemce: `user` nebo `group` (vsichni jsou skupina).
pub fn kind_of(who: u64) -> &'static str {
    if who >= GROUP_USERS { "group" } else { "user" }
}

/// Jmeno prijemce pro vypis: zname skupiny, ucty podle seznamu, jinak
/// cislo s druhem.
pub fn name_of(who: u64, accounts: &[Account]) -> String {
    match who {
        EVERYONE => String::from("everyone"),
        GROUP_USERS => String::from("users"),
        _ => match accounts.iter().find(|a| a.uid == who) {
            Some(a) => a.name.clone(),
            None if who >= GROUP_USERS => format!("skupina {who}"),
            None => format!("uživatel {who}"),
        },
    }
}

/// Cislo prijemce podle jmena: `everyone`, `users`, jmeno uctu nebo
/// cislo. `None` = nikdo takovy.
pub fn who_of(name: &str, accounts: &[Account]) -> Option<u64> {
    match name {
        "everyone" => Some(EVERYONE),
        "users" => Some(GROUP_USERS),
        _ => accounts
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.uid)
            .or_else(|| name.parse::<u64>().ok()),
    }
}

/// Prida nebo nahradi grant `who` (prava a priznaky); ostatni granty
/// i priznak dedeni zustanou. Objekt bez vlastniho popisu dostane novy
/// (dedi dal od rodice).
pub fn with_grant(acl: Option<Acl>, who: u64, rights: u8, flags: u8) -> Acl {
    let mut acl = acl.unwrap_or_default();
    acl.grants.retain(|g| g.who != who);
    acl.grants.push(Grant { who, rights, flags });
    acl
}

/// Odebere vsechny granty `who`. `None` = zadny nebyl.
pub fn without_grant(acl: Option<Acl>, who: u64) -> Option<Acl> {
    let mut acl = acl?;
    let n = acl.grants.len();
    acl.grants.retain(|g| g.who != who);
    (acl.grants.len() != n).then_some(acl)
}
