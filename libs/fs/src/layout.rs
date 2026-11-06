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
/// Uzel B+stromu zabira 4 souvisle bloky (16 KiB): delsi jmena, melci strom,
/// vic extentu v listu.
pub const NODE_BLOCKS: u64 = 4;
pub const NODE_SIZE: usize = BLOCK_SIZE * NODE_BLOCKS as usize;
pub const SB_FIRST: u64 = 1;
pub const SB_COUNT: u64 = 8;
/// Prvni blok, ktery muze alokator pouzit.
pub const DATA_START: u64 = SB_FIRST + SB_COUNT;
pub const MAGIC: [u8; 8] = *b"OSFS\0\0v3";
pub const VERSION: u32 = 3;

/// Vlastnosti svazku, ktere tento ovladac umi (bit = vlastnost).
/// *compat*: starsi ovladac smi svazek normalne pouzivat; *ro_compat*:
/// jen cist; *incompat*: nesmi vubec. Neznamy bit ro_compat nebo
/// incompat = pripojeni odmitnuto (`NOT_SUPPORTED`). Zatim zadne.
pub const FEATURES_RO_COMPAT: u64 = 0;
pub const FEATURES_INCOMPAT: u64 = 0;

/// Bloky uvolnene commitem `g` se znovu pouziji az v transakci `g + REUSE_DELAY`.
/// Poslednich `REUSE_DELAY` ulozenych stavu tak zustava nedotcenych a pripojeni
/// se k nim umi vratit, kdyz je nejnovejsi stav poskozeny (disk, ktery lze
/// o flush).
pub const REUSE_DELAY: u64 = 4;

/// Soubory do teto velikosti lezi primo ve strome (bez datovych bloku).
/// Musi se vejit do `node::MAX_VALUE`.
pub const INLINE_MAX: usize = 1984;
/// Nejvic bloku jednoho extentu (~2 MiB): polozka i se soucty kazdeho bloku
/// (24 + 496 * 16 = 7960 B) se vejde do `node::MAX_VALUE`; rezie metadat
/// dat ~0,39 % kapacity.
pub const MAX_EXTENT: u64 = 496;
const _: () = assert!(24 + MAX_EXTENT as usize * 16 <= crate::node::MAX_VALUE);
const _: () = assert!(INLINE_MAX <= crate::node::MAX_VALUE);

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
    /// Cil symbolickeho odkazu (text UTF-8), offset 0.
    pub const SYMLINK: u8 = 7;
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

/// Ukazatel na blok (40 B): kde lezi (zarizeni + blok), kolik bloku ma,
/// generace vzniku, kontrolni soucet obsahu a priznaky.
///
/// ```text
/// 0   adresa u64: blok (bity 0-47) | zarizeni svazku (bity 48-63)
/// 8   generace vzniku u64
/// 16  kontrolni soucet (16 B)
/// 32  delka v blocich u32 (uzel stromu = NODE_BLOCKS)
/// 36  priznaky u16 (PTR_COMPRESSED, PTR_ENCRYPTED, PTR_SHARED)
/// 38  rezerva u16
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BlockPtr {
    /// Cislo bloku na zarizeni `dev`.
    pub block: u64,
    pub birth: u64,
    pub csum: Csum,
    /// Pocet bloku (0 se cte jako 1).
    pub len: u32,
    /// Zarizeni svazku (zatim vzdy 0; pripraveno pro vic disku).
    pub dev: u16,
    pub flags: u16,
}

pub const PTR_SIZE: usize = 40;
/// Priznaky ukazatele (pouzije faze 3 a 4).
#[allow(dead_code, reason = "pouzije faze 3 (komprese, sdilene bloky)")]
pub const PTR_COMPRESSED: u16 = 1 << 0;
#[allow(dead_code, reason = "pouzije faze 4 (sifrovane trezory)")]
pub const PTR_ENCRYPTED: u16 = 1 << 1;
#[allow(dead_code, reason = "pouzije faze 3 (okamzite kopie, deduplikace)")]
pub const PTR_SHARED: u16 = 1 << 2;
const BLOCK_BITS: u64 = (1 << 48) - 1;

impl BlockPtr {
    /// Ukazatel na jeden blok zarizeni 0.
    pub fn single(block: u64, birth: u64, csum: Csum) -> Self {
        Self {
            block,
            birth,
            csum,
            len: 1,
            ..Self::default()
        }
    }

    pub fn put(&self, b: &mut [u8]) {
        let addr = (self.block & BLOCK_BITS) | (self.dev as u64) << 48;
        b[0..8].copy_from_slice(&addr.to_le_bytes());
        b[8..16].copy_from_slice(&self.birth.to_le_bytes());
        b[16..32].copy_from_slice(&self.csum.0);
        b[32..36].copy_from_slice(&self.len.to_le_bytes());
        b[36..38].copy_from_slice(&self.flags.to_le_bytes());
        b[38..40].fill(0);
    }

