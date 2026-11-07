//! Rozsirene atributy objektu a expirace souboru.
//!
//! **Atributy** (`item::XATTR`, offset = hash jmena) jsou obecna data
//! ulozena atomicky s objektem: FS je jen uklada, nevyklada (nastaveni
//! sluzeb, `user.*` aplikaci). Jmenne prostory:
//! - `user.*` - aplikace a uzivatel (cteni `READ`, zmena `WRITE`),
//! - `sys.*` - vyznam zna FS nebo system; menit jde jen pres vlastni API
//!   (`set_expires`) nebo systemem.
//!
//! **Expirace** (`sys.expires`): po datu presune `expire_tick` objekt do
//! kose. Aby krok nemusel prochazet strom, ma dataset index podle casu
//! (`item::EXPIRY`: klic = cas, hodnota = seznam (oid, generace)); index
//! a atribut se meni vzdy spolu, fsck kontroluje shodu.

use alloc::string::String;
use alloc::vec::Vec;

use abi::{Error, Result};

use super::{Fs, zil};
use crate::btree;
use crate::device::BlockDevice;
use crate::layout::{Key, META_OID, TRASH_OID, Timestamp, item};
use crate::names::name_hash;
use crate::node::Child;
use crate::perm::rights;
use crate::records::{
    XATTR_EXPIRES, XATTR_VALUE_MAX, decode_expires, decode_pairs, decode_xattrs, encode_expires, encode_pairs,
    encode_xattrs, expiry_key, jattr, valid_xattr_name,
};

/// Nejvic objektu, ktere jeden krok expirace presune do kose.
const EXPIRE_BATCH: usize = 256;

fn xkey(oid: u64, name: &str) -> Key {
    Key::new(oid, item::XATTR, name_hash(name))
}

impl<D: BlockDevice> Fs<D> {
    /// Hodnota atributu objektu ve strome `root`.
    pub(super) fn xattr_in(&mut self, root: &Child, oid: u64, name: &str) -> Result<Option<Vec<u8>>> {
        let Some(raw) = btree::get(&mut self.s, root, &xkey(oid, name))? else {
            return Ok(None);
        };
        Ok(decode_xattrs(&raw)?.into_iter().find(|(n, _)| n == name).map(|(_, v)| v))
    }

    /// Vsechny atributy objektu ve strome `root` (serazene podle jmena).
    pub(super) fn xattrs_in(&mut self, root: &Child, oid: u64) -> Result<Vec<(String, Vec<u8>)>> {
        let mut out = Vec::new();
        for (_, v) in btree::collect(&mut self.s, root, &Key::new(oid, item::XATTR, 0), &Key::new(oid, item::XATTR + 1, 0))? {
            out.extend(decode_xattrs(&v)?);
        }
        out.sort();
        Ok(out)
    }

    /// Nastavi (`Some`) nebo smaze (`None`) atribut objektu `oid` aktivniho
    /// datasetu; udrzuje pocet atributu v zaznamu objektu a index expirace.
    /// Zurnal i intent log (i pri prehrani logu).
    pub(super) fn set_xattr_oid(&mut self, oid: u64, name: &str, value: Option<&[u8]>) -> Result<()> {
        if !valid_xattr_name(name) || value.is_some_and(|v| v.len() > XATTR_VALUE_MAX) {
            return Err(Error::InvalidArgs);
        }
        let expires = name == XATTR_EXPIRES;
        let new_exp = match (expires, value) {
            (true, Some(v)) => Some(decode_expires(v).map_err(|_| Error::InvalidArgs)?),
            _ => None,
        };
        let key = xkey(oid, name);
        let root = self.s.root;
        let mut list = match btree::get(&mut self.s, &root, &key)? {
            Some(raw) => decode_xattrs(&raw)?,
            None => Vec::new(),
        };
        let old = list.iter().position(|(n, _)| n == name).map(|i| list.remove(i).1);
        if old.is_none() && value.is_none() {
            return Err(Error::NotFound);
        }
        if let Some(v) = value {
            list.push((name.into(), v.to_vec()));
        }
        let encoded = encode_xattrs(&list);
        if encoded.len() > crate::node::MAX_VALUE {
            return Err(Error::NoResources);
        }
        let mut inode = self.live_inode(oid)?;
        let generation = inode.generation;
        if expires {
            if let Some(o) = &old {
                self.expiry_unindex(decode_expires(o)?, oid, generation)?;
            }
            if let Some(t) = new_exp {
                self.expiry_index(t, oid, generation)?;
            }
        }
        if list.is_empty() {
            self.del(&key)?;
        } else {
            self.put(key, encoded)?;
        }
        match (old.is_some(), value.is_some()) {
            (false, true) => inode.xattrs = inode.xattrs.saturating_add(1),
            (true, false) => inode.xattrs = inode.xattrs.saturating_sub(1),
            _ => {}
        }
        inode.ctime = self.now;
        self.put_inode(oid, &inode)?;
        self.jlog_attr(oid, jattr::XATTR)?;
        self.zil_op(zil::Op::SetXattr { oid, name: name.into(), value: value.map(<[u8]>::to_vec), time: self.now });
        Ok(())
    }

