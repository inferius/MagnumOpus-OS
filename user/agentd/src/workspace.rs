//! Pracovni prostor relace: dataset, kde agent zapisuje bez potvrzovani.
//!
//! Relace FS prostoru ma pripojeny jen dataset prostoru `P` (pod stejnym
//! jmenem), takze zapis jinam neprojde. Historie: zacatek relace = commit,
//! kazdy krok se zapisem = commit (`COMMIT_VERSION`), rozdil = `DIFF`
//! mezi snimky commitu, vraceni = obnova obsahu ze snimku (pridane soubory
//! do kose, nic se nemaze natrvalo).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agentproto::proto::{StepInfo, WsChange};
use fsproto::{FsClient, kind};
use libos::{Error, Result};

use crate::state::write_all;

pub struct Workspace {
    pub fs: FsClient,
    /// Koren datasetu (stejna cesta v relaci prostoru i v relaci cteni).
    pub path: String,
    session: u64,
    /// Snimek commitu zacatku relace.
    start: String,
    /// Kroky se zapisem (cislo kroku = poradi odpovedi modelu v relaci).
    pub steps: Vec<StepInfo>,
}

fn join(base: &str, rel: &str) -> String {
    let rel = rel.trim_start_matches('/');
    if rel.is_empty() { String::from(base) } else { format!("{}/{rel}", base.trim_end_matches('/')) }
}

impl Workspace {
    /// Zacatek relace: overi, ze `path` je koren datasetu, a udela commit.
    pub fn start(fs: FsClient, path: &str, session: u64) -> Result<Self> {
        // Koren datasetu mimo pripojene cesty relace = cesta neni dataset.
        let root = match fs.dataset_of(path) {
            Err(Error::NotFound) if fs.stat(path).is_ok() => return Err(Error::NotSupported),
            r => r?,
        };
        if root.trim_end_matches('/') != path.trim_end_matches('/') {
            // Prostor musi byt dataset (historie je vlastnost datasetu).
            return Err(Error::NotSupported);
        }
        let c = fs.commit_version(path, &format!("agent {session}: zacatek relace"))?;
        Ok(Self { fs, path: String::from(path), session, start: c.snapshot, steps: Vec::new() })
    }

    /// Commit kroku `step` (zapsal do prostoru nastroji `tools`).
    pub fn commit(&mut self, step: u32, tools: &str) -> Result<()> {
        let message = format!("agent {}: krok {step}: {tools}", self.session);
        let c = self.fs.commit_version(&self.path, &message)?;
        self.steps.push(StepInfo { step, snapshot: c.snapshot, message, time: c.time.secs });
        Ok(())
    }

    /// Snimek po kroku `step` (0 = zacatek relace).
    fn snapshot_of(&self, step: u32) -> Result<String> {
        if step == 0 {
            return Ok(self.start.clone());
        }
        self.steps.iter().find(|s| s.step == step).map(|s| s.snapshot.clone()).ok_or(Error::NotFound)
    }

    /// Kroky vcetne zacatku (krok 0).
    pub fn list(&self) -> Vec<StepInfo> {
        let mut v = alloc::vec![StepInfo { step: 0, snapshot: self.start.clone(), message: String::from("zacatek relace"), time: 0 }];
        v.extend(self.steps.iter().cloned());
        v
    }

    /// Zmeny kroku `step` (proti predchozimu commitu); 0 = vse od zacatku
    /// relace do zive verze.
    pub fn diff(&self, step: u32) -> Result<Vec<WsChange>> {
        let (from, to) = if step == 0 {
            (self.start.clone(), None)
        } else {
            let i = self.steps.iter().position(|s| s.step == step).ok_or(Error::NotFound)?;
            let prev = if i == 0 { self.start.clone() } else { self.steps[i - 1].snapshot.clone() };
            (prev, Some(self.steps[i].snapshot.clone()))
        };
        let changes = self.fs.diff(&self.path, &from, to.as_deref())?;
        Ok(changes
            .into_iter()
            .map(|c| WsChange { kind: c.kind, path: join(&self.path, &c.path), old_path: if c.old_path.is_empty() { String::new() } else { join(&self.path, &c.old_path) }, is_dir: c.is_dir })
            .collect())
    }

    /// Obnovi obsah souboru nebo slozky `rel` ze snimku `snap`.
    fn restore(&self, snap: &str, rel: &str, is_dir: bool) -> Result<()> {
        let full = join(&self.path, rel);
        if is_dir {
            return match self.fs.create(&full, kind::DIR) {
                Ok(()) | Err(Error::AlreadyExists) => Ok(()),
                Err(e) => Err(e),
            };
        }
        let data = self.fs.snapshot_read_file_of(&self.path, snap, rel)?;
        write_all(&self.fs, &full, &data)
    }

    /// Vrati prostor na stav po kroku `to` (0 = zacatek relace). Vraci
    /// pocet obnovenych polozek. Konci commitem "vraceno".
    pub fn revert(&mut self, to: u32) -> Result<u32> {
        let snap = self.snapshot_of(to)?;
        let mut changes = self.fs.diff(&self.path, &snap, None)?;
        // Koren datasetu sam neni zmena (DIFF ho muze hlasit u slozek).
        changes.retain(|c| !c.path.trim_matches('/').is_empty());
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        let ctx = |c: &fsproto::hist::Change| {
            let (k, p, d) = (c.kind, c.path.clone(), c.is_dir);
            move |e: Error| {
                libos::println!("agentd: vraceni: zmena {k} {p} (slozka {d}): {e}");
                e
            }
        };
        let mut n = 0u32;
        // 1. Presuny zpet (stara cesta zase existuje), pak obsah.
        for c in changes.iter().filter(|c| c.kind == 4 || c.kind == 5) {
            self.fs.rename(&join(&self.path, &c.path), &join(&self.path, &c.old_path)).map_err(ctx(c))?;
            if c.kind == 5 && !c.is_dir {
                self.restore(&snap, &c.old_path, false).map_err(ctx(c))?;
            }
            n += 1;
        }
        // 2. Smazane a zmenene: obsah ze snimku (slozky pred soubory diky razeni).
        for c in changes.iter().filter(|c| c.kind == 2 || c.kind == 3) {
            if c.kind == 3 && c.is_dir {
                continue;
            }
            self.restore(&snap, &c.path, c.is_dir).map_err(ctx(c))?;
            n += 1;
        }
        // 3. Pridane do kose (jen nejvyssi: obsah slozky jde do kose s ni).
        let mut trashed: Vec<&str> = Vec::new();
        for c in changes.iter().filter(|c| c.kind == 1) {
            if trashed.iter().any(|t| c.path.strip_prefix(t).is_some_and(|r| r.starts_with('/'))) {
                continue;
            }
            match self.fs.trash(&join(&self.path, &c.path)) {
                Ok(_) => {}
                // DIFF hlasi i polozky, ktere v zive verzi uz nejsou (kos).
                Err(Error::NotFound) => continue,
                Err(e) => return Err(ctx(c)(e)),
            }
            trashed.push(&c.path);
            n += 1;
        }
        let message = format!("agent {}: vraceno na krok {to}", self.session);
        let c = self.fs.commit_version(&self.path, &message)?;
        // Kroky po cili uz neplati (dalsi krok navaze na vraceny stav).
        self.steps.retain(|s| s.step <= to);
        if let Some(last) = self.steps.last_mut().filter(|_| to > 0) {
            last.snapshot = c.snapshot;
        } else if to == 0 {
            self.start = c.snapshot;
        }
        Ok(n)
    }
}
