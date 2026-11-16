//! Protokol v2: identita relaci, jmenny prostor relace, opravneni a profily.
//!
//! - **Identita relace** neni na klientovi: hlavni (ridici) kanal sluzby ma
//!   init (pozdeji spravce sezeni) a ten zaklada relace pro uzivatele
//!   (`CONNECT_AS`: uzivatel, skupiny, pripojene cesty). Klient si identitu
//!   vymyslet nemuze - dostane jen kanal.
//! - **Jmenny prostor relace** (capability): relace vidi jen sve
//!   "pripojene" cesty - `jmeno v relaci -> cesta ve FS` (napr. aplikace
//!   `/app -> /home/alice/apps/editor` a `/vybrane/foto.jpg -> ...` z dialogu
//!   Otevrit). Cesta mimo pripojene = `NOT_FOUND`; `..` a `.` se odmitnou.
//!   Uvnitr navic plati opravneni FS podle identity.
//! - **Profily** jsou nastaveni systemu (ne format FS): rozhoduji, jake
//!   pripojene cesty aplikace dostane a zda datasety s daty maji zakaz
//!   spousteni. Pouziva je spravce sezeni (`Profile`), sluzba FS jen
//!   provadi mechanismy.

use alloc::string::String;
use alloc::vec::Vec;

use libos::{Channel, Error, Result};

use crate::{FsClient, Reader, Writer};

/// Kody operaci.
pub mod op {
    /// (jen ridici relace) uzivatel u64, skupiny (u32 + u64...), pripojene
    /// cesty (u32 + Mount...) -> cislo relace u64 + kanal v handlech
    pub const CONNECT_AS: u32 = 46;
    /// (jen ridici) relace u64, Mount - prida pripojenou cestu (powerbox)
    pub const GRANT: u32 = 47;
    /// (jen ridici) relace u64, jmeno str - odebere pripojenou cestu
    pub const REVOKE: u32 = 48;
    /// cesta -> vlastnik u64, prava u8, ma popis u8, [popis]
    pub const PERMISSIONS: u32 = 49;
    /// cesta, ma popis u8, [popis] (vyzaduje MANAGE)
    pub const SET_ACL: u32 = 50;
    /// cesta, vlastnik u64 (jen ridici relace)
    pub const SET_OWNER: u32 = 51;
    /// koren datasetu, zapnout u8 (vyzaduje MANAGE)
    pub const SET_NOEXEC: u32 = 52;
    /// koren datasetu -> zakaz spousteni u8
    pub const NOEXEC: u32 = 53;
    /// cesta -> jako `PERMISSIONS` + zdedene granty (u32 + [who u64,
    /// prava u8, puvod str]); puvod = slozka v relaci, prazdny = mimo
    /// pripojene cesty relace
    pub const ACL_VIEW: u32 = 94;
}

/// Priznaky pripojene cesty.
pub mod mount_flags {
    /// Jen cteni: zapisy, vytvareni a mazani pres tuto cestu se odmitnou.
    pub const READ_ONLY: u8 = 1;
    /// Relace smi pod touto cestou zapsat puvod souboru ([`super::ORIGIN_XATTR`]:
    /// odkud a kdy byl stazen). Dostane ho jen `fetch` pro `~/Stazene`
    /// (docs/cs/stahovani.md); aplikace a uzivatel ne. Ostatni systemove
    /// atributy (`sys.*`) smi dal jen system.
    pub const ORIGIN: u8 = 2;
}

/// Puvod stazeneho souboru: `<adresa>\n<cas ns od 1970>`. Jmenny prostor
/// `sys.` zapisuje jen system (`libs/fs`), tento atribut navic relace
/// s [`mount_flags::ORIGIN`].
pub const ORIGIN_XATTR: &str = "sys.origin";

/// Prava a priznaky grantu (stejne bity jako `fs::rights`, `fs::grant`).
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

pub mod grant {
    pub const THIS: u8 = 1;
    pub const INHERIT: u8 = 2;
    pub const BOTH: u8 = 3;
}

/// Popis opravneni objektu `PROTECTED` = nededit od rodice.
pub const ACL_PROTECTED: u32 = 1;
/// Prijemce grantu: kazdy uzivatel.
pub const EVERYONE: u64 = u64::MAX;

/// Pripojena cesta relace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    /// Jmeno v relaci (absolutni, napr. "/" nebo "/app").
    pub name: String,
    /// Cesta ve FS.
    pub target: String,
    pub flags: u8,
}

