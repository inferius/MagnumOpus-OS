//! Zamky souboru ve stylu POSIX (`fcntl` F_SETLK/F_GETLK, `flock`) pro
//! fsd: cista logika bez I/O (testovatelna na hostiteli).
//!
//! - **Vlastnik** je cislo (fsd: relace = proces u `fcntl`, handle
//!   u `flock`); vlastni zamky se nikdy nebiji, novy zamek vlastnika
//!   v rozsahu nahradi stary (zmena druhu, rozdeleni), sousedni stejneho
//!   druhu se slouci.
//! - **Rozsah** `[start, end)`, `end = u64::MAX` = do konce souboru (i za
//!   nim).
//! - **Cekani** si vede klient (opakovane zkouseni); tabulka si pamatuje,
//!   na koho vlastnik ceka ([`PosixLocks::note_wait`]), a odhali cyklus
//!   (`EDEADLK`). Zaznam o cekani vyprsi, kdyz ho klient neobnovi.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// Soubor: (dataset, oid) a trida (0 = `fcntl`, 1 = `flock`; navzajem se
/// neovlivnuji jako v Linuxu).
pub type FileKey = (u64, u64, u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Sdileny (`F_RDLCK`, `LOCK_SH`).
    Read,
    /// Vyhradni (`F_WRLCK`, `LOCK_EX`).
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lock {
    pub owner: u64,
    pub start: u64,
    pub end: u64,
    pub kind: Kind,
}

impl Lock {
    fn overlaps(&self, start: u64, end: u64) -> bool {
        self.start < end && start < self.end
    }

    fn conflicts(&self, owner: u64, start: u64, end: u64, kind: Kind) -> bool {
        self.owner != owner && self.overlaps(start, end) && (kind == Kind::Write || self.kind == Kind::Write)
    }
}

/// Vysledek pokusu o zamek.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetResult {
    Locked,
    /// Konflikt s cizim zamkem (vraci ho, `F_GETLK` ho ukaze).
    Conflict(Lock),
    /// Cekani by uzavrelo cyklus (`EDEADLK`).
    Deadlock,
}

/// Jak dlouho plati zaznam o cekani bez obnoveni (ns).
pub const WAIT_TTL: u64 = 1_000_000_000;

#[derive(Default)]
pub struct PosixLocks {
    files: BTreeMap<FileKey, Vec<Lock>>,
    /// Kdo ceka na koho: (cekajici) -> (drzitel, cas zaznamu).
    waits: BTreeMap<u64, (u64, u64)>,
}

/// Konec rozsahu z delky (`0` = do konce souboru).
pub fn range(start: u64, len: u64) -> (u64, u64) {
    if len == 0 { (start, u64::MAX) } else { (start, start.saturating_add(len)) }
}

impl PosixLocks {
    pub fn new() -> Self {
        Self::default()
    }

    /// Prvni cizi zamek v konfliktu (`F_GETLK`).
    pub fn test(&self, key: FileKey, owner: u64, start: u64, end: u64, kind: Kind) -> Option<Lock> {
        self.files.get(&key)?.iter().find(|l| l.conflicts(owner, start, end, kind)).copied()
    }

    /// Zamkne rozsah (`F_SETLK`); pri konfliktu nic nezmeni. S `wait`
    /// (`F_SETLKW`) zaznamena cekani a odhali cyklus; `now` v ns.
    pub fn set(&mut self, key: FileKey, owner: u64, start: u64, end: u64, kind: Kind, wait: bool, now: u64) -> SetResult {
        if start >= end {
            return SetResult::Locked;
        }
        if let Some(c) = self.test(key, owner, start, end, kind) {
            if wait {
                if self.would_cycle(owner, c.owner, now) {
                    self.waits.remove(&owner);
                    return SetResult::Deadlock;
                }
                self.waits.insert(owner, (c.owner, now));
            }
            return SetResult::Conflict(c);
        }
        self.waits.remove(&owner);
        self.remove(key, owner, start, end);
        let list = self.files.entry(key).or_default();
        list.push(Lock { owner, start, end, kind });
        Self::merge(list, owner);
        SetResult::Locked
    }

    /// Odemkne rozsah vlastnika (`F_UNLCK`); casti mimo rozsah zustanou.
    pub fn unlock(&mut self, key: FileKey, owner: u64, start: u64, end: u64) {
        self.remove(key, owner, start, end);
    }

    /// Vsechny zamky vlastnika na souboru pryc (zavreni posledniho handlu
    /// vlastnika na souboru).
    pub fn release_file(&mut self, key: FileKey, owner: u64) {
        self.remove(key, owner, 0, u64::MAX);
    }

    /// Vsechny zamky a cekani vlastnika pryc (konec relace).
    pub fn release_owner(&mut self, owner: u64) {
        for list in self.files.values_mut() {
            list.retain(|l| l.owner != owner);
        }
        self.files.retain(|_, l| !l.is_empty());
        self.waits.remove(&owner);
        self.waits.retain(|_, (holder, _)| *holder != owner);
    }

    /// Ma vlastnik nejaky zamek na souboru?
    pub fn holds(&self, key: FileKey, owner: u64) -> bool {
        self.files.get(&key).is_some_and(|l| l.iter().any(|x| x.owner == owner))
    }

    /// Zamky souboru (testy, ladeni).
    pub fn locks(&self, key: FileKey) -> &[Lock] {
        self.files.get(&key).map_or(&[], |v| v.as_slice())
    }

    /// Uzavrelo by cekani `owner` na `holder` cyklus? Sleduje platne
    /// zaznamy cekani od drzitele.
    fn would_cycle(&self, owner: u64, holder: u64, now: u64) -> bool {
        let mut at = holder;
        for _ in 0..=self.waits.len() {
            if at == owner {
                return true;
            }
            match self.waits.get(&at) {
                Some(&(next, t)) if now.saturating_sub(t) <= WAIT_TTL => at = next,
                _ => return false,
            }
        }
        false
    }

    fn remove(&mut self, key: FileKey, owner: u64, start: u64, end: u64) {
        let Some(list) = self.files.get_mut(&key) else { return };
        let mut out = Vec::with_capacity(list.len() + 1);
        for l in list.drain(..) {
            if l.owner != owner || !l.overlaps(start, end) {
                out.push(l);
                continue;
            }
            // Cast pred a za odemcenym rozsahem zustane.
            if l.start < start {
                out.push(Lock { end: start, ..l });
            }
            if l.end > end {
                out.push(Lock { start: end, ..l });
            }
        }
        if out.is_empty() {
            self.files.remove(&key);
        } else {
            *list = out;
        }
    }

    /// Slouci sousedni a prekryvajici se zamky vlastnika stejneho druhu.
    fn merge(list: &mut Vec<Lock>, owner: u64) {
        list.sort_by_key(|l| (l.owner, l.start));
        let mut out: Vec<Lock> = Vec::with_capacity(list.len());
        for l in list.drain(..) {
            if let Some(last) = out.last_mut()
                && last.owner == owner
                && l.owner == owner
                && last.kind == l.kind
                && l.start <= last.end
            {
                last.end = last.end.max(l.end);
                continue;
            }
            out.push(l);
        }
        *list = out;
    }
}
