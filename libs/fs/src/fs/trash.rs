//! Kos datasetu.
//!
//! Smazani do kose nic nekopiruje: objekt se **presune** do skryteho
//! adresare `TRASH_OID` (rodic 0, zadne jmeno ve strome) pod jmenem, ktere
//! je cislem zaznamu kose. Zaznam (`item::TRASH`) drzi puvodni rodic
//! a jmeno, cas a kdo mazal. Otevrene handly funguji dal (objekt se jen
//! presunul), data se uvolni az vysypanim (`trash_purge`, `trash_tick`).
//!
//! Bezne `remove` zustava trvale (kompilatory, docasne soubory); do kose
//! presouva jen `trash` (pruzkumnik, uzivatel).
//!
//! **Kdo vidi zaznam:** spravce datasetu (`MANAGE` ke koreni), kdo objekt
//! smazal a vlastnik objektu. Obnova potrebuje navic `CREATE` v cilove
//! slozce.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use abi::{Error, Result};

use super::{Fs, Kind, split_parent, zil};
use crate::btree;
use crate::device::BlockDevice;
use crate::layout::{Inode, KIND_DATASET, KIND_DIR, Key, META_OID, ROOT_OID, TRASH_OID, Timestamp, item, u64_at};
use crate::node::Child;
use crate::perm::rights;
use crate::records::{TrashRecord, jop};

/// Nejvic zaznamu, ktere jeden krok uklidu kose vysype.
const TICK_BATCH: usize = 64;
/// Pod timto podilem volneho mista (procenta) `trash_tick` vysypava
/// nejstarsi zaznamy i pred uplynutim doby.
const LOW_SPACE_PERCENT: u64 = 10;

/// Polozka kose ve vypisu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashEntry {
    /// Cislo zaznamu (pro obnovu a vysypani).
    pub id: u64,
    pub name: String,
    /// Puvodni slozka (cesta uvnitr datasetu); prazdna, kdyz uz neexistuje.
    pub original_dir: String,
    pub kind: Kind,
    pub oid: u64,
    pub time: Timestamp,
    /// Kdo objekt smazal.
    pub actor: u64,
    /// Velikost (u slozky souhrn podstromu).
    pub size: u64,
}

fn rec_key(id: u64) -> Key {
    Key::new(META_OID, item::TRASH, id)
}

impl<D: BlockDevice> Fs<D> {
    /// Adresar kose aktivniho datasetu (zalozi ho pri prvnim pouziti).
    fn ensure_trash_dir(&mut self) -> Result<()> {
        let root = self.s.root;
        if btree::get(&mut self.s, &root, &Key::new(TRASH_OID, item::INODE, 0))?.is_some() {
            return Ok(());
        }
        let dir = Inode::new(KIND_DIR, 0, self.now, self.s.txg);
        self.put_inode(TRASH_OID, &dir)
    }

    /// Dalsi cislo zaznamu kose (pocitadlo v `TRASH` s offsetem 0).
    pub(super) fn next_trash_id(&mut self) -> Result<u64> {
        let root = self.s.root;
        Ok(match btree::get(&mut self.s, &root, &rec_key(0))? {
            Some(raw) if raw.len() == 8 => u64_at(&raw, 0),
            Some(_) => return Err(Error::Corrupted),
            None => 1,
        })
    }

    fn trash_record(&mut self, id: u64) -> Result<TrashRecord> {
        if id == 0 {
            return Err(Error::NotFound);
        }
        let root = self.s.root;
        TrashRecord::decode(&btree::get(&mut self.s, &root, &rec_key(id))?.ok_or(Error::NotFound)?)
    }

