//! Rozlozeni na disku (format v2).
//!
//! ```text
//! blok 0        nepouzity (misto pro tabulku oddilu / zavadec)
//! blok 1..=8    kruh superbloku: commit generace g jde do slotu 1 + g % 8
//! dal           uzly B+stromu, data, space logy, tabulka AG, intent log
//! ```
//!
//! Vsechny struktury krome superbloku a bloku intent logu se overuji
//! kontrolnim souctem ulozenym v ukazateli rodice ([`BlockPtr`]); vznika
//! Merkleuv strom se superblokem na vrcholu. Cela cisla jsou little-endian.

use alloc::string::String;
use alloc::vec::Vec;

use abi::{Error, Result};

use crate::hash::{Algo, Csum, xxh3};

pub const BLOCK_SIZE: usize = 4096;
pub const SB_FIRST: u64 = 1;
pub const SB_COUNT: u64 = 8;
/// Prvni blok, ktery muze alokator pouzit.
pub const DATA_START: u64 = SB_FIRST + SB_COUNT;
pub const MAGIC: [u8; 8] = *b"OSFS\0\0v2";
pub const VERSION: u32 = 2;

/// Bloky uvolnene commitem `g` se znovu pouziji az v transakci `g + REUSE_DELAY`.
/// Poslednich `REUSE_DELAY` ulozenych stavu tak zustava nedotcenych a pripojeni
/// se k nim umi vratit, kdyz je nejnovejsi stav poskozeny (disk, ktery lze
/// o flush).
pub const REUSE_DELAY: u64 = 4;

/// Soubory do teto velikosti lezi primo ve strome (bez datovych bloku).
/// Musi se vejit do `node::MAX_VALUE`.
pub const INLINE_MAX: usize = 1984;
/// Nejvic bloku jednoho extentu (polozka i s checksumy se vejde do listu
/// dvakrat: rezie metadat dat ~0,42 % kapacity).
pub const MAX_EXTENT: u64 = 120;

/// Pevna ID objektu.
pub const META_OID: u64 = 0;
pub const ROOT_OID: u64 = 1;
pub const FIRST_FREE_OID: u64 = 16;

/// Druhy polozek stromu (cast klice).
pub mod item {
    /// Zaznam objektu (`Inode`), offset 0.
    pub const INODE: u8 = 1;
    /// Polozky adresare, offset = hash slozeneho jmena.
    pub const DIRENT: u8 = 2;
    /// Data maleho souboru, offset 0.
    pub const INLINE: u8 = 3;
    /// Extent souboru, offset = prvni logicky blok.
    pub const EXTENT: u8 = 4;
    /// Snapshot (objekt META_OID), offset = cislo snapshotu.
    pub const SNAPSHOT: u8 = 5;
    /// Seznam mrtvych bloku (objekt META_OID), offset = seznam << 32 | poradi.
    pub const DEADLIST: u8 = 6;
}

/// Klic polozky stromu; razeni podle (oid, kind, offset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key {
    pub oid: u64,
    pub kind: u8,
    pub offset: u64,
}

impl Key {
    pub const fn new(oid: u64, kind: u8, offset: u64) -> Self {
        Self { oid, kind, offset }
    }

    pub const MIN: Key = Key::new(0, 0, 0);
}

pub fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

pub fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

pub fn csum_at(b: &[u8], o: usize) -> Csum {
    Csum(b[o..o + 16].try_into().unwrap())
}

/// Ukazatel na blok: kde lezi, generace vzniku a kontrolni soucet obsahu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BlockPtr {
    pub block: u64,
    pub birth: u64,
    pub csum: Csum,
}

pub const PTR_SIZE: usize = 32;

impl BlockPtr {
    pub fn put(&self, b: &mut [u8]) {
        b[0..8].copy_from_slice(&self.block.to_le_bytes());
        b[8..16].copy_from_slice(&self.birth.to_le_bytes());
        b[16..32].copy_from_slice(&self.csum.0);
    }

    pub fn get(b: &[u8]) -> Self {
        Self {
            block: u64_at(b, 0),
            birth: u64_at(b, 8),
            csum: csum_at(b, 16),
        }
    }
}

/// Kontrolni blok souboroveho systemu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Superblock {
    pub generation: u64,
    pub block_count: u64,
    pub algo: Algo,
    /// Velikost alokacni skupiny (bloky).
    pub ag_blocks: u64,
    /// Koren B+stromu.
    pub root: BlockPtr,
    /// Index tabulky alokacnich skupin (space logy).
    pub space: BlockPtr,
    /// Prvni blok intent logu teto epochy (predem alokovany) a seminko jeho
    /// kontrolnich souctu.
    pub zil_block: u64,
    pub zil_seed: u64,
    pub next_oid: u64,
    /// Generace posledniho snapshotu: bloky narozene do ni patri snapshotum.
    pub snapshot_generation: u64,
    pub next_snapshot: u64,
    /// Dalsi poradove cislo polozky seznamu mrtvych bloku zive verze.
    pub dead_seq: u64,
    /// Verze Unicode tabulek pro skladani jmen.
    pub unicode_version: u32,
    pub label: [u8; 32],
}

