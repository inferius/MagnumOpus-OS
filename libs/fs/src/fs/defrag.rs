//! Defragmentace a TRIM.
//!
//! Fragmentovany soubor se prepise souvisle: jeho bloky se nactou do bufferu
//! jako by byly zapsane znovu a odlozena alokace jim pri zapisu najde jeden
//! souvisly usek (best-fit). Diky copy-on-write je to bezpecne - dokud neni
//! commit, plati stara verze. Bloky sdilene se snapshoty se vynechavaji
//! (prepsanim by se zdvojily). Velke soubory se zpracuji po oknech, aby
//! nezabraly moc pameti; dalsi okno navazuje fyzicky za predchozim.

use alloc::vec::Vec;

use abi::{Error, Result};

use super::{DIRTY_LIMIT, Fs};
use crate::btree;
use crate::device::BlockDevice;
use crate::layout::{BLOCK_SIZE, INLINE_MAX, Inode, KIND_FILE, Key, item};

/// Vysledek kroku defragmentace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DefragReport {
    /// Prepsane soubory.
    pub files: u64,
    /// Prepsane bloky.
    pub blocks: u64,
    /// Pruchod vsemi soubory je dokonceny.
    pub done: bool,
}

/// Soubor stoji za defragmentaci, kdyz prumerny souvisly usek je kratsi nez
/// 4 MiB (1024 bloku) a usek je vic nez jeden.
fn worth(blocks: u64, runs: u64) -> bool {
    runs > 1 && blocks / runs < 1024
}

impl<D: BlockDevice> Fs<D> {
    /// Defragmentuje soubor; vraci pocet prepsanych bloku.
    pub fn defrag_file(&mut self, path: &str) -> Result<u64> {
        self.guarded(|fs| {
            let (oid, inode) = fs.file_oid(path)?;
            fs.defrag_oid(oid, &inode)
        })
    }

    fn defrag_oid(&mut self, oid: u64, inode: &Inode) -> Result<u64> {
        if inode.kind != KIND_FILE || inode.size as usize <= INLINE_MAX {
            return Ok(0);
        }
        self.writeback(oid, None)?;
        let root = self.s.root;
        let ext = self.extents_in(&root, oid, 0, u64::MAX)?;
        let blocks: u64 = ext.iter().map(|(_, e)| e.len()).sum();
        let mut runs = 0;
        let mut next = None;
        for (_, e) in &ext {
            if next != Some(e.phys) {
                runs += 1;
            }
            next = Some(e.phys + e.len());
        }
        if !worth(blocks, runs) {
            return Ok(0);
        }
        // Jen bloky, ktere nedrzi snapshot.
        let snap_gen = self.s.snapshot_generation;
        let eligible: Vec<_> = ext
            .into_iter()
            .filter(|(_, e)| e.birth > snap_gen)
            .collect();
        let window = DIRTY_LIMIT / 2;
        let mut rewritten = 0;
        let mut batch = 0;
        for (ks, e) in eligible {
            if !self.s.has_room(self.dirty_blocks + e.len() + 16) {
                break;
            }
            let mut buf = alloc::vec![0u8; e.len() as usize * BLOCK_SIZE];
            self.s.read_data(e.phys, &e.csums, &mut buf)?;
            let fd = self.data.entry(oid).or_default();
            for (j, block) in buf.chunks(BLOCK_SIZE).enumerate() {
                if fd.blocks.insert(ks + j as u64, block.to_vec()).is_none() {
                    self.dirty_blocks += 1;
                }
            }
            rewritten += e.len();
            batch += e.len();
            if batch >= window {
                self.writeback(oid, None)?;
                batch = 0;
            }
        }
        self.writeback(oid, None)?;
        Ok(rewritten)
    }

    /// Krok defragmentace na pozadi: projde dalsi soubory, dokud neprepise
    /// `budget` bloku. Volat opakovane (napr. kdyz je disk necinny); po
    /// dokonceni pruchodu zacne znovu od zacatku.
    pub fn defrag_step(&mut self, budget: u64) -> Result<DefragReport> {
        self.guarded(|fs| {
            let mut report = DefragReport::default();
            let root = fs.s.root;
            let mut todo: Vec<(u64, Inode)> = Vec::new();
            let start = Key::new(fs.defrag_cursor, item::INODE, 0);
            btree::range(
                &mut fs.s,
                &root,
                &start,
                &Key::new(u64::MAX, 0, 0),
                &mut |k, v| {
                    if k.kind == item::INODE {
                        todo.push((k.oid, Inode::decode(v)?));
                    }
                    Ok(todo.len() < 256)
                },
            )?;
            if todo.is_empty() {
                fs.defrag_cursor = 0;
                report.done = true;
                return Ok(report);
            }
            for (oid, inode) in todo {
                fs.defrag_cursor = oid + 1;
                let n = fs.defrag_oid(oid, &inode)?;
                if n > 0 {
                    report.files += 1;
                    report.blocks += n;
                }
                if report.blocks >= budget {
                    return Ok(report);
                }
            }
            Ok(report)
        })
    }

    /// Posle zarizeni TRIM pro vsechny volne bloky (jako `fstrim`); vraci
    /// pocet bloku. Bezne staci `set_discard(true)`, ktery posila TRIM
    /// prubezne.
    pub fn trim_all(&mut self) -> Result<u64> {
        let mut total = 0;
        for (s, l) in self.s.space.free_extents() {
            match self.s.dev.discard(s, l) {
                Ok(()) => total += l,
                Err(Error::NotSupported) => return Ok(0),
                Err(e) => return Err(e),
            }
        }
        Ok(total)
    }
}
