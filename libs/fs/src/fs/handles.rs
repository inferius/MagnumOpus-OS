//! Otevrene soubory (handly), sirotci, sdileci rezimy a zamky rozsahu.
//!
//! **Handle** drzi objekt (dataset, oid, generace) - ne cestu. Otevreny
//! soubor proto prezije prejmenovani i smazani sveho jmena:
//!
//! - **Sirotek:** kdyz se smaze posledni jmeno objektu, ktery nekdo ma
//!   otevreny, objekt zustane (zaznam s rodicem 0 a polozka `ORPHAN`
//!   v datasetu); kdo ho ma otevreny, cte a zapisuje dal. S poslednim
//!   handlem zmizi i jeho data. Po padu se sirotci uklidi pri pripojeni.
//! - **Atomicke nahrazeni** (`rename_replace`): cil se nahradi jednou
//!   operaci; kdo mel puvodni cil otevreny, cte dal puvodni obsah, nove
//!   `open` dostane novy objekt.
//! - **Smazani pri zavreni** (`set_delete_on_close`): jmeno zmizi az se
//!   zavrenim posledniho handlu (Windows `FILE_FLAG_DELETE_ON_CLOSE`).
//!
//! **Sdileci rezim** (vrstva Windows `FILE_SHARE_*`) se kontroluje pri
//! otevreni: novy pristup musi byt sdileny vsemi otevrenymi handly a novy
//! handle musi sdilet jejich pristupy. Smazani a prejmenovani otevreneho
//! objektu vyzaduje, aby vsechny handly sdilely mazani.
//!
//! **Zamky rozsahu** (`LockFileEx`): sdileny nebo vyhradni, poradni nebo
//! povinny. Poradni zamek hlida jen jine zamky; povinny navic zapisy
//! (vyhradni i cteni) z jinych handlu a cest. Zamky patri handlu a zmizi
//! s jeho zavrenim.
//!
//! Handly jsou jen v pameti (po restartu sluzby neplati, viz navrh
//! protokolu v2).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use abi::{Error, Result};

use super::{DirEntry, Fs, Stat, split_parent};
use crate::btree;
use crate::device::BlockDevice;
use crate::layout::{Inode, KIND_DIR, KIND_SYMLINK, Key, META_OID, decode_dirents, item};
use crate::names::validate_name;

/// Cislo otevreneho souboru.
pub type Handle = u64;

/// Revize objektu: kterym objektem (dataset, oid, generace) a v jake verzi
/// obsahu je. Dve stejne revize = tentyz obsah.
///
/// Verze obsahu se nepise do intent logu: po padu a prehrani logu muze
/// byt jina nez pred padem - klient pak dostane `CONFLICT` (bezpecny smer,
/// nic se tise neprepise).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Revision {
    pub dataset: u64,
    pub oid: u64,
    pub generation: u64,
    pub version: u64,
}

/// Pozadovany pristup (bity).
pub mod access {
    pub const READ: u8 = 1;
    pub const WRITE: u8 = 2;
    /// Smi nastavit smazani pri zavreni.
    pub const DELETE: u8 = 4;
    /// Spusteni (nacteni programu); odmitne ho dataset se zakazem
    /// spousteni (`ds_flags::NOEXEC`).
    pub const EXECUTE: u8 = 8;
}

/// Co handle dovoli ostatnim (bity, vrstva Windows `FILE_SHARE_*`).
pub mod share {
    pub const READ: u8 = 1;
    pub const WRITE: u8 = 2;
    pub const DELETE: u8 = 4;
    /// Vse - vychozi pro nativni programy.
    pub const ALL: u8 = READ | WRITE | DELETE;
}

#[derive(Debug, Clone)]
pub(super) struct Open {
    ds: u64,
    pub(super) oid: u64,
    generation: u64,
    access: u8,
    share: u8,
    /// Granty pro potomky (vypis adresare pres handle skryje, k cemu
    /// otevirajici nema pravo); `None` = bez kontrol.
    view: Option<(crate::perm::Principal, crate::perm::Inherited)>,
    /// Ucinna prava otevirajiciho v okamziku `open` (pro `stat_h`).
    rights: u8,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct RangeLock {
    handle: Handle,
    start: u64,
    end: u64,
    exclusive: bool,
    mandatory: bool,
}

/// Stav otevrenych souboru (v `Fs`).
#[derive(Default)]
pub(super) struct Handles {
    opens: BTreeMap<Handle, Open>,
    next: Handle,
    /// (dataset, oid) -> zamky rozsahu
    locks: BTreeMap<(u64, u64), Vec<RangeLock>>,
    /// Objekty se smazanim pri zavreni.
    delete_pending: BTreeSet<(u64, u64)>,
}

impl Handles {
    fn on_object(&self, ds: u64, oid: u64) -> impl Iterator<Item = (&Handle, &Open)> {
        self.opens.iter().filter(move |(_, o)| o.ds == ds && o.oid == oid)
    }