impl Superblock {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = alloc::vec![0u8; BLOCK_SIZE];
        b[0..8].copy_from_slice(&MAGIC);
        b[8..12].copy_from_slice(&VERSION.to_le_bytes());
        b[12] = self.algo as u8;
        b[16..24].copy_from_slice(&self.generation.to_le_bytes());
        b[24..32].copy_from_slice(&self.block_count.to_le_bytes());
        b[32..40].copy_from_slice(&self.ag_blocks.to_le_bytes());
        self.root.put(&mut b[40..72]);
        self.space.put(&mut b[72..104]);
        b[104..112].copy_from_slice(&self.zil_block.to_le_bytes());
        b[112..120].copy_from_slice(&self.zil_seed.to_le_bytes());
        b[120..128].copy_from_slice(&self.next_oid.to_le_bytes());
        b[128..136].copy_from_slice(&self.snapshot_generation.to_le_bytes());
        b[136..144].copy_from_slice(&self.next_snapshot.to_le_bytes());
        b[144..152].copy_from_slice(&self.dead_seq.to_le_bytes());
        b[152..156].copy_from_slice(&self.unicode_version.to_le_bytes());
        b[160..192].copy_from_slice(&self.label);
        let c = xxh3(&b[..BLOCK_SIZE - 16]);
        b[BLOCK_SIZE - 16..].copy_from_slice(&c.0);
        b
    }

    /// `NotFound` = v bloku neni superblok; `Corrupted` = poskozeny.
    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() != BLOCK_SIZE || b[0..8] != MAGIC {
            return Err(Error::NotFound);
        }
        if xxh3(&b[..BLOCK_SIZE - 16]) != csum_at(b, BLOCK_SIZE - 16) {
            return Err(Error::Corrupted);
        }
        if u32_at(b, 8) != VERSION {
            return Err(Error::NotSupported);
        }
        let mut label = [0u8; 32];
        label.copy_from_slice(&b[160..192]);
        let sb = Self {
            algo: Algo::from_raw(b[12])?,
            generation: u64_at(b, 16),
            block_count: u64_at(b, 24),
            ag_blocks: u64_at(b, 32),
            root: BlockPtr::get(&b[40..72]),
            space: BlockPtr::get(&b[72..104]),
            zil_block: u64_at(b, 104),
            zil_seed: u64_at(b, 112),
            next_oid: u64_at(b, 120),
            snapshot_generation: u64_at(b, 128),
            next_snapshot: u64_at(b, 136),
            dead_seq: u64_at(b, 144),
            unicode_version: u32_at(b, 152),
            label,
        };
        if sb.ag_blocks == 0 || sb.block_count <= DATA_START {
            return Err(Error::Corrupted);
        }
        Ok(sb)
    }
}

/// Zaznam objektu (souboru nebo adresare).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inode {
    /// 1 = soubor, 2 = adresar.
    pub kind: u8,
    pub size: u64,
    pub mtime: u64,
    pub ctime: u64,
    /// Rodicovsky adresar (0 u korene); kazdy objekt ma prave jednoho rodice.
    pub parent: u64,
    /// Souhrn podstromu (u adresaru): soucet velikosti souboru.
    pub tree_bytes: u64,
    /// Souhrn podstromu: pocet souboru.
    pub tree_files: u64,
    /// Souhrn podstromu: pocet adresaru (bez sebe).
    pub tree_dirs: u64,
}

pub const KIND_FILE: u8 = 1;
pub const KIND_DIR: u8 = 2;
pub const INODE_SIZE: usize = 64;

impl Inode {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = alloc::vec![0u8; INODE_SIZE];
        b[0] = self.kind;
        b[8..16].copy_from_slice(&self.size.to_le_bytes());
        b[16..24].copy_from_slice(&self.mtime.to_le_bytes());
        b[24..32].copy_from_slice(&self.ctime.to_le_bytes());
        b[32..40].copy_from_slice(&self.parent.to_le_bytes());
        b[40..48].copy_from_slice(&self.tree_bytes.to_le_bytes());
        b[48..56].copy_from_slice(&self.tree_files.to_le_bytes());
        b[56..64].copy_from_slice(&self.tree_dirs.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() != INODE_SIZE || !(b[0] == KIND_FILE || b[0] == KIND_DIR) {
            return Err(Error::Corrupted);
        }
        Ok(Self {
            kind: b[0],
            size: u64_at(b, 8),
            mtime: u64_at(b, 16),
            ctime: u64_at(b, 24),
            parent: u64_at(b, 32),
            tree_bytes: u64_at(b, 40),
            tree_files: u64_at(b, 48),
            tree_dirs: u64_at(b, 56),
        })
    }
}