    /// Presune `parent`/`name` do kose aktivniho datasetu jako zaznam `id`
    /// (i pri prehrani logu - cislo je z logu).
    pub(super) fn trash_at(&mut self, parent: u64, name: &str, id: u64) -> Result<()> {
        if parent == TRASH_OID || id == 0 {
            return Err(Error::InvalidArgs);
        }
        let root = self.s.root;
        let d = self.lookup_child(&root, parent, name)?.ok_or(Error::NotFound)?;
        if d.kind == KIND_DATASET {
            // Hranice v klonu: `NOT_SUPPORTED` jako kazda zmena hranice.
            if self.is_boundary(parent, &d)? {
                return Err(Error::NotSupported);
            }
            // Dataset ma vlastni historii a bloky: maze ho `delete_dataset`.
            return Err(Error::WrongType);
        }
        let inode = self.inode(&root, d.oid)?;
        self.ensure_trash_dir()?;
        let m = self.move_entry(parent, name, TRASH_OID, &id.to_string())?;
        let rec = TrashRecord {
            oid: d.oid,
            generation: inode.generation,
            parent,
            time: self.now,
            kind: inode.kind,
            actor: self.actor(),
            size: if inode.kind == KIND_DIR { inode.tree_bytes } else { inode.size },
            name: m.name.clone(),
        };
        self.put(rec_key(id), rec.encode())?;
        let next = id.max(self.next_trash_id()?.saturating_sub(1)) + 1;
        self.put(rec_key(0), next.to_le_bytes().to_vec())?;
        self.jappend(crate::records::JournalEntry {
            op: jop::TRASH,
            kind: inode.kind,
            oid: d.oid,
            generation: inode.generation,
            parent,
            name: m.name.clone(),
            size: rec.size,
            aux: id,
            ..Default::default()
        })?;
        self.zil_op(zil::Op::Trash { parent, name: m.name, id, time: self.now });
        Ok(())
    }

    /// Presune soubor nebo slozku do kose jeho datasetu (nic se
    /// nekopiruje). Vyzaduje `DELETE` jako `remove`. Vraci cislo zaznamu.
    pub fn trash(&mut self, path: &str) -> Result<u64> {
        if self.checking() {
            self.require(path, rights::DELETE)?;
        }
        self.guarded(|fs| {
            let (parent_path, name) = split_parent(path)?;
            let parent = fs.dir_of(parent_path)?;
            fs.ensure_room(8)?;
            let id = fs.next_trash_id()?;
            fs.trash_at(parent, name, id)?;
            Ok(id)
        })
    }

    /// Smi volajici videt zaznam kose (spravce, kdo mazal, vlastnik)?
    fn may_see(&mut self, manage: bool, rec: &TrashRecord) -> Result<bool> {
        if manage || !self.checking() {
            return Ok(true);
        }
        let me = self.actor();
        if rec.actor == me {
            return Ok(true);
        }
        let root = self.s.root;
        Ok(self.inode(&root, rec.oid)?.owner == me)
    }

    /// Ma volajici `MANAGE` ke koreni datasetu `dataset`? (aktivuje ho)
    fn manages(&mut self, dataset: &str) -> Result<bool> {
        let r = if self.checking() { self.walk(dataset)?.rights } else { rights::ALL };
        self.enter_dataset(dataset)?;
        Ok(r & rights::MANAGE != 0)
    }

    /// Obsah kose datasetu `dataset` (jen zaznamy, ktere volajici smi videt),
    /// od nejnovejsiho.
    pub fn trash_list(&mut self, dataset: &str) -> Result<Vec<TrashEntry>> {
        let manage = self.manages(dataset)?;
        let root = self.s.root;
        let items = btree::collect(&mut self.s, &root, &rec_key(1), &Key::new(META_OID, item::TRASH + 1, 0))?;
        let mut out = Vec::new();
        for (k, v) in items {
            let rec = TrashRecord::decode(&v)?;
            if !self.may_see(manage, &rec)? {
                continue;
            }
            let original_dir = self.path_of(rec.parent)?.unwrap_or_default();
            out.push(TrashEntry {
                id: k.offset,
                name: rec.name,
                original_dir,
                kind: Kind::from_raw(rec.kind),
                oid: rec.oid,
                time: rec.time,
                actor: rec.actor,
                size: rec.size,
            });
        }
        out.reverse();
        Ok(out)
    }

