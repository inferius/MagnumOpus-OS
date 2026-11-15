//! Kontrola opravneni v `Fs` (model viz `crate::perm`).
//!
//! Kdo se pta, nastavuje volajici (`set_principal`, fsd podle relace) -
//! jako cas pres `set_time`. `None` nebo system = bez kontrol (nastroje na
//! hostiteli, ovladac pro Windows, interni operace).
//!
//! **Kde se prava kontroluji:** verejne operace s cestou projdou cestu od
//! korene a pocitaji zdedene granty. Kazdy adresar po ceste potrebuje
//! `TRAVERSE`; objekt, ke kteremu volajici nema zadne pravo, je pro nej
//! neviditelny (`NOT_FOUND`, ne `ACCESS_DENIED` - neprozradi se ani jmeno).
//! Operace pres handle se uz nekontroluji: prava se overila pri `open`.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use abi::{Error, Result};

use super::{DirEntry, Fs, Kind, split_parent, zil};
use crate::btree;
use crate::device::BlockDevice;
use crate::layout::{KIND_DATASET, KIND_DIR, Key, META_OID, ROOT_DATASET, ROOT_OID, feature, item};
use crate::node::Child;
use crate::perm::{Acl, Inherited, Principal, SYSTEM, acl_hash, decode_acl, encode_acl, grant, rights};

/// Vlastnosti datasetu (polozka `PROPS` datasetu, bity).
pub mod ds_flags {
    /// Zakaz spousteni: `open` s `access::EXECUTE` selze pro kazdeho
    /// (datasety s daty - stazene soubory, dokumenty).
    pub const NOEXEC: u32 = 1 << 0;
}

/// Vysledek pruchodu cestou s pravy.
pub(super) struct Walked {
    pub oid: u64,
    /// Ucinna prava k objektu.
    pub rights: u8,
    /// Granty pro potomky objektu.
    pub below: Inherited,
    /// Granty, ktere k objektu prisly od rodice (mod POSIX, `crate::mode`).
    pub above: Inherited,
}

/// Opravneni objektu, jak je vraci `Fs::permissions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Permissions {
    pub owner: u64,
    /// Vlastni popis objektu (`None` = jen zdedena prava).
    pub acl: Option<Acl>,
    /// Ucinna prava volajiciho.
    pub rights: u8,
}

/// Zdedeny grant s puvodem (`Fs::acl_view`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InheritedGrant {
    pub who: u64,
    pub rights: u8,
    /// Nejblizsi nadrazena slozka, jejiz popis grant dedi (cesta ve FS).
    pub from: String,
}

/// Opravneni objektu pro zobrazeni: vlastni popis a zdedene granty
/// s puvodem (karta Opravneni ve Vlastnostech, prikaz `acl`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AclView {
    pub perms: Permissions,
    /// Zdedene granty, ktere na objekt opravdu plati (prazdne, kdyz vlastni
    /// popis ma `PROTECTED`). Kazda slozka po ceste zvlast, poradi od
    /// korene; v ramci slozky podle `who`.
    pub inherited: Vec<InheritedGrant>,
}

impl<D: BlockDevice> Fs<D> {
    /// Kdo provadi nasledujici operace (`None` = system, bez kontrol).
    pub fn set_principal(&mut self, p: Option<Principal>) {
        self.principal = p.filter(|p| !p.is_system());
    }

    /// Kontroluje se? (Prehrani logu bezi pri pripojeni, kdy volajici
    /// jeste neni nastaveny - operace uz byly povolene.)
    pub(super) fn checking(&self) -> bool {
        self.principal.is_some()
    }

    /// Vlastnik pro nove objekty.
    pub(super) fn new_owner(&self) -> u64 {
        if let Some(o) = self.owner_override {
            return o;
        }
        self.principal.as_ref().map_or(SYSTEM, |p| p.user)
    }

    pub(super) fn load_acl(&mut self, root: &Child, id: u32) -> Result<Option<Acl>> {
        if id == 0 {
            return Ok(None);
        }
        let raw = btree::get(&mut self.s, root, &Key::new(META_OID, item::ACL, id as u64))?.ok_or(Error::Corrupted)?;
        Ok(Some(decode_acl(&raw)?.0))
    }