/// Polozka adresare (v hodnote DIRENT muze byt vic polozek se stejnym
/// hashem jmena).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirent {
    pub oid: u64,
    pub kind: u8,
    /// Jmeno, jak ho zadal uzivatel (NFC).
    pub name: String,
}

pub fn encode_dirents(list: &[Dirent]) -> Vec<u8> {
    let mut b = Vec::new();
    for d in list {
        b.extend_from_slice(&d.oid.to_le_bytes());
        b.push(d.kind);
        b.extend_from_slice(&(d.name.len() as u16).to_le_bytes());
        b.extend_from_slice(d.name.as_bytes());
    }
    b
}

pub fn decode_dirents(mut b: &[u8]) -> Result<Vec<Dirent>> {
    let mut out = Vec::new();
    while !b.is_empty() {
        if b.len() < 11 {
            return Err(Error::Corrupted);
        }
        let oid = u64_at(b, 0);
        let kind = b[8];
        let len = u16::from_le_bytes([b[9], b[10]]) as usize;
        let name = b.get(11..11 + len).ok_or(Error::Corrupted)?;
        let name = core::str::from_utf8(name).map_err(|_| Error::Corrupted)?;
        out.push(Dirent {
            oid,
            kind,
            name: name.into(),
        });
        b = &b[11 + len..];
    }
    Ok(out)
}

/// Souvisly usek dat souboru: fyzicky zacatek, delka, generace vzniku
/// (rozhoduje, zda bloky po prepsani uvolnit, nebo patri snapshotu) a
/// kontrolni soucet kazdeho bloku (nahodne cteni overi jen svuj blok).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extent {
    pub phys: u64,
    pub birth: u64,
    pub csums: Vec<Csum>,
}

const EXTENT_HEADER: usize = 24;

impl Extent {
    pub fn len(&self) -> u64 {
        self.csums.len() as u64
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(EXTENT_HEADER + self.csums.len() * 16);
        b.extend_from_slice(&self.phys.to_le_bytes());
        b.extend_from_slice(&(self.csums.len() as u32).to_le_bytes());
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&self.birth.to_le_bytes());
        for c in &self.csums {
            b.extend_from_slice(&c.0);
        }
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < EXTENT_HEADER {
            return Err(Error::Corrupted);
        }
        let len = u32_at(b, 8) as usize;
        if len == 0 || len as u64 > MAX_EXTENT || b.len() != EXTENT_HEADER + len * 16 {
            return Err(Error::Corrupted);
        }
        let csums = (0..len)
            .map(|i| csum_at(b, EXTENT_HEADER + i * 16))
            .collect();
        Ok(Self {
            phys: u64_at(b, 0),
            birth: u64_at(b, 16),
            csums,
        })
    }
}

/// Snapshot: koren stromu v okamziku vytvoreni.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotRecord {
    pub root: BlockPtr,
    pub generation: u64,
    pub name: String,
}

impl SnapshotRecord {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = alloc::vec![0u8; PTR_SIZE + 8];
        self.root.put(&mut b[..PTR_SIZE]);
        b[PTR_SIZE..].copy_from_slice(&self.generation.to_le_bytes());
        b.extend_from_slice(self.name.as_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < PTR_SIZE + 8 {
            return Err(Error::Corrupted);
        }
        let name = core::str::from_utf8(&b[PTR_SIZE + 8..]).map_err(|_| Error::Corrupted)?;
        Ok(Self {
            root: BlockPtr::get(b),
            generation: u64_at(b, PTR_SIZE),
            name: name.into(),
        })
    }
}

/// Mrtvy usek: bloky, ktere zivy stav uz nepouziva, ale drzi je snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeadExtent {
    pub start: u64,
    pub len: u64,
    pub birth: u64,
}

const DEAD_SIZE: usize = 24;
/// Kolik mrtvych useku se vejde do jedne polozky.
pub const DEAD_PER_ITEM: usize = 80;

pub fn encode_dead(list: &[DeadExtent]) -> Vec<u8> {
    let mut b = Vec::with_capacity(list.len() * DEAD_SIZE);
    for d in list {
        b.extend_from_slice(&d.start.to_le_bytes());
        b.extend_from_slice(&d.len.to_le_bytes());
        b.extend_from_slice(&d.birth.to_le_bytes());
    }
    b
}

pub fn decode_dead(b: &[u8]) -> Result<Vec<DeadExtent>> {
    if !b.len().is_multiple_of(DEAD_SIZE) {
        return Err(Error::Corrupted);
    }
    Ok(b.as_chunks::<DEAD_SIZE>()
        .0
        .iter()
        .map(|c| DeadExtent {
            start: u64_at(c, 0),
            len: u64_at(c, 8),
            birth: u64_at(c, 16),
        })
        .collect())
}