impl Mount {
    pub fn new(name: &str, target: &str, flags: u8) -> Self {
        Self { name: name.into(), target: target.into(), flags }
    }

    pub fn put(&self, w: &mut Writer) {
        w.str(&self.name).str(&self.target).u8(self.flags);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        Ok(Self { name: r.str()?, target: r.str()?, flags: r.u8()? })
    }

    pub fn read_only(&self) -> bool {
        self.flags & mount_flags::READ_ONLY != 0
    }
}

/// Grant popisu opravneni.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grant {
    pub who: u64,
    pub rights: u8,
    pub flags: u8,
}

/// Popis opravneni (viz `fs::Acl`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Acl {
    pub flags: u32,
    pub grants: Vec<Grant>,
}

impl Acl {
    pub fn put(&self, w: &mut Writer) {
        w.u32(self.flags).u32(self.grants.len() as u32);
        for g in &self.grants {
            w.u64(g.who).u8(g.rights).u8(g.flags);
        }
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        let flags = r.u32()?;
        let n = r.u32()? as usize;
        if n > 256 {
            return Err(Error::InvalidArgs);
        }
        let mut grants = Vec::with_capacity(n);
        for _ in 0..n {
            grants.push(Grant { who: r.u64()?, rights: r.u8()?, flags: r.u8()? });
        }
        Ok(Self { flags, grants })
    }
}

/// Opravneni objektu (odpoved `PERMISSIONS`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Permissions {
    pub owner: u64,
    pub rights: u8,
    pub acl: Option<Acl>,
}

/// Zdedeny grant s puvodem (odpoved `ACL_VIEW`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InheritedGrant {
    pub who: u64,
    pub rights: u8,
    /// Slozka (v relaci), jejiz popis grant dedi; prazdne = mimo
    /// pripojene cesty relace.
    pub from: String,
}

/// Opravneni pro zobrazeni (odpoved `ACL_VIEW`): vlastni popis a zdedene
/// granty, ktere na objekt plati (prazdne pri `ACL_PROTECTED`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AclView {
    pub perms: Permissions,
    pub inherited: Vec<InheritedGrant>,
}

/// Nejvic zdedenych grantu v odpovedi (jako `Acl::get`, ale pres vic
/// slozek).
pub const MAX_INHERITED: usize = 1024;

impl AclView {
    pub fn put(&self, w: &mut Writer) {
        w.u64(self.perms.owner).u8(self.perms.rights);
        put_opt_acl(w, self.perms.acl.as_ref());
        w.u32(self.inherited.len() as u32);
        for g in &self.inherited {
            w.u64(g.who).u8(g.rights).str(&g.from);
        }
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        let perms = Permissions { owner: r.u64()?, rights: r.u8()?, acl: get_opt_acl(r)? };
        let n = r.u32()? as usize;
        if n > MAX_INHERITED {
            return Err(Error::InvalidArgs);
        }
        let mut inherited = Vec::with_capacity(n);
        for _ in 0..n {
            inherited.push(InheritedGrant { who: r.u64()?, rights: r.u8()?, from: r.str()? });
        }
        Ok(Self { perms, inherited })
    }
}

pub fn put_opt_acl(w: &mut Writer, acl: Option<&Acl>) {
    match acl {
        Some(a) => {
            w.u8(1);
            a.put(w);
        }
        None => {
            w.u8(0);
        }
    }
}

pub fn get_opt_acl(r: &mut Reader) -> Result<Option<Acl>> {
    match r.u8()? {
        0 => Ok(None),
        1 => Ok(Some(Acl::get(r)?)),
        _ => Err(Error::InvalidArgs),
    }
}

/// Overi tvar cesty: absolutni, bez `.` a `..` a prazdnych slozek
/// (krome koncoveho `/`).
pub fn check_path(path: &str) -> Result<()> {
    if !path.starts_with('/') {
        return Err(Error::InvalidArgs);
    }
    let t = path.trim_end_matches('/');
    for c in t.split('/').skip(1) {
        if c.is_empty() || c == "." || c == ".." {
            return Err(Error::InvalidArgs);
        }
    }
    Ok(())
}