    /// Prekryva rozsah `start..end` povinny zamek jineho handlu, ktery
    /// pristup brani (`write` = zapis, jinak cteni)?
    fn mandatory_conflict(&self, ds: u64, oid: u64, by: Option<Handle>, start: u64, end: u64, write: bool) -> bool {
        self.locks.get(&(ds, oid)).is_some_and(|list| {
            list.iter().any(|l| {
                l.mandatory && Some(l.handle) != by && l.start < end && start < l.end && (write || l.exclusive)
            })
        })
    }
}

impl<D: BlockDevice> Fs<D> {
    /// Otevre soubor nebo adresar `path` s pristupem `access` a sdilenim
    /// `share` (bity z `access` a `share`). Odkaz se nenasleduje.
    pub fn open(&mut self, path: &str, access_bits: u8, share_bits: u8) -> Result<Handle> {
        if access_bits & !(access::READ | access::WRITE | access::DELETE | access::EXECUTE) != 0 {
            return Err(Error::InvalidArgs);
        }
        let (oid, view) = if self.checking() {
            let w = self.walk(path)?;
            let mut need = 0;
            for (a, r) in [
                (access::READ, crate::perm::rights::READ),
                (access::WRITE, crate::perm::rights::WRITE),
                (access::DELETE, crate::perm::rights::DELETE),
                (access::EXECUTE, crate::perm::rights::EXECUTE),
            ] {
                if access_bits & a != 0 {
                    need |= r;
                }
            }
            if w.rights & need != need {
                return Err(Error::AccessDenied);
            }
            (w.oid, Some((self.principal.clone().unwrap_or_default(), w.below, w.rights)))
        } else {
            (self.enter(path)?, None)
        };
        let rights = view.as_ref().map_or(crate::perm::rights::ALL, |v| v.2);
        let view = view.map(|(p, b, _)| (p, b));
        if access_bits & access::EXECUTE != 0 && self.ds_flags()? & super::ds_flags::NOEXEC != 0 {
            return Err(Error::AccessDenied);
        }
        let root = self.s.root;
        let inode = self.inode(&root, oid)?;
        if inode.kind == KIND_SYMLINK {
            return Err(Error::WrongType);
        }
        if access_bits & (access::WRITE | access::EXECUTE) != 0 {
            if inode.kind == KIND_DIR {
                return Err(Error::IsADirectory);
            }
            self.s.check_writable()?;
        }
        let ds = self.cur_ds;
        for (_, o) in self.handles.on_object(ds, oid) {
            let denied = access_bits & !o.share != 0 || o.access & !share_bits != 0;
            if denied {
                return Err(Error::Locked);
            }
        }
        self.handles.next += 1;
        let h = self.handles.next;
        self.handles.opens.insert(
            h,
            Open {
                ds,
                oid,
                generation: inode.generation,
                access: access_bits,
                share: share_bits,
                view,
                rights,
            },
        );
        Ok(h)
    }