    pub fn get(b: &[u8]) -> Self {
        let addr = u64_at(b, 0);
        Self {
            block: addr & BLOCK_BITS,
            dev: (addr >> 48) as u16,
            birth: u64_at(b, 8),
            csum: csum_at(b, 16),
            len: u32_at(b, 32),
            flags: u16::from_le_bytes([b[36], b[37]]),
        }
    }
}

/// Kontrolni blok souboroveho systemu (4 KiB, vlastni soucet XXH3-128).
///
/// ```text
/// 0    magic "OSFS\0\0v3"          8    verze u32 (3)
/// 12   algoritmus u8                16   generace u64
/// 24   bloku u64                    32   velikost AG u64
/// 40   UUID svazku (16 B)           56   vlastnosti compat u64
/// 64   vlastnosti ro_compat u64     72   vlastnosti incompat u64
/// 80   koren stromu (BlockPtr 40 B) 120  index mista (BlockPtr 40 B)
/// 160  blok intent logu u64         168  seminko intent logu u64
/// 176  dalsi oid u64                184  generace posl. snapshotu u64
/// 192  dalsi snapshot u64           200  poradi deadlistu u64
/// 208  verze Unicode u32            212  zarizeni svazku u16, pocet u16
/// 216  jmeno svazku (32 B)          4080 soucet XXH3-128
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Superblock {
    pub generation: u64,
    pub block_count: u64,
    pub algo: Algo,
    /// Velikost alokacni skupiny (bloky).
    pub ag_blocks: u64,
    /// Jednoznacne ID svazku (vyber svazku, ochrana pred zamenou disku).
    pub uuid: [u8; 16],
    pub features_compat: u64,
    pub features_ro_compat: u64,
    pub features_incompat: u64,
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
    /// Toto zarizeni ve svazku a pocet zarizeni (zatim vzdy 0 a 1).
    pub device_index: u16,
    pub device_count: u16,
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
        b[40..56].copy_from_slice(&self.uuid);
        b[56..64].copy_from_slice(&self.features_compat.to_le_bytes());
        b[64..72].copy_from_slice(&self.features_ro_compat.to_le_bytes());
        b[72..80].copy_from_slice(&self.features_incompat.to_le_bytes());
        self.root.put(&mut b[80..120]);
        self.space.put(&mut b[120..160]);
        b[160..168].copy_from_slice(&self.zil_block.to_le_bytes());
        b[168..176].copy_from_slice(&self.zil_seed.to_le_bytes());
        b[176..184].copy_from_slice(&self.next_oid.to_le_bytes());
        b[184..192].copy_from_slice(&self.snapshot_generation.to_le_bytes());
        b[192..200].copy_from_slice(&self.next_snapshot.to_le_bytes());
        b[200..208].copy_from_slice(&self.dead_seq.to_le_bytes());
        b[208..212].copy_from_slice(&self.unicode_version.to_le_bytes());
        b[212..214].copy_from_slice(&self.device_index.to_le_bytes());
        b[214..216].copy_from_slice(&self.device_count.to_le_bytes());
        b[216..248].copy_from_slice(&self.label);
        let c = xxh3(&b[..BLOCK_SIZE - 16]);
        b[BLOCK_SIZE - 16..].copy_from_slice(&c.0);
        b
    }

    /// `NotFound` = v bloku neni superblok; `Corrupted` = poskozeny;
    /// `NotSupported` = jina verze nebo neznama vlastnost.
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
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&b[40..56]);
        let mut label = [0u8; 32];
        label.copy_from_slice(&b[216..248]);
        let sb = Self {
            algo: Algo::from_raw(b[12])?,
            generation: u64_at(b, 16),
            block_count: u64_at(b, 24),
            ag_blocks: u64_at(b, 32),
            uuid,
            features_compat: u64_at(b, 56),
            features_ro_compat: u64_at(b, 64),
            features_incompat: u64_at(b, 72),
            root: BlockPtr::get(&b[80..120]),
            space: BlockPtr::get(&b[120..160]),
            zil_block: u64_at(b, 160),
            zil_seed: u64_at(b, 168),
            next_oid: u64_at(b, 176),
            snapshot_generation: u64_at(b, 184),
            next_snapshot: u64_at(b, 192),
            dead_seq: u64_at(b, 200),
            unicode_version: u32_at(b, 208),
            device_index: u16::from_le_bytes([b[212], b[213]]),
            device_count: u16::from_le_bytes([b[214], b[215]]),
            label,
        };
        if sb.features_ro_compat & !FEATURES_RO_COMPAT != 0 || sb.features_incompat & !FEATURES_INCOMPAT != 0 {
            return Err(Error::NotSupported);
        }
        // Velikost AG: rozumny rozsah (jinak by pamet pro skupiny vybuchla).
        let ags = sb.block_count.checked_div(sb.ag_blocks).unwrap_or(u64::MAX);
        if sb.ag_blocks < 64 || ags > 1 << 20 || sb.block_count <= DATA_START || sb.device_count == 0 {
            return Err(Error::Corrupted);
        }
        Ok(sb)
    }
}