/// Prelozi cestu relace na cestu ve FS podle pripojenych cest (nejdelsi
/// shoda na hranici slozky). Vraci (cesta ve FS, pripojeni jen pro cteni).
/// Cesta mimo vsechna pripojeni = `NOT_FOUND`.
/// Priznaky pripojeni, pod kterym cesta `path` relace lezi.
pub fn mount_flags_of(mounts: &[Mount], path: &str) -> Result<u8> {
    check_path(path)?;
    let p = path.trim_end_matches('/');
    let p = if p.is_empty() { "/" } else { p };
    mounts
        .iter()
        .filter(|m| {
            let name = m.name.trim_end_matches('/');
            name.is_empty() || p == name || p.strip_prefix(name).is_some_and(|r| r.starts_with('/'))
        })
        .max_by_key(|m| m.name.trim_end_matches('/').len())
        .map(|m| m.flags)
        .ok_or(Error::NotFound)
}

pub fn map_path(mounts: &[Mount], path: &str) -> Result<(String, bool)> {
    check_path(path)?;
    let p = path.trim_end_matches('/');
    let p = if p.is_empty() { "/" } else { p };
    let mut best: Option<(&Mount, &str)> = None;
    for m in mounts {
        let name = m.name.trim_end_matches('/');
        let rest = if name.is_empty() {
            Some(p)
        } else if p == name {
            Some("")
        } else {
            p.strip_prefix(name).filter(|r| r.starts_with('/'))
        };
        if let Some(rest) = rest
            && best.is_none_or(|(b, _)| b.name.trim_end_matches('/').len() < name.len())
        {
            best = Some((m, rest));
        }
    }
    let (m, rest) = best.ok_or(Error::NotFound)?;
    let target = m.target.trim_end_matches('/');
    let mut out = String::from(target);
    if rest.is_empty() || rest == "/" {
        if out.is_empty() {
            out.push('/');
        }
    } else {
        out.push_str(rest);
    }
    Ok((out, m.read_only()))
}

/// Opak `map_path`: cesta ve FS -> cesta relace (nejdelsi cil pripojene
/// cesty na hranici slozky). `None`, kdyz cesta lezi mimo vsechna pripojeni.
pub fn unmap_path(mounts: &[Mount], path: &str) -> Option<String> {
    let p = path.trim_end_matches('/');
    let p = if p.is_empty() { "/" } else { p };
    let mut best: Option<(&Mount, &str)> = None;
    for m in mounts {
        let target = m.target.trim_end_matches('/');
        let rest = if target.is_empty() {
            Some(p)
        } else if p == target {
            Some("")
        } else {
            p.strip_prefix(target).filter(|r| r.starts_with('/'))
        };
        if let Some(rest) = rest
            && best.is_none_or(|(b, _)| b.target.trim_end_matches('/').len() < target.len())
        {
            best = Some((m, rest));
        }
    }
    let (m, rest) = best?;
    let mut out = String::from(m.name.trim_end_matches('/'));
    if rest.is_empty() || rest == "/" {
        if out.is_empty() {
            out.push('/');
        }
    } else {
        out.push_str(rest);
    }
    Some(out)
}

/// Profil zabezpeceni systemu (nastaveni, ne format). Rozhoduje o
/// pripojenych cestach aplikaci a o zakazu spousteni na datech.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Vychozi: uzivatel vidi sve soubory bez omezeni, aplikace jen svuj
    /// adresar a to, co uzivatel vybral (vyber jde zapamatovat).
    Standard,
    /// Citliva data: aplikace jen svuj adresar a vybrane soubory, vyber se
    /// nepamatuje (kazde otevreni dialogem).
    Strict,
    /// Vyvojari: aplikace vidi cely domov uzivatele.
    Free,
}

impl Profile {
    /// Pripojene cesty relace aplikace `app` uzivatele s domovem `home`
    /// a vybranymi polozkami `picked` (jmeno v relaci, cesta, jen cteni).
    pub fn app_mounts(self, home: &str, app: &str, picked: &[Mount]) -> Vec<Mount> {
        let home = home.trim_end_matches('/');
        let own = Mount::new("/app", &alloc::format!("{home}/apps/{app}"), 0);
        let mut v = match self {
            Profile::Free => alloc::vec![Mount::new("/", home, 0), own],
            Profile::Standard | Profile::Strict => alloc::vec![own],
        };
        v.extend(picked.iter().cloned());
        v
    }

