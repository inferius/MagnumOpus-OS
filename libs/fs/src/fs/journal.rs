//! Zurnal zmen datasetu: usporadany proud zmen (poradove cislo, operace,
//! objekt, kdo, cas) pro sledovani zmen, indexovani, synchronizaci
//! a vraceni operaci.
//!
//! - **Polozky stromu datasetu** (`item::JOURNAL`, offset = poradove cislo):
//!   zaznam se uklada atomicky se zmenou, ke ktere patri, a snapshot ho
//!   zachyti take. Stav (`item::JSTATE`) drzi dalsi a nejstarsi cislo.
//! - **Cislo se nikdy neopakuje** pro nikoho, kdo ho videl: ctenar dostane
//!   jen zaznamy do `published` - posledniho potvrzeneho (commit nebo
//!   fsync). Zmena potvrzena `fsync` se po padu prehraje z intent logu a jeji
//!   zaznamy vzniknou znovu se stejnymi cisly (stejne poradi, stejny cas
//!   i uzivatel - `Record::Actor` v logu). Nepotvrzene zmeny po padu zmizi
//!   i se svymi cisly - ty ale nikdo nevidel.
//! - **Zapisy obsahu se slucuji:** jeden zaznam `CONTENT` na objekt za commit
//!   nebo fsync (dostane cislo az na konci davky). Ctenar stejne cte
//!   aktualni obsah; zurnal pak neroste s poctem zapisu (databaze, logy).
//! - **Retence** (nastaveni datasetu): pri commitu se zahodi zaznamy nad
//!   limit poctu nebo starsi nez limit stari (nejvic `PRUNE_BATCH` za
//!   commit). Kdo zada starsi cislo, dostane `JOURNAL_GAP`.
//! - **Novy vychozi stav i s pozici:** `view` vytvori docasny snapshot
//!   a vrati cislo posledniho zaznamu, ktery obsahuje.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use abi::{Error, Result};

use super::{Fs, zil};
use crate::btree;
use crate::device::BlockDevice;
use crate::layout::{Key, META_OID, Timestamp, item};
use crate::records::{JournalEntry, JournalState, Settings, jop, snapkind};

/// Nejvic zaznamu zahozenych retenci za jeden commit (commit nesmi trvat
/// neomezene dlouho po zmene nastaveni).
const PRUNE_BATCH: u64 = 4096;
/// Nejvic zaznamu v jedne strance cteni.
pub const READ_MAX: usize = 1024;

/// Stav zurnalu jednoho datasetu v pameti.
#[derive(Default)]
pub(super) struct DsJournal {
    /// Stav nacteny ze stromu (jinak se nacte pri prvnim pouziti).
    loaded: bool,
    state: JournalState,
    /// Posledni potvrzene cislo (vidi ho ctenari).
    published: u64,
    /// Zmeneny obsah od posledniho commitu/fsync: oid -> kdo.
    content: BTreeMap<u64, u64>,
    /// Stav se zmenil (zapsat `JSTATE`).
    dirty: bool,
}

impl DsJournal {
    pub(super) fn note_content(&mut self, oid: u64, actor: u64) {
        self.content.insert(oid, actor);
    }

    /// Kdo naposledy zmenil obsah objektu (0 = system).
    pub(super) fn content_actor(&self, oid: u64) -> u64 {
        self.content.get(&oid).copied().unwrap_or(0)
    }

    pub(super) fn publish(&mut self) {
        if self.loaded {
            self.published = self.state.next - 1;
        }
    }
}

/// Stranka zurnalu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalPage {
    /// (poradove cislo, zaznam) vzestupne.
    pub entries: Vec<(u64, JournalEntry)>,
    /// Posledni potvrzene cislo v okamziku cteni (dalsi cteni od
    /// `entries.last + 1`, nebo od `last + 1`, kdyz je stranka prazdna).
    pub last: u64,
    /// Nejstarsi ulozene cislo.
    pub first: u64,
}

/// Konzistentni pohled na dataset: docasny snapshot a pozice v zurnalu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    /// Jmeno snapshotu (cist jako kazdy jiny snapshot; uvolnit `release_view`).
    pub snapshot: String,
    pub id: u64,
    /// Posledni zaznam zurnalu, ktery pohled obsahuje: odber navaze od +1.
    pub seq: u64,
}