    /// Znovu otevre objekt podle **trvaleho handlu** (dataset, oid, generace)
    /// po restartu sluzby: klient drzi identitu objektu, nova instance ho
    /// otevre s pravy overenymi znovu. Objekt s cestou se otevre jako
    /// `open` (prava podle cesty; jiny objekt na ceste = `NOT_FOUND`).
    /// Sirotek (smazany, ale otevreny pred padem - uchovany s
    /// `keep_orphans`) nema cestu: smi ho otevrit jen vlastnik (a system).
    /// Vraci handle a cestu objektu ve FS (`None` = sirotek). Jina generace
    /// = objekt mezitim zanikl (`NOT_FOUND`).
    pub fn reopen(&mut self, dataset: u64, oid: u64, generation: u64, access_bits: u8, share_bits: u8) -> Result<(Handle, Option<alloc::string::String>)> {
        if !self.all_dataset_ids()?.contains(&dataset) {
            return Err(Error::NotFound);
        }
        self.activate(dataset)?;
        let root = self.s.root;
        let Some(raw) = btree::get(&mut self.s, &root, &Key::new(oid, item::INODE, 0))? else {
            return Err(Error::NotFound);
        };
        let inode = Inode::decode(&raw, self.s.time_format())?;
        if inode.generation != generation {
            return Err(Error::NotFound);
        }
        if let Some(rel) = self.path_of(oid)? {
            let base = self.dataset_path(dataset)?;
            let mut full = alloc::string::String::from(base.trim_end_matches('/'));
            if rel != "/" || full.is_empty() {
                full.push_str(&rel);
            }
            let h = self.open(&full, access_bits, share_bits)?;
            let same = self.handles.opens.get(&h).is_some_and(|o| o.ds == dataset && o.oid == oid);
            if !same {
                self.close(h)?;
                return Err(Error::NotFound);
            }
            return Ok((h, Some(full)));
        }
        // Sirotek: rodic 0 a polozka ORPHAN.
        self.activate(dataset)?;
        let root = self.s.root;
        let orphan = btree::get(&mut self.s, &root, &Key::new(META_OID, item::ORPHAN, oid))?.is_some();
        if inode.parent != 0 || !orphan || inode.kind == KIND_SYMLINK {
            return Err(Error::NotFound);
        }
        if access_bits & !(access::READ | access::WRITE | access::DELETE) != 0 {
            return Err(Error::InvalidArgs);
        }
        let view = if self.checking() {
            let p = self.principal.clone().unwrap_or_default();
            if p.user != inode.owner {
                return Err(Error::AccessDenied);
            }
            Some((p, crate::perm::Inherited::default()))
        } else {
            None
        };
        if access_bits & access::WRITE != 0 {
            self.s.check_writable()?;
        }
        for (_, o) in self.handles.on_object(dataset, oid) {
            if access_bits & !o.share != 0 || o.access & !share_bits != 0 {
                return Err(Error::Locked);
            }
        }
        self.handles.next += 1;
        let h = self.handles.next;
        self.handles.opens.insert(
            h,
            Open {
                ds: dataset,
                oid,
                generation,
                access: access_bits,
                share: share_bits,
                view,
                rights: crate::perm::rights::READ | crate::perm::rights::WRITE | crate::perm::rights::DELETE,
            },
        );
        Ok((h, None))
    }

    /// Cisla novych handlu zacnou nad `base` (nova instance sluzby po padu:
    /// cislo handlu stare instance tak nikdy neukaze na cizi soubor).
    pub fn set_handle_base(&mut self, base: u64) {
        self.handles.next = self.handles.next.max(base);
    }

    /// Zavre handle. Se zavrenim posledniho handlu sirotek zmizi (i data)
    /// a provede se smazani pri zavreni.
    pub fn close(&mut self, h: Handle) -> Result<()> {
        let o = self.handles.opens.remove(&h).ok_or(Error::BadHandle)?;
        if let Some(list) = self.handles.locks.get_mut(&(o.ds, o.oid)) {
            list.retain(|l| l.handle != h);
        }
        if self.handles.on_object(o.ds, o.oid).next().is_some() {
            return Ok(());
        }
        self.handles.locks.remove(&(o.ds, o.oid));
        let pending = self.handles.delete_pending.remove(&(o.ds, o.oid));
        self.activate(o.ds)?;
        let root = self.s.root;
        let Some(raw) = btree::get(&mut self.s, &root, &Key::new(o.oid, item::INODE, 0))? else {
            return Ok(());
        };
        let inode = Inode::decode(&raw, self.s.time_format())?;
        if inode.generation != o.generation || self.s.read_only {
            return Ok(());
        }
        if inode.parent == 0 && o.oid != crate::layout::ROOT_OID {
            return self.guarded(|fs| {
                fs.ensure_room_freeing(4)?;
                fs.destroy_orphan(o.oid)
            });
        }
        if pending {
            return self.guarded(|fs| {
                fs.ensure_room_freeing(4)?;
                fs.unlink_oid(o.oid, inode.parent)
            });
        }
        Ok(())
    }