/// Zaznam objektu v3 (128 B): soubor, adresar nebo symbolicky odkaz.
///
/// ```text
/// 0   druh u8 (KIND_*)          1   priznaky u8 (zatim 0)
/// 4   popis opravneni u32 (0 = zdedeny; faze 1d)
/// 8   velikost u64 (soubor: bajty, symlink: delka cile)
/// 16  zmena obsahu (mtime)      24  zmena metadat (ctime)
/// 32  vytvoreni (btime)         40  posledni pristup (atime, 0 = nesleduje se)
/// 48  rodic u64 (0 u korene)    56  generace vzniku u64 (overeni handlu)
/// 64  souhrn podstromu: bajty   72  soubory   80  adresare (bez sebe)
/// 88  vlastnik u64 (0 = system) 96  pocet rozsirenych atributu u32
/// 100 rezerva (28 B, nuly)
/// ```
///
/// Casy jsou v nanosekundach z `Fs::set_time` (fsd: od startu systemu).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Inode {
    /// `KIND_FILE`, `KIND_DIR` nebo `KIND_SYMLINK`.
    pub kind: u8,
    pub flags: u8,
    /// Cislo popisu opravneni (0 = zdedit od rodice).
    pub acl: u32,
    pub size: u64,
    pub mtime: u64,
    pub ctime: u64,
    pub btime: u64,
    pub atime: u64,
    /// Rodicovsky adresar (0 u korene); kazdy objekt ma prave jednoho rodice.
    pub parent: u64,
    /// Generace (txg), ve ktere objekt vznikl; handle s jinou generaci
    /// patri smazanemu objektu se stejnym oid.
    pub generation: u64,
    /// Souhrn podstromu (u adresaru): soucet velikosti souboru (saturuje).
    pub tree_bytes: u64,
    /// Souhrn podstromu: pocet souboru a symlinku.
    pub tree_files: u64,
    /// Souhrn podstromu: pocet adresaru (bez sebe).
    pub tree_dirs: u64,
    pub owner: u64,
    pub xattrs: u32,
}

pub const KIND_FILE: u8 = 1;
pub const KIND_DIR: u8 = 2;
pub const KIND_SYMLINK: u8 = 3;
pub const INODE_SIZE: usize = 128;
/// Nejvetsi velikost souboru (2^63 - 1 B): velikosti a rozdily se tak vzdy
/// vejdou do `i64`.
pub const MAX_FILE_SIZE: u64 = i64::MAX as u64;
/// Nejdelsi cil symbolickeho odkazu v bajtech UTF-8.
pub const SYMLINK_MAX: usize = 4096;

impl Inode {
    /// Novy prazdny objekt druhu `kind` v adresari `parent`.
    pub fn new(kind: u8, parent: u64, now: u64, generation: u64) -> Self {
        Self { kind, parent, mtime: now, ctime: now, btime: now, generation, ..Self::default() }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut b = alloc::vec![0u8; INODE_SIZE];
        b[0] = self.kind;
        b[1] = self.flags;
        b[4..8].copy_from_slice(&self.acl.to_le_bytes());
        for (k, v) in [
            self.size,
            self.mtime,
            self.ctime,
            self.btime,
            self.atime,
            self.parent,
            self.generation,
            self.tree_bytes,
            self.tree_files,
            self.tree_dirs,
            self.owner,
        ]
        .into_iter()
        .enumerate()
        {
            b[8 + k * 8..16 + k * 8].copy_from_slice(&v.to_le_bytes());
        }
        b[96..100].copy_from_slice(&self.xattrs.to_le_bytes());
        b
    }

    /// Odmitne neznamy druh a velikost nad `MAX_FILE_SIZE`.
    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() != INODE_SIZE || !matches!(b[0], KIND_FILE | KIND_DIR | KIND_SYMLINK) {
            return Err(Error::Corrupted);
        }
        let i = Self {
            kind: b[0],
            flags: b[1],
            acl: u32_at(b, 4),
            size: u64_at(b, 8),
            mtime: u64_at(b, 16),
            ctime: u64_at(b, 24),
            btime: u64_at(b, 32),
            atime: u64_at(b, 40),
            parent: u64_at(b, 48),
            generation: u64_at(b, 56),
            tree_bytes: u64_at(b, 64),
            tree_files: u64_at(b, 72),
            tree_dirs: u64_at(b, 80),
            owner: u64_at(b, 88),
            xattrs: u32_at(b, 96),
        };
        let too_big = i.size > MAX_FILE_SIZE || (i.kind == KIND_SYMLINK && i.size as usize > SYMLINK_MAX);
        if too_big {
            return Err(Error::Corrupted);
        }
        Ok(i)
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