impl<D: BlockDevice> Fs<D> {
    /// Kdo provadi operaci (pro zurnal): pri prehrani logu puvodni uzivatel.
    pub(super) fn actor(&self) -> u64 {
        self.actor_override
            .unwrap_or_else(|| self.principal.as_ref().map_or(crate::perm::SYSTEM, |p| p.user))
    }

    /// Operace do intent logu (s uzivatelem).
    pub(super) fn zil_op(&mut self, op: zil::Op) {
        let a = self.actor();
        self.zil.op(a, op);
    }

    /// Nacte stav zurnalu aktivniho datasetu (jednou).
    fn jload(&mut self) -> Result<()> {
        if self.jr.loaded {
            return Ok(());
        }
        let root = self.s.root;
        let st = match btree::get(&mut self.s, &root, &Key::new(META_OID, item::JSTATE, 0))? {
            Some(raw) => JournalState::decode(&raw)?,
            None => JournalState::default(),
        };
        self.jr = DsJournal { loaded: true, state: st, published: st.next - 1, ..DsJournal::default() };
        Ok(())
    }

    /// Prida zaznam do zurnalu aktivniho datasetu (cas, uzivatel doplni).
    pub(super) fn jappend(&mut self, mut e: JournalEntry) -> Result<()> {
        self.jload()?;
        e.time = self.now;
        e.actor = self.actor();
        let seq = self.jr.state.next;
        self.jr.state.next = seq.checked_add(1).ok_or(Error::NoResources)?;
        self.jr.dirty = true;
        self.put(Key::new(META_OID, item::JOURNAL, seq), e.encode())
    }

    /// Zaznam operace nad objektem (`op`, druh, oid, generace, rodic, jmeno).
    #[allow(clippy::too_many_arguments, reason = "zaznam ma tolik poli")]
    pub(super) fn jlog(&mut self, op: u8, kind: u8, oid: u64, generation: u64, parent: u64, name: &str, size: u64) -> Result<()> {
        self.jappend(JournalEntry {
            op,
            kind,
            oid,
            generation,
            parent,
            name: name.into(),
            size,
            ..JournalEntry::default()
        })
    }

    /// Presun nebo prejmenovani.
    #[allow(clippy::too_many_arguments, reason = "zaznam ma tolik poli")]
    pub(super) fn jlog_rename(&mut self, kind: u8, oid: u64, generation: u64, src: u64, name: &str, dst: u64, new_name: &str) -> Result<()> {
        self.jappend(JournalEntry {
            op: jop::RENAME,
            kind,
            oid,
            generation,
            parent: src,
            new_parent: dst,
            name: name.into(),
            new_name: new_name.into(),
            ..JournalEntry::default()
        })
    }

    /// Zmena metadat objektu (`jattr::*`).
    pub(super) fn jlog_attr(&mut self, oid: u64, what: u64) -> Result<()> {
        let root = self.s.root;
        let i = self.inode(&root, oid)?;
        self.jappend(JournalEntry {
            op: jop::ATTR,
            kind: i.kind,
            oid,
            generation: i.generation,
            parent: i.parent,
            size: i.size,
            aux: what,
            ..JournalEntry::default()
        })
    }

    /// Zaznamena zmenu obsahu (zaznam vznikne pri `jflush`).
    pub(super) fn jnote_content(&mut self, oid: u64) {
        let a = self.actor();
        self.jr.note_content(oid, a);
    }