    /// Obnovi zaznam kose `id` datasetu `dataset` na puvodni misto, nebo do
    /// `dest` (cesta v zive verzi, ve stejnem datasetu). Puvodni slozka
    /// uz neni = `NOT_FOUND` (zvolit `dest`); jmeno obsazene =
    /// `ALREADY_EXISTS`. Vraci cestu obnoveneho objektu.
    pub fn trash_restore(&mut self, dataset: &str, id: u64, dest: Option<&str>) -> Result<String> {
        let manage = self.manages(dataset)?;
        let ds = self.cur_ds;
        let rec = self.trash_record(id)?;
        if !self.may_see(manage, &rec)? {
            return Err(Error::NotFound);
        }
        let target = match dest {
            Some(p) => String::from(p),
            None => {
                let dir = self.path_of(rec.parent)?.ok_or(Error::NotFound)?;
                let mut p = String::from(dataset.trim_end_matches('/'));
                p.push_str(dir.trim_end_matches('/'));
                p.push('/');
                p.push_str(&rec.name);
                p
            }
        };
        self.require_parent(&target, rights::CREATE)?;
        self.guarded(|fs| {
            let (parent_path, name) = split_parent(&target)?;
            crate::names::validate_name(name)?;
            let parent = fs.dir_of(parent_path)?;
            if fs.cur_ds != ds {
                return Err(Error::NotSupported);
            }
            fs.ensure_room(8)?;
            fs.restore_at(id, parent, name)
        })?;
        Ok(target)
    }

    /// Obnova zaznamu `id` do `dst`/`name` aktivniho datasetu.
    pub(super) fn restore_at(&mut self, id: u64, dst: u64, name: &str) -> Result<()> {
        let rec = self.trash_record(id)?;
        let root = self.s.root;
        if self.lookup_child(&root, dst, name)?.is_some() {
            return Err(Error::AlreadyExists);
        }
        // Cil nesmi lezet v kosi (adresar v kosi nema cestu ke koreni).
        let mut a = dst;
        for _ in 0..super::MAX_DEPTH {
            if a == ROOT_OID {
                break;
            }
            if a == TRASH_OID || a == 0 {
                return Err(Error::InvalidArgs);
            }
            a = self.inode(&root, a)?.parent;
        }
        let m = self.move_entry(TRASH_OID, &id.to_string(), dst, name)?;
        self.del(&rec_key(id))?;
        self.jappend(crate::records::JournalEntry {
            op: jop::RESTORE,
            kind: rec.kind,
            oid: rec.oid,
            generation: rec.generation,
            parent: rec.parent,
            new_parent: dst,
            name: rec.name.clone(),
            new_name: m.new_name.clone(),
            aux: id,
            ..Default::default()
        })?;
        self.zil_op(zil::Op::Restore { id, dst, name: m.new_name, time: self.now });
        Ok(())
    }

    /// Trvale smaze zaznam kose `id` (i s podstromem). Smi spravce datasetu
    /// a kdo objekt smazal.
    pub fn trash_purge(&mut self, dataset: &str, id: u64) -> Result<()> {
        let manage = self.manages(dataset)?;
        let rec = self.trash_record(id)?;
        if !manage && self.checking() && rec.actor != self.actor() {
            return Err(Error::AccessDenied);
        }
        self.guarded(|fs| {
            fs.ensure_room_freeing(16)?;
            fs.purge_at(id)
        })
    }

    /// Vysype zaznam `id` aktivniho datasetu: smaze cely podstrom (bez
    /// zaznamu do zurnalu po objektech - jeden zaznam `REMOVE`).
    pub(super) fn purge_at(&mut self, id: u64) -> Result<()> {
        let rec = self.trash_record(id)?;
        let name = id.to_string();
        // Pred prvni zmenou overit, ze podstrom neobsahuje dataset (ten se
        // maze `delete_dataset`) - operace musi byt vse, nebo nic.
        let mut stack = alloc::vec![rec.oid];
        let mut order = Vec::new();
        while let Some(dir) = stack.pop() {
            let root = self.s.root;
            if self.inode(&root, dir)?.kind != KIND_DIR {
                continue;
            }
            order.push(dir);
            for e in self.list_oid(&root, dir)? {
                match e.kind {
                    Kind::Dataset => return Err(Error::NotEmpty),
                    Kind::Dir => stack.push(e.oid),
                    _ => {}
                }
            }
        }
        // Od nejhlubsich adresaru: nejdriv obsah, pak adresar.
        for &dir in order.iter().rev() {
            let root = self.s.root;
            for e in self.list_oid(&root, dir)? {
                if e.kind != Kind::Dir {
                    let (d, _) = self.unlink_entry(dir, &e.name)?;
                    self.zil.forget(d.oid);
                }
            }
            // Podadresare jsou uz prazdne (zpracovane drive).
            let root = self.s.root;
            for e in self.list_oid(&root, dir)? {
                let (d, _) = self.unlink_entry(dir, &e.name)?;
                self.zil.forget(d.oid);
            }
        }
        let (d, inode) = self.unlink_entry(TRASH_OID, &name)?;
        self.zil.forget(d.oid);
        self.del(&rec_key(id))?;
        self.jlog(jop::REMOVE, inode.kind, d.oid, inode.generation, TRASH_OID, &name, rec.size)?;
        self.zil_op(zil::Op::Purge { id, time: self.now });
        Ok(())
    }