    fn expiry_index(&mut self, t: Timestamp, oid: u64, generation: u64) -> Result<()> {
        let key = Key::new(META_OID, item::EXPIRY, expiry_key(t));
        let root = self.s.root;
        let mut list = match btree::get(&mut self.s, &root, &key)? {
            Some(raw) => decode_pairs(&raw)?,
            None => Vec::new(),
        };
        if !list.contains(&(oid, generation)) {
            list.push((oid, generation));
        }
        // Mnoho objektu se stejnou sekundou expirace: polozka nesmi
        // prerust uzel.
        let v = encode_pairs(&list);
        if v.len() > crate::node::MAX_VALUE {
            return Err(Error::NoResources);
        }
        self.put(key, v)
    }

    fn expiry_unindex(&mut self, t: Timestamp, oid: u64, generation: u64) -> Result<()> {
        let key = Key::new(META_OID, item::EXPIRY, expiry_key(t));
        let root = self.s.root;
        let Some(raw) = btree::get(&mut self.s, &root, &key)? else {
            return Ok(());
        };
        let mut list = decode_pairs(&raw)?;
        list.retain(|&p| p != (oid, generation));
        if list.is_empty() {
            self.del(&key)?;
            Ok(())
        } else {
            self.put(key, encode_pairs(&list))
        }
    }

    /// Pri zaniku objektu: atributy (a polozka indexu expirace), pamet
    /// logickeho hashe.
    pub(super) fn drop_extras(&mut self, oid: u64) -> Result<()> {
        let root = self.s.root;
        if let Some(v) = self.xattr_in(&root, oid, XATTR_EXPIRES)? {
            let generation = self.inode(&root, oid)?.generation;
            self.expiry_unindex(decode_expires(&v)?, oid, generation)?;
        }
        let keys: Vec<Key> = btree::collect(&mut self.s, &root, &Key::new(oid, item::XATTR, 0), &Key::new(oid, item::XATTR + 1, 0))?
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        for k in keys {
            self.del(&k)?;
        }
        self.del(&Key::new(oid, item::LHASH, 0))?;
        Ok(())
    }

    /// Hodnota atributu `name` objektu `path` (`NOT_FOUND`, kdyz chybi).
    /// Vyzaduje `READ`.
    pub fn get_xattr(&mut self, path: &str, name: &str) -> Result<Vec<u8>> {
        let oid = self.require(path, rights::READ)?;
        let root = self.s.root;
        self.xattr_in(&root, oid, name)?.ok_or(Error::NotFound)
    }

    /// Jmena atributu objektu `path`. Vyzaduje `READ`.
    pub fn list_xattrs(&mut self, path: &str) -> Result<Vec<String>> {
        let oid = self.require(path, rights::READ)?;
        let root = self.s.root;
        Ok(self.xattrs_in(&root, oid)?.into_iter().map(|(n, _)| n).collect())
    }

    /// Nastavi atribut `user.*` (vyzaduje `WRITE`). Atributy `sys.*` meni
    /// jen system nebo vlastni API (`set_expires`).
    pub fn set_xattr(&mut self, path: &str, name: &str, value: &[u8]) -> Result<()> {
        self.change_xattr(path, name, Some(value))
    }

    /// Smaze atribut (`NOT_FOUND`, kdyz neni).
    pub fn remove_xattr(&mut self, path: &str, name: &str) -> Result<()> {
        self.change_xattr(path, name, None)
    }

    fn change_xattr(&mut self, path: &str, name: &str, value: Option<&[u8]>) -> Result<()> {
        if !valid_xattr_name(name) {
            return Err(Error::InvalidArgs);
        }
        if name.starts_with("sys.") && self.checking() {
            return Err(Error::AccessDenied);
        }
        let oid = self.require(path, rights::WRITE)?;
        self.guarded(|fs| {
            fs.s.check_writable()?;
            fs.ensure_room(4)?;
            fs.set_xattr_oid(oid, name, value)
        })
    }