    /// Zapise sloucene zaznamy `CONTENT` a stav zurnalu aktivniho datasetu
    /// do stromu (pred commitem a fsync, po kazde prehrane davce logu).
    pub(super) fn jflush(&mut self) -> Result<()> {
        if !self.jr.loaded && self.jr.content.is_empty() {
            return Ok(());
        }
        let content = core::mem::take(&mut self.jr.content);
        let back = self.actor_override;
        for (oid, actor) in content {
            let root = self.s.root;
            let Some(raw) = btree::get(&mut self.s, &root, &Key::new(oid, item::INODE, 0))? else {
                // Objekt mezitim zanikl (smazany, sirotek uklizeny).
                continue;
            };
            let i = crate::layout::Inode::decode(&raw, self.s.time_format())?;
            // Sirotek (smazany, ale otevreny): do jmenneho prostoru uz
            // nepatri; po prehrani logu by ani neexistoval.
            if i.parent == 0 && oid != crate::layout::ROOT_OID {
                continue;
            }
            self.actor_override = Some(actor);
            let r = self.jappend(JournalEntry {
                op: jop::CONTENT,
                kind: i.kind,
                oid,
                generation: i.generation,
                parent: i.parent,
                size: i.size,
                ..JournalEntry::default()
            });
            self.actor_override = back;
            r?;
        }
        if self.jr.dirty {
            let st = self.jr.state;
            self.put(Key::new(META_OID, item::JSTATE, 0), st.encode())?;
            self.jr.dirty = false;
        }
        Ok(())
    }

    /// Po potvrzeni (commit, fsync): zaznamy vsech datasetu jsou trvale.
    pub(super) fn jpublish_all(&mut self) {
        self.jr.publish();
        for c in self.others.values_mut() {
            c.jr.publish();
        }
    }

    /// Retence: zahodi zaznamy nad limit poctu nebo stari (nejvic
    /// `PRUNE_BATCH` za commit). Jen kdyz zurnal v tomto commitu rostl.
    pub(super) fn jprune(&mut self) -> Result<()> {
        if !self.jr.loaded || !self.jr.dirty {
            return Ok(());
        }
        let set = self.settings_active()?;
        let max = set.journal_entries();
        let oldest = self.now.secs.saturating_sub(set.journal_age().min(i64::MAX as u64) as i64);
        let mut budget = PRUNE_BATCH;
        while budget > 0 && self.jr.state.first < self.jr.state.next {
            let first = self.jr.state.first;
            let over = self.jr.state.next - first > max;
            if !over {
                let root = self.s.root;
                let key = Key::new(META_OID, item::JOURNAL, first);
                let old = match btree::get(&mut self.s, &root, &key)? {
                    Some(raw) => JournalEntry::decode(&raw)?.time.secs < oldest,
                    None => true,
                };
                if !old {
                    break;
                }
            }
            self.del(&Key::new(META_OID, item::JOURNAL, first))?;
            self.jr.state.first = first + 1;
            budget -= 1;
        }
        Ok(())
    }

    /// Nastaveni aktivniho datasetu (chybi = vychozi).
    pub(super) fn settings_active(&mut self) -> Result<Settings> {
        let root = self.s.root;
        match btree::get(&mut self.s, &root, &Key::new(META_OID, item::SETTINGS, 0))? {
            Some(raw) => Settings::decode(&raw),
            None => Ok(Settings::default()),
        }
    }

    /// Nastaveni datasetu `dataset` (retence zurnalu a kose, pravidla
    /// automatickych snapshotu). Vyzaduje `READ` ke koreni datasetu.
    pub fn settings(&mut self, dataset: &str) -> Result<Settings> {
        self.require(dataset, crate::perm::rights::READ)?;
        self.enter_dataset(dataset)?;
        self.settings_active()
    }

    /// Zmeni nastaveni datasetu (vyzaduje `MANAGE`). Hned commituje.
    pub fn set_settings(&mut self, dataset: &str, s: &Settings) -> Result<()> {
        s.validate()?;
        self.require(dataset, crate::perm::rights::MANAGE)?;
        self.guarded(|fs| {
            fs.enter_dataset(dataset)?;
            fs.ensure_room(4)?;
            fs.put(Key::new(META_OID, item::SETTINGS, 0), s.encode())?;
            fs.commit()
        })
    }