    /// Aktivuje dataset handlu a vrati jeho objekt (s kontrolou generace).
    pub(super) fn handle_object(&mut self, h: Handle, need: u8) -> Result<(Open, Inode)> {
        let o = self.handles.opens.get(&h).ok_or(Error::BadHandle)?.clone();
        if o.access & need != need {
            return Err(Error::AccessDenied);
        }
        self.activate(o.ds)?;
        let root = self.s.root;
        let raw = btree::get(&mut self.s, &root, &Key::new(o.oid, item::INODE, 0))?.ok_or(Error::BadHandle)?;
        let inode = Inode::decode(&raw, self.s.time_format())?;
        if inode.generation != o.generation {
            return Err(Error::BadHandle);
        }
        Ok((o, inode))
    }

    pub fn read_h(&mut self, h: Handle, offset: u64, len: usize) -> Result<Vec<u8>> {
        let (o, _) = self.handle_object(h, access::READ)?;
        let end = offset.saturating_add(len as u64);
        if self.handles.mandatory_conflict(o.ds, o.oid, Some(h), offset, end, false) {
            return Err(Error::Locked);
        }
        let root = self.s.root;
        self.read_oid(&root, o.oid, offset, len, true)
    }

    pub fn write_h(&mut self, h: Handle, offset: u64, data: &[u8]) -> Result<()> {
        let (o, inode) = self.handle_object(h, access::WRITE)?;
        let end = offset.saturating_add(data.len() as u64);
        if self.handles.mandatory_conflict(o.ds, o.oid, Some(h), offset, end, true) {
            return Err(Error::Locked);
        }
        self.write_oid(o.oid, inode, offset, data)
    }

    pub fn set_len_h(&mut self, h: Handle, size: u64) -> Result<()> {
        let (o, inode) = self.handle_object(h, access::WRITE)?;
        if self.handles.mandatory_conflict(o.ds, o.oid, Some(h), size.min(inode.size), u64::MAX, true) {
            return Err(Error::Locked);
        }
        self.truncate_checked(o.oid, inode, size)
    }

    pub fn stat_h(&mut self, h: Handle) -> Result<Stat> {
        let (o, _) = self.handle_object(h, 0)?;
        let root = self.s.root;
        let mut st = self.stat_oid(&root, o.oid)?;
        if o.view.is_some() {
            st.rights = o.rights;
            self.limit_stat(&mut st);
        }
        Ok(st)
    }

    pub fn list_h(&mut self, h: Handle) -> Result<Vec<DirEntry>> {
        let (o, _) = self.handle_object(h, 0)?;
        let root = self.s.root;
        let list = self.list_oid(&root, o.oid)?;
        match &o.view {
            Some((p, below)) => {
                let saved = self.principal.replace(p.clone());
                let r = self.visible(&root, below, list);
                self.principal = saved;
                r
            }
            None => Ok(list),
        }
    }

    /// Smazani pri zavreni posledniho handlu (vyzaduje pristup DELETE
    /// a sdileni mazani vsemi handly).
    pub fn set_delete_on_close(&mut self, h: Handle, on: bool) -> Result<()> {
        let (o, _) = self.handle_object(h, access::DELETE)?;
        if on {
            self.check_delete_allowed(o.oid)?;
            self.handles.delete_pending.insert((o.ds, o.oid));
        } else {
            self.handles.delete_pending.remove(&(o.ds, o.oid));
        }
        Ok(())
    }

    /// Zamek rozsahu `start..start+len` (len 0 = do konce souboru).
    pub fn lock_range(&mut self, h: Handle, start: u64, len: u64, exclusive: bool, mandatory: bool) -> Result<()> {
        let (o, _) = self.handle_object(h, 0)?;
        let end = if len == 0 { u64::MAX } else { start.saturating_add(len) };
        let list = self.handles.locks.entry((o.ds, o.oid)).or_default();
        let conflict = list
            .iter()
            .any(|l| l.handle != h && l.start < end && start < l.end && (exclusive || l.exclusive));
        if conflict {
            return Err(Error::Locked);
        }
        list.push(RangeLock { handle: h, start, end, exclusive, mandatory });
        Ok(())
    }