    /// Nastavi (`Some`) nebo zrusi (`None`) datum expirace souboru nebo
    /// slozky: po nem ho `expire_tick` presune do kose. Vyzaduje `WRITE`.
    pub fn set_expires(&mut self, path: &str, when: Option<Timestamp>) -> Result<()> {
        let oid = self.require(path, rights::WRITE)?;
        if oid == crate::layout::ROOT_OID {
            return Err(Error::InvalidArgs);
        }
        self.guarded(|fs| {
            fs.s.check_writable()?;
            fs.ensure_room(4)?;
            let value = when.map(encode_expires);
            match fs.set_xattr_oid(oid, XATTR_EXPIRES, value.as_deref()) {
                Err(Error::NotFound) if when.is_none() => Ok(()),
                r => r,
            }
        })
    }

    /// Datum expirace objektu (`None` = bez expirace). Vyzaduje `READ`.
    pub fn expires(&mut self, path: &str) -> Result<Option<Timestamp>> {
        let oid = self.require(path, rights::READ)?;
        let root = self.s.root;
        self.xattr_in(&root, oid, XATTR_EXPIRES)?.map(|v| decode_expires(&v)).transpose()
    }

    /// Krok expirace (sluzba ho vola periodicky): objekty s datem expirace
    /// nejpozdeji `now` presune do kose a atribut smaze. Vraci pocet
    /// presunutych (nejvic `EXPIRE_BATCH`; zbytek pristi krok). Jen system.
    pub fn expire_tick(&mut self, now: Timestamp) -> Result<usize> {
        self.require_system()?;
        let mut moved = 0;
        for ds in self.all_dataset_ids()? {
            self.activate(ds)?;
            let root = self.s.root;
            let due = btree::collect(
                &mut self.s,
                &root,
                &Key::new(META_OID, item::EXPIRY, 0),
                &Key::new(META_OID, item::EXPIRY, expiry_key(now).saturating_add(1)),
            )?;
            for (_, v) in due {
                for (oid, generation) in decode_pairs(&v)? {
                    if moved >= EXPIRE_BATCH {
                        return Ok(moved);
                    }
                    let root = self.s.root;
                    let Some(raw) = btree::get(&mut self.s, &root, &Key::new(oid, item::INODE, 0))? else {
                        continue;
                    };
                    let inode = crate::layout::Inode::decode(&raw, self.s.time_format())?;
                    if inode.generation != generation {
                        continue;
                    }
                    let Some(exp) = self.xattr_in(&root, oid, XATTR_EXPIRES)? else { continue };
                    if decode_expires(&exp)? > now {
                        continue;
                    }
                    let saved = self.now;
                    self.now = now;
                    let r = self.guarded(|fs| {
                        fs.ensure_room(8)?;
                        fs.set_xattr_oid(oid, XATTR_EXPIRES, None)?;
                        // Uz v kosi (smazany rucne): jen zrusit expiraci.
                        if inode.parent != TRASH_OID && inode.parent != 0 {
                            let name = fs.name_of(inode.parent, oid)?;
                            let id = fs.next_trash_id()?;
                            fs.trash_at(inode.parent, &name, id)?;
                        }
                        Ok(())
                    });
                    self.now = saved;
                    r?;
                    moved += 1;
                }
            }
        }
        self.activate(crate::layout::ROOT_DATASET)?;
        Ok(moved)
    }

    /// Jmeno objektu `oid` v adresari `parent` zive verze.
    pub(super) fn name_of(&mut self, parent: u64, oid: u64) -> Result<String> {
        let root = self.s.root;
        self.name_in(&root, parent, oid)
    }

    /// Jmeno objektu `oid` v adresari `parent` stromu `root` (pruchod
    /// polozek adresare).
    pub(super) fn name_in(&mut self, root: &Child, parent: u64, oid: u64) -> Result<String> {
        for (_, v) in btree::collect(
            &mut self.s,
            root,
            &Key::new(parent, item::DIRENT, 0),
            &Key::new(parent, item::DIRENT + 1, 0),
        )? {
            if let Some(d) = crate::layout::decode_dirents(&v)?.into_iter().find(|d| d.oid == oid) {
                return Ok(d.name);
            }
        }
        Err(Error::NotFound)
    }
}
