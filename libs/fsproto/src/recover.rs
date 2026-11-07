//! Obnova po padu fsd (protokol `docs/cs/obnova-sluzeb.md`).
//!
//! - **Druh operace** ([`kind_of`]): operace bez vedlejsiho ucinku
//!   (cteni, `stat`, vypis, `info`, stav) knihovna po padu zopakuje sama na
//!   nove instanci (`svcclient::Conn`); operace s ucinkem vrati
//!   `SERVICE_RESTARTED` a rozhodne aplikace.
//! - **Trvale handly**: aplikace dostava cislo handlu od knihovny, ne od
//!   sluzby. Knihovna si u nej drzi identitu objektu (dataset, oid,
//!   generace), pristup a sdileni. Po restartu sluzby (jina generace
//!   instance) ho pred dalsi operaci otevre znovu (`REOPEN`, prava se overi
//!   znovu). Cteni pres handle (`READ_H`, `STAT_H`, `LIST_H`) se pak
//!   zopakuje samo; zapis a ostatni operace s ucinkem vrati
//!   `SERVICE_RESTARTED`. Ztracene zamky a handle podmineneho ulozeni
//!   (docasny soubor patril stare instanci) se ohlasi jednou jako
//!   `SERVICE_RESTARTED`; handle podmineneho ulozeni pak uz neplati.

use alloc::collections::BTreeMap;

use libos::{Error, Result};
use svcclient::Kind;

use crate::v2::Revision;

/// Druh operace podle kodu (`Kind::Effect`, kdyz si nejsme jisti).
pub fn kind_of(op: u32) -> Kind {
    use crate::{data, hist, perm, v2};
    let idempotent = matches!(
        op,
        crate::op::INFO
            | crate::op::STAT
            | crate::op::LIST
            | crate::op::READ
            | crate::op::SNAPSHOTS
            | crate::op::SNAP_STAT
            | crate::op::SNAP_LIST
            | crate::op::SNAP_READ
            | crate::op::FRAGMENTS
            | crate::op::CHECK
            | crate::op::LIST_PAGE
            | crate::op::SNAP_LIST_PAGE
            | v2::op::HELLO
            | v2::op::CONNECT
            | perm::op::CONNECT_AS
            | perm::op::PERMISSIONS
            | perm::op::NOEXEC
            | hist::op::JOURNAL_READ
            | hist::op::JOURNAL_WATCH
            | hist::op::TRASH_LIST
            | hist::op::FILE_VERSIONS
            | hist::op::SETTINGS
            | hist::op::LOG
            | hist::op::BRANCHES
            | hist::op::ASIDE_CHANGES
            | hist::op::XATTR_GET
            | hist::op::XATTR_LIST
            | hist::op::EXPIRES
            | hist::op::HEAD
            | crate::op::LIST_STAT_PAGE
            | hist::op::DIFF
            | hist::op::DATASET_OF
            | hist::op::SNAP_STAT_OF
            | hist::op::SNAP_LIST_OF
            | hist::op::SNAP_READ_OF
            | data::op::DATA_SETTINGS
            | data::op::SCRUB_STATUS
    );
    if idempotent { Kind::Idempotent } else { Kind::Effect }
}

/// Handle u klienta.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Rec {
    /// Cislo handlu u sluzby (v instanci `generation`).
    pub fsd: u64,
    /// Generace spojeni (`Conn::generation`), ve ktere handle plati.
    pub generation: u64,
    pub revision: Revision,
    pub access: u8,
    pub share: u8,
    /// Handle docasneho souboru podmineneho ulozeni (po padu neplati).
    pub replace: bool,
    /// Drzi (drzel) zamek rozsahu.
    pub locked: bool,
    /// Smazat pri zavreni (po znovuotevreni se nastavi znovu).
    pub delete_on_close: bool,
    /// Po padu uz nejde obnovit (`BAD_HANDLE`).
    pub dead: bool,
}

/// Cisla handlu knihovny jsou jedinecna v celem procesu: handle jedne
/// relace v jine relaci neplati (`BAD_HANDLE`), ani nahodou.
static NEXT_HANDLE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);

/// Tabulka handlu klienta.
#[derive(Default)]
pub(crate) struct Table {
    recs: BTreeMap<u64, Rec>,
}

impl Table {
    pub fn insert(&mut self, rec: Rec) -> u64 {
        let h = NEXT_HANDLE.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        self.recs.insert(h, rec);
        h
    }

    pub fn get(&self, local: u64) -> Result<Rec> {
        self.recs.get(&local).copied().ok_or(Error::BadHandle)
    }

    pub fn update(&mut self, local: u64, f: impl FnOnce(&mut Rec)) {
        if let Some(r) = self.recs.get_mut(&local) {
            f(r);
        }
    }

    pub fn remove(&mut self, local: u64) -> Option<Rec> {
        self.recs.remove(&local)
    }
}