    /// Pripojene cesty relace uzivatele (pruzkumnik, shell): cely FS, prava
    /// urcuji opravneni.
    /// Vymenna media v `/media` (`v2::MEDIA_ROOT`) vidi vsechny aplikace
    /// uzivatele - docasne do Pracovnich slozek (docs/cs/media.md).
    pub fn user_mounts(self) -> Vec<Mount> {
        alloc::vec![Mount::new("/", "/", 0), Mount::new(crate::v2::MEDIA_ROOT, crate::v2::MEDIA_ROOT, 0)]
    }

    /// Maji datasety s daty (stazene soubory, dokumenty) zakaz spousteni?
    pub fn noexec_on_data(self) -> bool {
        !matches!(self, Profile::Free)
    }

    /// Smi se vyber souboru aplikaci zapamatovat ("tahle aplikace smi do
    /// Projekty vzdycky")?
    pub fn remember_picks(self) -> bool {
        !matches!(self, Profile::Strict)
    }
}

/// Spolecna docasna slozka; lide v ni jen prochazeji do sve [`user_tmp`].
pub const TMP_DIR: &str = "/tmp";

/// Soukroma docasna slozka uzivatele `uid` (`/tmp/<uid>`): zaklada ji init
/// pri otevreni relace, patri jen uzivateli (zadne granty). Do spolecneho
/// `/tmp` lide zapisovat nesmi.
pub fn user_tmp(uid: u64) -> String {
    alloc::format!("{TMP_DIR}/{uid}")
}

impl FsClient {
    /// (Ridici relace) Nova relace uzivatele `user` se skupinami `groups`
    /// a pripojenymi cestami `mounts`. Vraci cislo relace (pro `grant`)
    /// a klienta, ktery jde predat aplikaci.
    pub fn connect_as(&self, user: u64, groups: &[u64], mounts: &[Mount]) -> Result<(u64, FsClient)> {
        let mut w = Self::req(op::CONNECT_AS);
        w.u64(user).u32(groups.len() as u32);
        for g in groups {
            w.u64(*g);
        }
        w.u32(mounts.len() as u32);
        for m in mounts {
            m.put(&mut w);
        }
        let (body, mut handles) = self.call_with_handles(w)?;
        let id = Reader::new(&body).u64()?;
        let h = handles.pop().ok_or(Error::Corrupted)?;
        Ok((id, FsClient::new(Channel::from_handle(h))))
    }

    /// (Ridici relace) Prida relaci `session` pripojenou cestu.
    pub fn grant(&self, session: u64, m: &Mount) -> Result<()> {
        let mut w = Self::req(op::GRANT);
        w.u64(session);
        m.put(&mut w);
        self.call(w).map(|_| ())
    }

    /// (Ridici relace) Odebere relaci `session` pripojenou cestu `name`.
    pub fn revoke(&self, session: u64, name: &str) -> Result<()> {
        let mut w = Self::req(op::REVOKE);
        w.u64(session).str(name);
        self.call(w).map(|_| ())
    }

    pub fn permissions(&self, path: &str) -> Result<Permissions> {
        let mut w = Self::req(op::PERMISSIONS);
        w.str(path);
        let body = self.call(w)?;
        let mut r = Reader::new(&body);
        Ok(Permissions { owner: r.u64()?, rights: r.u8()?, acl: get_opt_acl(&mut r)? })
    }

    /// Opravneni pro zobrazeni (vlastni popis + zdedene s puvodem).
    pub fn acl_view(&self, path: &str) -> Result<AclView> {
        let mut w = Self::req(op::ACL_VIEW);
        w.str(path);
        let body = self.call(w)?;
        AclView::get(&mut Reader::new(&body))
    }

    pub fn set_acl(&self, path: &str, acl: Option<&Acl>) -> Result<()> {
        let mut w = Self::req(op::SET_ACL);
        w.str(path);
        put_opt_acl(&mut w, acl);
        self.call(w).map(|_| ())
    }

    pub fn set_owner(&self, path: &str, owner: u64) -> Result<()> {
        let mut w = Self::req(op::SET_OWNER);
        w.str(path).u64(owner);
        self.call(w).map(|_| ())
    }

    pub fn set_noexec(&self, dataset: &str, on: bool) -> Result<()> {
        let mut w = Self::req(op::SET_NOEXEC);
        w.str(dataset).u8(on as u8);
        self.call(w).map(|_| ())
    }

    pub fn noexec(&self, dataset: &str) -> Result<bool> {
        let mut w = Self::req(op::NOEXEC);
        w.str(dataset);
        Ok(Reader::new(&self.call(w)?).u8()? != 0)
    }
}