    /// Uvolni zamek s presne timto rozsahem.
    pub fn unlock_range(&mut self, h: Handle, start: u64, len: u64) -> Result<()> {
        let o = self.handles.opens.get(&h).ok_or(Error::BadHandle)?.clone();
        let end = if len == 0 { u64::MAX } else { start.saturating_add(len) };
        let list = self.handles.locks.get_mut(&(o.ds, o.oid)).ok_or(Error::NotFound)?;
        let i = list
            .iter()
            .position(|l| l.handle == h && l.start == start && l.end == end)
            .ok_or(Error::NotFound)?;
        list.remove(i);
        Ok(())
    }

    /// Ma nekdo objekt aktivniho datasetu otevreny?
    pub(super) fn is_open(&self, oid: u64) -> bool {
        self.handles.on_object(self.cur_ds, oid).next().is_some()
    }

    /// Smazani nebo prejmenovani otevreneho objektu: vsechny handly musi
    /// sdilet mazani.
    pub(super) fn check_delete_allowed(&self, oid: u64) -> Result<()> {
        let denied = self.handles.on_object(self.cur_ds, oid).any(|(_, o)| o.share & share::DELETE == 0);
        if denied { Err(Error::Locked) } else { Ok(()) }
    }

    /// Operace pres cestu (bez handlu) nad otevrenym objektem: musi ji
    /// dovolit sdileni vsech handlu a zadny povinny zamek v rozsahu.
    pub(super) fn check_path_access(&self, oid: u64, access_bits: u8, start: u64, end: u64) -> Result<()> {
        let ds = self.cur_ds;
        let denied = self.handles.on_object(ds, oid).any(|(_, o)| access_bits & !o.share != 0);
        let write = access_bits & access::WRITE != 0;
        if denied || self.handles.mandatory_conflict(ds, oid, None, start, end, write) {
            return Err(Error::Locked);
        }
        Ok(())
    }

    /// Udela z objektu sirotka (posledni jmeno smazane, ale je otevreny).
    pub(super) fn make_orphan(&mut self, oid: u64, mut inode: Inode) -> Result<()> {
        inode.parent = 0;
        inode.ctime = self.now;
        self.put_inode(oid, &inode)?;
        self.put(Key::new(META_OID, item::ORPHAN, oid), Vec::new())
    }

    /// Smaze sirotka i s daty.
    pub(super) fn destroy_orphan(&mut self, oid: u64) -> Result<()> {
        let root = self.s.root;
        let inode = self.inode(&root, oid)?;
        if inode.kind != KIND_DIR {
            self.drop_content(oid)?;
        }
        if inode.acl != 0 {
            self.acl_release(inode.acl)?;
        }
        self.drop_extras(oid)?;
        self.del(&Key::new(oid, item::INODE, 0))?;
        self.del(&Key::new(META_OID, item::ORPHAN, oid))?;
        self.zil.forget(oid);
        Ok(())
    }

    /// Smaze jmeno objektu `oid` v adresari `parent` (smazani pri zavreni).
    fn unlink_oid(&mut self, oid: u64, parent: u64) -> Result<()> {
        let root = self.s.root;
        let items = btree::collect(
            &mut self.s,
            &root,
            &Key::new(parent, item::DIRENT, 0),
            &Key::new(parent, item::DIRENT + 1, 0),
        )?;
        for (_, v) in items {
            if let Some(d) = decode_dirents(&v)?.into_iter().find(|d| d.oid == oid) {
                return self.remove_at(parent, &d.name);
            }
        }
        Err(Error::NotFound)
    }

    /// Uklid sirotku po padu (pri pripojeni): nikdo uz je nema otevrene.
    pub(super) fn cleanup_orphans(&mut self) -> Result<()> {
        self.cleanup_orphans_where(false)
    }

    /// Uklidi sirotky, ktere nikdo nema otevrene (po restartu sluzby se
    /// `keep_orphans`: co klienti stihli znovu otevrit, zustane do zavreni).
    /// Jen system.
    pub fn cleanup_unopened_orphans(&mut self) -> Result<()> {
        self.require_system()?;
        self.s.check_writable()?;
        self.cleanup_orphans_where(true)
    }