    /// Projde zivou cestu (pres hranice datasetu) a spocita prava. Na konci
    /// je aktivni dataset cile. Adresar bez `TRAVERSE` po ceste a cil bez
    /// prav = `NOT_FOUND`.
    pub(super) fn walk(&mut self, path: &str) -> Result<Walked> {
        if !path.starts_with('/') {
            return Err(Error::InvalidArgs);
        }
        self.activate(ROOT_DATASET)?;
        let root = self.s.root;
        self.walk_from(root, ROOT_OID, Inherited::default(), super::split_path(path), true)
    }

    /// Pruchod od objektu `oid` stromu `root` se zdedenymi granty
    /// `inherited`; `cross` = smi vstoupit do jineho datasetu (jen zivy
    /// strom aktivniho datasetu).
    fn walk_from<'a>(
        &mut self,
        mut root: Child,
        mut oid: u64,
        mut inherited: Inherited,
        names: impl Iterator<Item = &'a str>,
        cross: bool,
    ) -> Result<Walked> {
        let p = self.principal.clone().unwrap_or_default();
        let mut inode = self.inode(&root, oid)?;
        let mut acl = self.load_acl(&root, inode.acl)?;
        let mut r = inherited.rights_of(&p, inode.owner, acl.as_ref());
        for name in names {
            if inode.kind != KIND_DIR {
                return Err(Error::NotADirectory);
            }
            if r & rights::TRAVERSE == 0 {
                return Err(Error::NotFound);
            }
            inherited = inherited.child(acl.as_ref());
            let d = self.lookup_child(&root, oid, name)?.ok_or(Error::NotFound)?;
            if d.kind == KIND_DATASET {
                // Hranice v klonu (viz `dataset.rs`): dovnitr nejde.
                if !cross || self.is_boundary(oid, &d)? {
                    return Err(Error::NotSupported);
                }
                self.activate(d.oid)?;
                root = self.s.root;
                oid = ROOT_OID;
            } else {
                oid = d.oid;
            }
            inode = self.inode(&root, oid)?;
            acl = self.load_acl(&root, inode.acl)?;
            r = inherited.rights_of(&p, inode.owner, acl.as_ref());
        }
        if r == 0 {
            return Err(Error::NotFound);
        }
        let below = inherited.child(acl.as_ref());
        Ok(Walked { oid, rights: r, below, above: inherited })
    }

    /// Projde cestu a overi prava `need` (bez kontrol jen `enter`).
    pub(super) fn require(&mut self, path: &str, need: u8) -> Result<u64> {
        if !self.checking() {
            return self.enter(path);
        }
        let w = self.walk(path)?;
        if w.rights & need != need {
            return Err(Error::AccessDenied);
        }
        Ok(w.oid)
    }

    /// Overi prava `need` k rodici cesty (vytvoreni polozky).
    pub(super) fn require_parent(&mut self, path: &str, need: u8) -> Result<()> {
        if self.checking() {
            let (parent, _) = split_parent(path)?;
            self.require(parent, need)?;
        }
        Ok(())
    }

    /// Operace jen pro system (kontrola svazku, defragmentace).
    pub(super) fn require_system(&self) -> Result<()> {
        if self.checking() { Err(Error::AccessDenied) } else { Ok(()) }
    }

    /// Ucinna prava volajiciho k objektu (`rights::ALL` pro system).
    pub fn rights(&mut self, path: &str) -> Result<u8> {
        if !self.checking() {
            self.enter(path)?;
            return Ok(rights::ALL);
        }
        Ok(self.walk(path)?.rights)
    }

    /// Vlastnik, vlastni popis a ucinna prava objektu.
    pub fn permissions(&mut self, path: &str) -> Result<Permissions> {
        let rights = self.rights(path)?;
        let oid = self.enter(path)?;
        let root = self.s.root;
        let inode = self.inode(&root, oid)?;
        let acl = self.load_acl(&root, inode.acl)?;
        Ok(Permissions { owner: inode.owner, acl, rights })
    }

    /// Mod POSIX objektu (9 bitu, `crate::mode`) z vlastnika, druhu,
    /// zdedenych grantu `above` a vlastniho popisu.
    pub(super) fn mode_at(&mut self, root: &Child, oid: u64, above: &Inherited) -> Result<u16> {
        let inode = self.inode(root, oid)?;
        let acl = self.load_acl(root, inode.acl)?;
        let grants = crate::mode::effective(above, acl.as_ref());
        Ok(crate::mode::mode_of(inode.owner, inode.kind == KIND_DIR, &grants))
    }

    /// Mod POSIX objektu `path` (pro system i bez kontrol).
    pub fn mode(&mut self, path: &str) -> Result<u16> {
        let w = self.walk(path)?;
        let root = self.s.root;
        self.mode_at(&root, w.oid, &w.above)
    }

    /// Zmeni opravneni podle modu POSIX (9 bitu; setuid/setgid/sticky se
    /// ignoruji). `widen` = `chmod` (smi rozsirit), jinak jen zuzi (mod
    /// pri vytvoreni). Vyzaduje `MANAGE` (vlastnik ho ma vzdy).
    pub fn set_mode(&mut self, path: &str, mode: u16, widen: bool) -> Result<()> {
        let w = self.walk(path)?;
        if self.checking() && w.rights & rights::MANAGE == 0 {
            return Err(Error::AccessDenied);
        }
        let root = self.s.root;
        let inode = self.inode(&root, w.oid)?;
        let acl = self.load_acl(&root, inode.acl)?;
        let dir = inode.kind == KIND_DIR;
        let Some(new) = crate::mode::apply(inode.owner, dir, &w.above, acl.as_ref(), mode & 0o777, widen) else {
            return Ok(());
        };
        let oid = w.oid;
        self.guarded(|fs| {
            fs.s.check_writable()?;
            fs.ensure_room(4)?;
            fs.set_acl_oid(oid, Some(&new))
        })
    }

    /// Vlastnik, vlastni popis, ucinna prava a zdedene granty s puvodem.
    /// Stejna prava jako `permissions` (jakekoli pravo k objektu): kdo
    /// objekt vidi, smi videt, kdo k nemu ma pristup. Pruchod predky uz
    /// prava nekontroluje (overil je `rights` celou cestou).
    pub fn acl_view(&mut self, path: &str) -> Result<AclView> {
        let perms = self.permissions(path)?;
        let names: Vec<&str> = super::split_path(path).collect();
        let mut inherited = Vec::new();
        for i in 0..names.len() {
            let mut dir = String::from("/");
            dir.push_str(&names[..i].join("/"));
            let oid = self.enter(&dir)?;
            let root = self.s.root;
            let inode = self.inode(&root, oid)?;
            let Some(acl) = self.load_acl(&root, inode.acl)? else { continue };
            if acl.protected() {
                inherited.clear();
            }
            for g in acl.grants.iter().filter(|g| g.flags & grant::INHERIT != 0) {
                inherited.push(InheritedGrant { who: g.who, rights: g.rights, from: dir.clone() });
            }
        }
        if perms.acl.as_ref().is_some_and(Acl::protected) {
            inherited.clear();
        }
        Ok(AclView { perms, inherited })
    }

    /// Nastavi vlastni popis opravneni objektu (`None` = jen dedit).
    /// Vyzaduje `MANAGE`.
    pub fn set_acl(&mut self, path: &str, acl: Option<Acl>) -> Result<()> {
        let acl = match acl {
            Some(a) => Some(a.normalized()?),
            None => None,
        };
        self.guarded(|fs| {
            let oid = fs.require(path, rights::MANAGE)?;
            fs.s.check_writable()?;
            fs.ensure_room(4)?;
            fs.set_acl_oid(oid, acl.as_ref())
        })
    }

    pub(super) fn set_acl_oid(&mut self, oid: u64, acl: Option<&Acl>) -> Result<()> {
        let mut inode = self.live_inode(oid)?;
        let new = match acl {
            Some(a) => self.acl_attach(a)?,
            None => 0,
        };
        let old = inode.acl;
        if old != 0 {
            self.acl_release(old)?;
        }
        inode.acl = new as u32;
        inode.ctime = self.now;
        self.put_inode(oid, &inode)?;
        self.jlog_attr(oid, crate::records::jattr::ACL)?;
        self.zil_op(zil::Op::SetAcl { oid, acl: acl.cloned(), time: self.now });
        Ok(())
    }

    /// Zmeni vlastnika objektu. Smi jen system (predani vlastnictvi by
    /// jinak slo zneuzit k obejiti kvot a auditu).
    pub fn set_owner(&mut self, path: &str, owner: u64) -> Result<()> {
        self.require_system()?;
        self.guarded(|fs| {
            let oid = fs.enter(path)?;
            fs.s.check_writable()?;
            fs.ensure_room(4)?;
            fs.set_owner_oid(oid, owner)
        })
    }

    pub(super) fn set_owner_oid(&mut self, oid: u64, owner: u64) -> Result<()> {
        let mut inode = self.live_inode(oid)?;
        inode.owner = owner;
        inode.ctime = self.now;
        self.put_inode(oid, &inode)?;
        self.jlog_attr(oid, crate::records::jattr::OWNER)?;
        self.zil_op(zil::Op::SetOwner { oid, owner, time: self.now });
        Ok(())
    }

    /// Najde stejny popis (index podle otisku) nebo zalozi novy; zvysi
    /// pocet odkazu. Vraci cislo popisu.
    fn acl_attach(&mut self, acl: &Acl) -> Result<u64> {
        let root = self.s.root;
        let h = acl_hash(acl);
        let ikey = Key::new(META_OID, item::ACL_INDEX, h);
        let mut ids = match btree::get(&mut self.s, &root, &ikey)? {
            Some(v) => decode_ids(&v)?,
            None => Vec::new(),
        };
        for &id in &ids {
            let key = Key::new(META_OID, item::ACL, id);
            let raw = btree::get(&mut self.s, &root, &key)?.ok_or(Error::Corrupted)?;
            let (a, refs) = decode_acl(&raw)?;
            if a == *acl {
                self.put(key, encode_acl(&a, refs.checked_add(1).ok_or(Error::NoResources)?))?;
                return Ok(id);
            }
        }
        // Novy popis: o jedno vic nez nejvyssi existujici (popisu je malo -
        // stejne se sdileji). Cislo v zaznamu objektu ma 32 bitu.
        let last = btree::collect(
            &mut self.s,
            &root,
            &Key::new(META_OID, item::ACL, 0),
            &Key::new(META_OID, item::ACL + 1, 0),
        )?
        .last()
        .map_or(0, |(k, _)| k.offset);
        let id = last + 1;
        if id > u32::MAX as u64 {
            return Err(Error::NoResources);
        }
        self.put(Key::new(META_OID, item::ACL, id), encode_acl(acl, 1))?;
        ids.push(id);
        self.put(ikey, encode_ids(&ids))?;
        // Starsi ovladac by pri mazani objektu nesnizoval pocty odkazu.
        self.s.sb.features_ro_compat |= feature::RO_COMPAT_ACL;
        self.s.mark_changed();
        Ok(id)
    }

    /// Snizi pocet odkazu popisu; posledni odkaz popis smaze.
    pub(super) fn acl_release(&mut self, id: u32) -> Result<()> {
        let id = id as u64;
        let root = self.s.root;
        let key = Key::new(META_OID, item::ACL, id);
        let raw = btree::get(&mut self.s, &root, &key)?.ok_or(Error::Corrupted)?;
        let (a, refs) = decode_acl(&raw)?;
        if refs > 1 {
            return self.put(key, encode_acl(&a, refs - 1));
        }
        self.del(&key)?;
        let ikey = Key::new(META_OID, item::ACL_INDEX, acl_hash(&a));
        let ids: Vec<u64> = match btree::get(&mut self.s, &root, &ikey)? {
            Some(v) => decode_ids(&v)?.into_iter().filter(|&x| x != id).collect(),
            None => return Err(Error::Corrupted),
        };
        if ids.is_empty() {
            self.del(&ikey)?;
            Ok(())
        } else {
            self.put(ikey, encode_ids(&ids))
        }
    }

    /// Vlastnosti aktivniho datasetu.
    pub(super) fn ds_flags(&mut self) -> Result<u32> {
        let root = self.s.root;
        Ok(match btree::get(&mut self.s, &root, &Key::new(META_OID, item::PROPS, 0))? {
            Some(v) if v.len() >= 4 => u32::from_le_bytes(v[0..4].try_into().unwrap()),
            Some(_) => return Err(Error::Corrupted),
            None => 0,
        })
    }

    /// Zapne nebo vypne zakaz spousteni v datasetu `dataset`. Vyzaduje
    /// `MANAGE` ke koreni datasetu. Hned se commituje (nastaveni datasetu,
    /// jako jeho vytvoreni).
    pub fn set_noexec(&mut self, dataset: &str, on: bool) -> Result<()> {
        self.require(dataset, rights::MANAGE)?;
        self.enter_dataset(dataset)?;
        self.s.check_writable()?;
        self.guarded(|fs| {
            fs.ensure_room(4)?;
            let mut f = fs.ds_flags()?;
            if on {
                f |= ds_flags::NOEXEC;
            } else {
                f &= !ds_flags::NOEXEC;
            }
            fs.put(Key::new(META_OID, item::PROPS, 0), f.to_le_bytes().to_vec())?;
            fs.commit()
        })
    }

    /// Ma dataset `dataset` zakaz spousteni?
    pub fn noexec(&mut self, dataset: &str) -> Result<bool> {
        self.enter_dataset(dataset)?;
        Ok(self.ds_flags()? & ds_flags::NOEXEC != 0)
    }

    /// Necha ve vypisu adresare `w` jen polozky, ke kterym ma volajici
    /// nejake pravo (polozka datasetu: podle jeho korene). Aktivni dataset
    /// po navratu = dataset adresare.
    pub(super) fn visible(&mut self, root: &Child, below: &Inherited, list: Vec<DirEntry>) -> Result<Vec<DirEntry>> {
        if !self.checking() {
            return Ok(list);
        }
        let root = *root;
        let p = self.principal.clone().unwrap_or_default();
        let here = self.cur_ds;
        let mut out = Vec::with_capacity(list.len());
        let mut acls: BTreeMap<u32, Option<Acl>> = BTreeMap::new();
        for e in list {
            let (owner, acl) = if e.kind == Kind::Dataset && !self.dataset_exists(e.oid)? {
                // Hranice na zanikly dataset: jen zdedena prava slozky.
                (0, None)
            } else if e.kind == Kind::Dataset {
                self.activate(e.oid)?;
                let r = self.s.root;
                let i = self.inode(&r, ROOT_OID)?;
                let a = self.load_acl(&r, i.acl)?;
                self.activate(here)?;
                (i.owner, a)
            } else {
                let i = self.inode(&root, e.oid)?;
                if !acls.contains_key(&i.acl) {
                    let a = self.load_acl(&root, i.acl)?;
                    acls.insert(i.acl, a);
                }
                (i.owner, acls[&i.acl].clone())
            };
            if below.rights_of(&p, owner, acl.as_ref()) != 0 {
                out.push(e);
            }
        }
        Ok(out)
    }

    /// Granty pro objekty uvnitr snapshotu datasetu: ziva prava ke koreni
    /// datasetu (vyzaduje `READ`) a pak popisy ze snapshotu samotneho.
    pub(super) fn walk_snapshot(&mut self, dataset: &str, root: &Child, path: &str) -> Result<Walked> {
        let live = self.walk(dataset)?;
        if live.rights & rights::READ == 0 {
            return Err(Error::AccessDenied);
        }
        // Zdedene granty ke koreni datasetu (bez jeho vlastniho popisu, ten
        // se vezme ze snapshotu).
        let parent = match split_parent(dataset) {
            Ok((pp, _)) => self.walk(pp)?.below,
            Err(_) => Inherited::default(),
        };
        self.enter_dataset(dataset)?;
        self.walk_from(*root, ROOT_OID, parent, super::split_path(path), false)
    }
}

fn encode_ids(ids: &[u64]) -> Vec<u8> {
    ids.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn decode_ids(b: &[u8]) -> Result<Vec<u64>> {
    if b.is_empty() || b.len() % 8 != 0 {
        return Err(Error::Corrupted);
    }
    Ok(b.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect())
}