    /// Precte zaznamy zurnalu datasetu `dataset` od cisla `from` (nejvic
    /// `max`, nejvic `READ_MAX`). Jen potvrzene zaznamy. Cislo starsi nez
    /// nejstarsi ulozene = `JOURNAL_GAP` (nikdy tiche vynechani).
    /// Vyzaduje `MANAGE` (zurnal ukazuje jmena vsech objektu).
    pub fn journal_read(&mut self, dataset: &str, from: u64, max: usize) -> Result<JournalPage> {
        self.require(dataset, crate::perm::rights::MANAGE)?;
        self.enter_dataset(dataset)?;
        self.jload()?;
        let (first, last) = (self.jr.state.first, self.jr.published);
        if from == 0 || from > last + 1 {
            return Err(Error::InvalidArgs);
        }
        if from < first {
            return Err(Error::JournalGap);
        }
        let n = max.clamp(1, READ_MAX) as u64;
        let to = last.min(from + n - 1);
        let mut entries = Vec::new();
        if from <= to {
            let root = self.s.root;
            for (k, v) in btree::collect(
                &mut self.s,
                &root,
                &Key::new(META_OID, item::JOURNAL, from),
                &Key::new(META_OID, item::JOURNAL, to + 1),
            )? {
                entries.push((k.offset, JournalEntry::decode(&v)?));
            }
        }
        Ok(JournalPage { entries, last, first })
    }

    /// Posledni potvrzene cislo zurnalu aktivniho datasetu.
    pub(super) fn journal_published(&mut self) -> Result<u64> {
        self.jload()?;
        Ok(self.jr.published)
    }

    /// Posledni cislo zurnalu aktivniho datasetu vcetne nepotvrzenych a zda
    /// cekaji sloucene zmeny obsahu.
    pub(super) fn journal_pending(&mut self) -> Result<(u64, bool)> {
        self.jload()?;
        Ok((self.jr.state.next - 1, !self.jr.content.is_empty()))
    }

    /// Posledni potvrzene cislo zurnalu datasetu (pro cekani na zmeny).
    pub fn journal_last(&mut self, dataset: &str) -> Result<u64> {
        self.require(dataset, crate::perm::rights::MANAGE)?;
        self.enter_dataset(dataset)?;
        self.jload()?;
        Ok(self.jr.published)
    }

    /// Konzistentni pohled na dataset: docasny snapshot (jen pro cteni)
    /// a cislo posledniho zaznamu zurnalu, ktery obsahuje. Klient (indexer)
    /// projde pohled a navaze odberem od `seq + 1` - nic nepreskoci, ani
    /// zmenu behem vypisu. Pohled se uvolni `release_view` (po padu se
    /// uklidi pri pripojeni). Vyzaduje `MANAGE`.
    pub fn view(&mut self, dataset: &str) -> Result<View> {
        self.require(dataset, crate::perm::rights::MANAGE)?;
        self.guarded(|fs| {
            fs.enter_dataset(dataset)?;
            let name = alloc::format!("view-{}", fs.s.next_snapshot);
            let (id, meta) = fs.snapshot_internal(&name, snapkind::VIEW)?;
            Ok(View { snapshot: name, id, seq: meta.journal_seq })
        })
    }

    /// Uvolni pohled (smaze docasny snapshot).
    pub fn release_view(&mut self, dataset: &str, snapshot: &str) -> Result<()> {
        self.require(dataset, crate::perm::rights::MANAGE)?;
        self.guarded(|fs| {
            fs.enter_dataset(dataset)?;
            let (id, _) = fs.find_snapshot(snapshot)?;
            if fs.snap_meta(id)?.kind != snapkind::VIEW {
                return Err(Error::WrongType);
            }
            fs.ensure_room_freeing(16)?;
            fs.delete_snapshot_active(snapshot)?;
            fs.commit()
        })
    }

    /// Pri pripojeni: pohledy z doby pred padem nikdo nedrzi - smazat.
    pub(super) fn cleanup_views(&mut self) -> Result<()> {
        let mut changed = false;
        for id in self.all_dataset_ids()? {
            self.activate(id)?;
            for (sid, rec) in self.records_active()? {
                if !rec.hidden() && self.snap_meta(sid)?.kind == snapkind::VIEW {
                    self.delete_snapshot_active(&rec.name)?;
                    changed = true;
                }
            }
        }
        self.activate(crate::layout::ROOT_DATASET)?;
        if changed {
            self.commit()?;
        }
        Ok(())
    }

    /// Cas jako `Timestamp` (pro testy a sluzby).
    pub fn now(&self) -> Timestamp {
        self.now
    }
}