    fn cleanup_orphans_where(&mut self, skip_open: bool) -> Result<()> {
        let mut found = false;
        let ids = self.all_dataset_ids()?;
        for id in ids {
            self.activate(id)?;
            let root = self.s.root;
            let items = btree::collect(
                &mut self.s,
                &root,
                &Key::new(META_OID, item::ORPHAN, 0),
                &Key::new(META_OID, item::ORPHAN + 1, 0),
            )?;
            for (k, _) in items {
                if skip_open && self.handles.on_object(id, k.offset).next().is_some() {
                    continue;
                }
                self.destroy_orphan(k.offset)?;
                found = true;
            }
        }
        self.activate(crate::layout::ROOT_DATASET)?;
        if found {
            self.commit()?;
        }
        Ok(())
    }

    /// Atomicke nahrazeni: presune `from` na `to` a existujici cil v teze
    /// operaci nahradi (soubor soubor, prazdny adresar adresar). Kdo mel
    /// puvodni cil otevreny, cte dal jeho obsah (sirotek).
    pub fn rename_replace(&mut self, from: &str, to: &str) -> Result<()> {
        if self.checking() {
            self.require(from, crate::perm::rights::DELETE)?;
            self.require_parent(to, crate::perm::rights::CREATE)?;
            match self.require(to, crate::perm::rights::DELETE) {
                Ok(_) | Err(Error::NotFound) => {}
                Err(e) => return Err(e),
            }
        }
        self.guarded(|fs| {
            let (fp, fname) = split_parent(from)?;
            let (tp, tname) = split_parent(to)?;
            validate_name(tname)?;
            let src = fs.dir_of(fp)?;
            let src_ds = fs.cur_ds;
            let dst = fs.dir_of(tp)?;
            if fs.cur_ds != src_ds {
                return Err(Error::NotSupported);
            }
            fs.ensure_room(12)?;
            let root = fs.s.root;
            let d = fs.lookup_child(&root, src, fname)?.ok_or(Error::NotFound)?;
            if let Some(t) = fs.lookup_child(&root, dst, tname)?
                && t.oid != d.oid
            {
                let src_dir = d.kind == KIND_DIR;
                let dst_dir = t.kind == KIND_DIR;
                if src_dir && !dst_dir {
                    return Err(Error::NotADirectory);
                }
                if !src_dir && dst_dir {
                    return Err(Error::IsADirectory);
                }
                fs.remove_at(dst, &t.name)?;
            }
            fs.rename_at(src, fname, dst, tname)
        })
    }

    /// Revize objektu na ceste (pro podminene ulozeni).
    pub fn revision(&mut self, path: &str) -> Result<Revision> {
        let oid = self.require(path, 0)?;
        let root = self.s.root;
        let i = self.inode(&root, oid)?;
        Ok(Revision { dataset: self.cur_ds, oid, generation: i.generation, version: i.version })
    }

    /// Revize objektu za handlem.
    /// Ma handle aspon pristup `need` (`access::*`)? (fsd `OPEN_STREAM`:
    /// proud smi jen to, co handle klienta.)
    pub fn check_access_h(&mut self, h: Handle, need: u8) -> Result<()> {
        self.handle_object(h, need).map(|_| ())
    }

    pub fn revision_h(&mut self, h: Handle) -> Result<Revision> {
        let (o, i) = self.handle_object(h, 0)?;
        Ok(Revision { dataset: o.ds, oid: o.oid, generation: i.generation, version: i.version })
    }

    /// **Podminene ulozeni:** jako `rename_replace`, ale jen kdyz cil `to`
    /// ma porad revizi `expected` (`None` = cil nesmi existovat). Jinak
    /// `CONFLICT` a nic se nezmeni. Vraci revizi noveho obsahu v `to`.
    ///
    /// Typicke pouziti: editor si pri otevreni zapamatuje revizi, novy
    /// obsah zapise do docasneho souboru a pak ho podminene prejmenuje na
    /// misto originalu - zmenu, kterou mezitim udelal nekdo jiny, tak tise
    /// neprepise.
    pub fn rename_replace_if(&mut self, from: &str, to: &str, expected: Option<Revision>) -> Result<Revision> {
        let current = match self.revision(to) {
            Ok(r) => Some(r),
            Err(Error::NotFound) => None,
            Err(e) => return Err(e),
        };
        if current != expected {
            return Err(Error::Conflict);
        }
        self.rename_replace(from, to)?;
        self.revision(to)
    }

    /// Jmena objektu pro hlaseni (testy, ladeni).
    pub fn open_handles(&self) -> Vec<(Handle, u64, u64)> {
        self.handles.opens.iter().map(|(h, o)| (*h, o.ds, o.oid)).collect()
    }
}