    /// Krok uklidu kose (sluzba ho vola periodicky): ve vsech datasetech
    /// vysype zaznamy starsi nez nastaveni datasetu (`trash_max_age`) a pri
    /// malem volnem miste (pod 10 %) i nejstarsi dalsi. Vraci pocet
    /// vysypanych (nejvic `TICK_BATCH`). Jen system.
    pub fn trash_tick(&mut self, now: Timestamp) -> Result<usize> {
        self.require_system()?;
        let mut done = 0;
        let saved = self.now;
        self.now = now;
        let r = self.trash_tick_inner(now, &mut done);
        self.now = saved;
        r.map(|()| done)
    }

    fn low_space(&self) -> bool {
        self.s.free_blocks() * 100 < self.s.sb.block_count * LOW_SPACE_PERCENT
    }

    fn trash_tick_inner(&mut self, now: Timestamp, done: &mut usize) -> Result<()> {
        for ds in self.all_dataset_ids()? {
            self.activate(ds)?;
            let limit = now.secs.saturating_sub(self.settings_active()?.trash_age().min(i64::MAX as u64) as i64);
            loop {
                if *done >= TICK_BATCH {
                    return Ok(());
                }
                let root = self.s.root;
                // Nejstarsi zaznam = nejnizsi cislo.
                let mut first = None;
                btree::range(&mut self.s, &root, &rec_key(1), &Key::new(META_OID, item::TRASH + 1, 0), &mut |k, v| {
                    first = Some((k.offset, v.to_vec()));
                    Ok(false)
                })?;
                let Some((id, raw)) = first else { break };
                let rec = TrashRecord::decode(&raw)?;
                if rec.time.secs > limit && !self.low_space() {
                    break;
                }
                self.guarded(|fs| {
                    fs.ensure_room_freeing(16)?;
                    fs.purge_at(id)
                })?;
                *done += 1;
            }
        }
        self.activate(crate::layout::ROOT_DATASET)
    }

    /// Cesta objektu `oid` v zive verzi aktivniho datasetu (uvnitr datasetu,
    /// "/" = koren). `None`, kdyz objekt neexistuje nebo nelezi pod
    /// korenem (v kosi, sirotek).
    pub(super) fn path_of(&mut self, oid: u64) -> Result<Option<String>> {
        let root = self.s.root;
        self.path_in(&root, oid)
    }

    /// Cesta objektu `oid` ve strome `root` aktivniho datasetu (zivy stav
    /// nebo snapshot); `None` jako u `path_of`.
    pub(super) fn path_in(&mut self, root: &Child, mut oid: u64) -> Result<Option<String>> {
        let mut parts: Vec<String> = Vec::new();
        for _ in 0..super::MAX_DEPTH {
            if oid == ROOT_OID {
                let mut p = String::new();
                for part in parts.iter().rev() {
                    p.push('/');
                    p.push_str(part);
                }
                if p.is_empty() {
                    p.push('/');
                }
                return Ok(Some(p));
            }
            let Some(raw) = btree::get(&mut self.s, root, &Key::new(oid, item::INODE, 0))? else {
                return Ok(None);
            };
            let parent = Inode::decode(&raw, self.s.time_format())?.parent;
            if parent == 0 || parent == TRASH_OID {
                return Ok(None);
            }
            parts.push(self.name_in(root, parent, oid)?);
            oid = parent;
        }
        Err(Error::Corrupted)
    }
}
