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

/// Cas ve FS: sekundy od 1970 UTC (i64, se znamenkem) a nanosekundy (u32).
pub use abi::time::WallTime as Timestamp;

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
/// incompat = pripojeni odmitnuto (`NOT_SUPPORTED`).
pub const FEATURES_RO_COMPAT: u64 = feature::RO_COMPAT_ACL | feature::RO_COMPAT_HISTORY;
pub const FEATURES_INCOMPAT: u64 =
    feature::INCOMPAT_TIME_SECS | feature::INCOMPAT_PTR_COPY | feature::INCOMPAT_DATA;
/// Vlastnosti, ktere svazek mit musi: bez `INCOMPAT_PTR_COPY` ma stary tvar
/// ukazatele (40 B), ktery tento ovladac uz neumi (`NOT_SUPPORTED`,
/// svazek je potreba preformatovat).
pub const FEATURES_INCOMPAT_REQUIRED: u64 = feature::INCOMPAT_PTR_COPY;

/// Rezervovane bity vlastnosti (zatim je ovladac neumi, takze svazek s nimi
/// odmitne). Vyznam je pevny uz ted, aby se formaty nerozesly.
pub mod feature {
    /// incompat: svazek obsahuje placeholdery (data jen v cloudu, faze 6).
    #[allow(dead_code, reason = "pouzije faze 6 (cloud)")]
    pub const INCOMPAT_REMOTE: u64 = 1 << 0;
    /// incompat: casy jsou sekundy i64 + nanosekundy u32 (rozsah +/- 292
    /// miliard let). Bez nej (svazky z doby pred zmenou) jsou casy u64 ns
    /// od 1970 - ovladac je umi cist i zapisovat dal ve starem tvaru.
    pub const INCOMPAT_TIME_SECS: u64 = 1 << 1;
    /// incompat: ukazatel 48 B s adresou druhe kopie (metadata 2x,
    /// samoleceni; faze 3). Nastavuje se pri formatovani.
    pub const INCOMPAT_PTR_COPY: u64 = 1 << 2;
    /// incompat: extenty dat faze 3 - komprimovane kusy, sdilene bloky
    /// (tabulka sdileni, okamzite kopie, deduplikace), predalokovane
    /// (nezapsane) a s druhou kopii. Starsi ovladac by sdilene bloky pri
    /// mazani uvolnil a komprimovana data cetl jako surova.
    pub const INCOMPAT_DATA: u64 = 1 << 3;
    /// ro_compat: svazek ma popisy opravneni s pocty odkazu. Starsi ovladac
    /// smi jen cist (pri mazani by pocty nesnizoval).
    pub const RO_COMPAT_ACL: u64 = 1 << 0;
    /// ro_compat: svazek vede historii (faze 2): zurnal zmen, metadata
    /// snapshotu a commitu, kos, rozsirene atributy, index expirace, vetve.
    /// Starsi ovladac smi jen cist - pri zmene by zurnal ani indexy
    /// neaktualizoval.
    pub const RO_COMPAT_HISTORY: u64 = 1 << 1;
}

/// Tvar casu na svazku (podle `feature::INCOMPAT_TIME_SECS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeFormat {
    /// u64 nanosekund od 1970 (jen 1970 az 2262).
    LegacyNanos,
    /// i64 sekund + u32 nanosekund.
    Secs,
}

impl TimeFormat {
    pub fn of(features_incompat: u64) -> Self {
        if features_incompat & feature::INCOMPAT_TIME_SECS != 0 { Self::Secs } else { Self::LegacyNanos }
    }
}

/// Cas jako u64 ns od 1970 (stary tvar): pred 1970 -> 0, po 2262 -> max.
pub fn legacy_nanos(t: Timestamp) -> u64 {
    t.as_nanos().clamp(0, u64::MAX as i128) as u64
}

pub fn from_legacy_nanos(ns: u64) -> Timestamp {
    Timestamp { secs: (ns / 1_000_000_000) as i64, nanos: (ns % 1_000_000_000) as u32 }
}

/// Bloky uvolnene commitem `g` se znovu pouziji az v transakci `g + REUSE_DELAY`.
/// Poslednich `REUSE_DELAY` ulozenych stavu tak zustava nedotcenych a pripojeni
/// se k nim umi vratit, kdyz je nejnovejsi stav poskozeny (disk, ktery lze
/// o flush).
pub const REUSE_DELAY: u64 = 4;

/// Soubory do teto velikosti lezi primo ve strome (bez datovych bloku).
/// Musi se vejit do `node::MAX_VALUE`.
pub const INLINE_MAX: usize = 1984;
/// Nejvic bloku jednoho extentu (~2 MiB): polozka i se soucty kazdeho bloku
/// a adresou kopie (24 + 8 + 496 * 16 = 7968 B) se vejde do
/// `node::MAX_VALUE`; rezie metadat dat ~0,39 % kapacity.
pub const MAX_EXTENT: u64 = 496;
const _: () = assert!(32 + MAX_EXTENT as usize * 16 <= crate::node::MAX_VALUE);

mod extent;
pub use extent::*;
const _: () = assert!(INLINE_MAX <= crate::node::MAX_VALUE);

/// Pevna ID objektu.
pub const META_OID: u64 = 0;
pub const ROOT_OID: u64 = 1;
/// Kos datasetu: skryty adresar (rodic 0, zadne jmeno), do ktereho se
/// presouvaji smazane objekty (`Fs::trash`). Polozky se jmenuji cislem
/// zaznamu kose (`item::TRASH`).
pub const TRASH_OID: u64 = 2;
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
    /// Zaznam datasetu (`DatasetRecord`) - jen ve strome korenoveho
    /// datasetu, objekt META_OID, offset = cislo datasetu.
    pub const DATASET: u8 = 8;
    /// Sirotek: objekt bez jmena, ktery nekdo ma otevreny (objekt META_OID,
    /// offset = oid, hodnota prazdna). Po padu se pri pripojeni smaze.
    pub const ORPHAN: u8 = 9;
    /// Popis opravneni (`perm::encode_acl`), objekt META_OID, offset =
    /// cislo popisu (`Inode::acl`).
    pub const ACL: u8 = 10;
    /// Index popisu podle otisku (`perm::acl_hash`), objekt META_OID,
    /// hodnota = cisla popisu u64 se stejnym otiskem.
    pub const ACL_INDEX: u8 = 11;
    /// Vlastnosti datasetu (objekt META_OID, offset 0): priznaky u32
    /// (`fs::ds_flags`).
    pub const PROPS: u8 = 12;
    /// Zaznam zurnalu zmen (`records::JournalEntry`), objekt META_OID,
    /// offset = poradove cislo (od 1, nikdy se neopakuje).
    pub const JOURNAL: u8 = 13;
    /// Stav zurnalu (`records::JournalState`), objekt META_OID, offset 0.
    pub const JSTATE: u8 = 14;
    /// Metadata snapshotu (`records::SnapMeta`: druh, cas, pozice zurnalu,
    /// u commitu autor, zprava, rodice, logicke ID), offset = cislo snapshotu.
    pub const SNAPMETA: u8 = 15;
    /// Nastaveni datasetu (`records::Settings`: retence zurnalu a kose,
    /// pravidla automatickych snapshotu), objekt META_OID, offset 0.
    pub const SETTINGS: u8 = 16;
    /// Zaznam kose (`records::TrashRecord`), objekt META_OID, offset =
    /// cislo zaznamu (= jmeno polozky v adresari `TRASH_OID`).
    pub const TRASH: u8 = 17;
    /// Rozsirene atributy objektu (`records::encode_xattrs`), offset =
    /// hash jmena; vic atributu se stejnym hashem sdili hodnotu.
    pub const XATTR: u8 = 18;
    /// Index expirace, objekt META_OID, offset = cas (`records::expiry_key`),
    /// hodnota = seznam (oid, generace).
    pub const EXPIRY: u8 = 19;
    /// Pamet logickeho hashe obsahu souboru (`records::LogicalHash`), offset 0.
    pub const LHASH: u8 = 20;
    /// Vetve datasetu (`records::encode_branches`), objekt META_OID,
    /// offset = hash slozeneho jmena.
    pub const BRANCH: u8 = 21;
    /// Posledni commit datasetu a rozpracovany merge (`records::Head`),
    /// objekt META_OID, offset 0.
    pub const HEAD: u8 = 22;
    /// Kde je dataset pripojeny (`records::DsLink`: dataset rodice, slozka,
    /// jmeno) - ve strome datasetu samotneho, objekt META_OID, offset 0.
    /// Cesta datasetu se tak zjisti bez pruchodu celym stromem.
    pub const DSLINK: u8 = 23;
    /// Nastaveni dat datasetu (`datarec::DataSettings`: komprese, kopie,
    /// deduplikace), objekt META_OID, offset 0.
    pub const DATAOPT: u8 = 24;
    /// Tabulka sdileni bloku (`datarec::ShareEntry`) - jen ve strome
    /// korenoveho datasetu, objekt META_OID, offset = fyzicky zacatek useku.
    pub const SHARE: u8 = 25;
    /// Index deduplikace (`datarec::encode_dedup`) - jen ve strome
    /// korenoveho datasetu, objekt META_OID, offset = zacatek hashe.
    pub const DEDUP: u8 = 26;
    /// Stav scrubu (`datarec::ScrubState`) - jen ve strome korenoveho
    /// datasetu, objekt META_OID, offset 0.
    pub const SCRUB: u8 = 27;
    /// Neopravitelna chyba nalezena scrubem (text: cesta a popis), objekt
    /// META_OID, offset = poradi.
    pub const SCRUBERR: u8 = 28;
    /// Kde pokracuje deduplikace na pozadi v datasetu (generace u64),
    /// objekt META_OID, offset 0.
    pub const DEDUPPOS: u8 = 29;
}

/// Klic polozky stromu; razeni podle (oid, kind, offset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
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

/// Ukazatel na blok (48 B): kde lezi (zarizeni + blok), kde lezi druha
/// kopie, kolik bloku ma, generace vzniku, kontrolni soucet obsahu
/// a priznaky.
///
/// ```text
/// 0   adresa u64: blok (bity 0-47) | zarizeni svazku (bity 48-63)
/// 8   generace vzniku u64
/// 16  kontrolni soucet (16 B) - plati pro obe kopie
/// 32  delka v blocich u32 (uzel stromu = NODE_BLOCKS)
/// 36  priznaky u16 (PTR_COMPRESSED, PTR_ENCRYPTED, PTR_SHARED, PTR_REMOTE)
/// 38  rezerva u16
/// 40  adresa druhe kopie u64 (tvar jako adresa; 0 = bez kopie)
/// ```
///
/// Druha kopie (metadata 2x) lezi v jine alokacni skupine nez prvni, aby
/// ji lokalni poskozeni disku nezasahlo spolu s prvni. Kdyz prvni kopie
/// nesedi se souctem (nebo nejde precist), cte se druha a prvni se jeji
/// obsah prepise (samoleceni).
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
    /// Blok druhe kopie (0 = bez kopie) a jeji zarizeni.
    pub copy: u64,
    pub copy_dev: u16,
}

pub const PTR_SIZE: usize = 48;
/// Priznaky ukazatele (pouzije faze 3 a 4).
#[allow(dead_code, reason = "pouzije faze 3 (komprese, sdilene bloky)")]
pub const PTR_COMPRESSED: u16 = 1 << 0;
#[allow(dead_code, reason = "pouzije faze 4 (sifrovane trezory)")]
pub const PTR_ENCRYPTED: u16 = 1 << 1;
#[allow(dead_code, reason = "pouzije faze 3 (okamzite kopie, deduplikace)")]
pub const PTR_SHARED: u16 = 1 << 2;
#[allow(dead_code, reason = "pouzije faze 6 (soubory na vyzadani)")]
pub const PTR_REMOTE: u16 = 1 << 3;
const BLOCK_BITS: u64 = (1 << 48) - 1;
/// Nejvetsi velikost zarizeni v blocich (48 bitu adresy = 1 EiB).
pub const MAX_BLOCKS: u64 = 1 << 48;

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
        debug_assert!(self.block <= BLOCK_BITS, "cislo bloku nad 48 bitu");
        let addr = (self.block & BLOCK_BITS) | (self.dev as u64) << 48;
        b[0..8].copy_from_slice(&addr.to_le_bytes());
        b[8..16].copy_from_slice(&self.birth.to_le_bytes());
        b[16..32].copy_from_slice(&self.csum.0);
        b[32..36].copy_from_slice(&self.len.to_le_bytes());
        b[36..38].copy_from_slice(&self.flags.to_le_bytes());
        b[38..40].fill(0);
        debug_assert!(self.copy <= BLOCK_BITS, "cislo bloku kopie nad 48 bitu");
        let copy = (self.copy & BLOCK_BITS) | (self.copy_dev as u64) << 48;
        b[40..48].copy_from_slice(&copy.to_le_bytes());
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
            copy: u64_at(b, 40) & BLOCK_BITS,
            copy_dev: (u64_at(b, 40) >> 48) as u16,
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
/// 80   koren stromu (BlockPtr 48 B) 128  index mista (BlockPtr 48 B)
/// 176  blok intent logu u64         184  seminko intent logu u64
/// 192  dalsi oid u64                200  generace posl. snapshotu u64
/// 208  dalsi snapshot u64           216  poradi deadlistu u64
/// 224  verze Unicode u32            228  zarizeni svazku u16, pocet u16
/// 232  jmeno svazku (32 B)          4080 soucet XXH3-128
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
    /// Ma svazek vlastnost, kterou tento ovladac umi jen cist?
    pub fn ro_compat_unknown(&self) -> bool {
        self.features_ro_compat & !FEATURES_RO_COMPAT != 0
    }

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
        self.root.put(&mut b[80..128]);
        self.space.put(&mut b[128..176]);
        b[176..184].copy_from_slice(&self.zil_block.to_le_bytes());
        b[184..192].copy_from_slice(&self.zil_seed.to_le_bytes());
        b[192..200].copy_from_slice(&self.next_oid.to_le_bytes());
        b[200..208].copy_from_slice(&self.snapshot_generation.to_le_bytes());
        b[208..216].copy_from_slice(&self.next_snapshot.to_le_bytes());
        b[216..224].copy_from_slice(&self.dead_seq.to_le_bytes());
        b[224..228].copy_from_slice(&self.unicode_version.to_le_bytes());
        b[228..230].copy_from_slice(&self.device_index.to_le_bytes());
        b[230..232].copy_from_slice(&self.device_count.to_le_bytes());
        b[232..264].copy_from_slice(&self.label);
        let c = xxh3(&b[..BLOCK_SIZE - 16]);
        b[BLOCK_SIZE - 16..].copy_from_slice(&c.0);
        b
    }

    /// `NotFound` = v bloku neni superblok; `Corrupted` = poskozeny;
    /// `NotSupported` = jina verze nebo neznama vlastnost.
    /// Overi jen hlavicku: podpis rodiny formatu a vlastni soucet. Vraci
    /// (generace, verze). Slouzi k vyberu nejnovejsiho stavu *pred*
    /// rozhodnutim, zda ho tento ovladac umi otevrit - jinak by superblok
    /// novejsi verze nebo s neznamou vlastnosti byl tise preskocen a starsi
    /// ovladac by zapisoval do stareho stavu.
    pub fn decode_header(b: &[u8]) -> Result<(u64, u32)> {
        if b.len() != BLOCK_SIZE || b[0..6] != MAGIC[0..6] {
            return Err(Error::NotFound);
        }
        if xxh3(&b[..BLOCK_SIZE - 16]) != csum_at(b, BLOCK_SIZE - 16) {
            return Err(Error::Corrupted);
        }
        Ok((u64_at(b, 16), u32_at(b, 8)))
    }

    /// Cely superblok. `NotSupported` = jina verze nebo neznamy bit
    /// incompat; neznamy bit ro_compat projde (volajici pripoji jen pro
    /// cteni, viz `ro_compat_unknown`).
    pub fn decode(b: &[u8]) -> Result<Self> {
        let (_, version) = Self::decode_header(b)?;
        if version != VERSION || b[0..8] != MAGIC {
            return Err(Error::NotSupported);
        }
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&b[40..56]);
        // Pred cimkoli dalsim: bez ukazatele s kopii je to stary tvar
        // superbloku (jine posuny poli) - nic dal nejde cist.
        let incompat = u64_at(b, 72);
        if incompat & !FEATURES_INCOMPAT != 0
            || incompat & FEATURES_INCOMPAT_REQUIRED != FEATURES_INCOMPAT_REQUIRED
        {
            return Err(Error::NotSupported);
        }
        let mut label = [0u8; 32];
        label.copy_from_slice(&b[232..264]);
        let sb = Self {
            algo: Algo::from_raw(b[12])?,
            generation: u64_at(b, 16),
            block_count: u64_at(b, 24),
            ag_blocks: u64_at(b, 32),
            uuid,
            features_compat: u64_at(b, 56),
            features_ro_compat: u64_at(b, 64),
            features_incompat: u64_at(b, 72),
            root: BlockPtr::get(&b[80..128]),
            space: BlockPtr::get(&b[128..176]),
            zil_block: u64_at(b, 176),
            zil_seed: u64_at(b, 184),
            next_oid: u64_at(b, 192),
            snapshot_generation: u64_at(b, 200),
            next_snapshot: u64_at(b, 208),
            dead_seq: u64_at(b, 216),
            unicode_version: u32_at(b, 224),
            device_index: u16::from_le_bytes([b[228], b[229]]),
            device_count: u16::from_le_bytes([b[230], b[231]]),
            label,
        };
        // Velikost AG: rozumny rozsah (jinak by pamet pro skupiny vybuchla).
        let ags = sb.block_count.checked_div(sb.ag_blocks).unwrap_or(u64::MAX);
        let bad = sb.ag_blocks < 64
            || ags > 1 << 20
            || sb.block_count <= DATA_START
            || sb.block_count > MAX_BLOCKS
            || sb.device_index >= sb.device_count;
        if bad {
            return Err(Error::Corrupted);
        }
        Ok(sb)
    }
}

/// Zaznam objektu v3 (128 B): soubor, adresar nebo symbolicky odkaz.
///
/// ```text
/// 0   druh u8 (KIND_*)          1   priznaky u8 (INODE_*, zatim 0)
/// 4   popis opravneni u32 (0 = zdedeny; faze 1d)
/// 8   velikost u64 (soubor: bajty, symlink: delka cile)
/// 16  zmena obsahu (mtime)      24  zmena metadat (ctime)
/// 32  vytvoreni (btime)         40  posledni pristup (atime, 0 = nesleduje se)
///     ^ sekundy i64 od 1970 UTC (stary tvar: u64 ns)
/// 48  rodic u64 (0 u korene)    56  generace vzniku u64 (overeni handlu)
/// 64  souhrn podstromu: bajty   72  soubory   80  adresare (bez sebe)
/// 88  vlastnik u64 (0 = system) 96  pocet rozsirenych atributu u32
/// 100 rezerva u32              104 verze obsahu u64 (revize)
/// 112 ns mtime u32  116 ns ctime u32  120 ns btime u32  124 ns atime u32
///     (stary tvar: nuly)
/// ```
///
/// Casy nastavuje `Fs::set_time` (fsd: realny cas). Nanosekundy >= 10^9
/// = poskozeny zaznam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Inode {
    /// `KIND_FILE`, `KIND_DIR` nebo `KIND_SYMLINK`.
    pub kind: u8,
    /// `INODE_*` (zatim jen REMOTE).
    pub flags: u8,
    /// Cislo popisu opravneni (0 = zdedit od rodice).
    pub acl: u32,
    pub size: u64,
    pub mtime: Timestamp,
    pub ctime: Timestamp,
    pub btime: Timestamp,
    /// `Timestamp::UNIX_EPOCH` = nesleduje se.
    pub atime: Timestamp,
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
    /// Pocitadlo zmen obsahu (kazdy zapis, zmena velikosti): spolu s oid
    /// a generaci tvori revizi pro podminene ulozeni.
    pub version: u64,
}

pub const KIND_FILE: u8 = 1;
pub const KIND_DIR: u8 = 2;
pub const KIND_SYMLINK: u8 = 3;
/// Druh polozky adresare: vstup do jineho datasetu (`Dirent::oid` = cislo
/// datasetu). Nema zaznam objektu v rodicovskem strome.
pub const KIND_DATASET: u8 = 4;
/// Cislo korenoveho datasetu (jeho stav je v superbloku).
pub const ROOT_DATASET: u64 = 1;
/// Priznak objektu: data nejsou lokalne, jen v cloudu (faze 6). FS ho musi
/// znat (cteni si data vyzada). Nastaveni sluzby sync ("vzdy offline",
/// "nesynchronizovat") nejsou bity formatu, ale atributy `sync.*`.
#[allow(dead_code, reason = "pouzije faze 6 (soubory na vyzadani)")]
pub const INODE_REMOTE: u8 = 1 << 0;

pub const INODE_SIZE: usize = 128;
/// Nejvetsi velikost souboru (2^63 - 1 B): velikosti a rozdily se tak vzdy
/// vejdou do `i64`.
pub const MAX_FILE_SIZE: u64 = i64::MAX as u64;
/// Nejdelsi cil symbolickeho odkazu v bajtech UTF-8.
pub const SYMLINK_MAX: usize = 4096;

impl Inode {
    /// Novy prazdny objekt druhu `kind` v adresari `parent`.
    pub fn new(kind: u8, parent: u64, now: Timestamp, generation: u64) -> Self {
        Self { kind, parent, mtime: now, ctime: now, btime: now, generation, ..Self::default() }
    }

    pub fn encode(&self, tf: TimeFormat) -> Vec<u8> {
        let mut b = alloc::vec![0u8; INODE_SIZE];
        b[0] = self.kind;
        b[1] = self.flags;
        b[4..8].copy_from_slice(&self.acl.to_le_bytes());
        let times = [self.mtime, self.ctime, self.btime, self.atime];
        let secs = times.map(|t| match tf {
            TimeFormat::Secs => t.secs as u64,
            TimeFormat::LegacyNanos => legacy_nanos(t),
        });
        for (k, v) in [
            self.size,
            secs[0],
            secs[1],
            secs[2],
            secs[3],
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
        b[104..112].copy_from_slice(&self.version.to_le_bytes());
        if tf == TimeFormat::Secs {
            for (k, t) in times.iter().enumerate() {
                b[112 + k * 4..116 + k * 4].copy_from_slice(&t.nanos.to_le_bytes());
            }
        }
        b
    }

    /// Odmitne neznamy druh, velikost nad `MAX_FILE_SIZE` a nanosekundy
    /// mimo rozsah.
    pub fn decode(b: &[u8], tf: TimeFormat) -> Result<Self> {
        if b.len() != INODE_SIZE || !matches!(b[0], KIND_FILE | KIND_DIR | KIND_SYMLINK) {
            return Err(Error::Corrupted);
        }
        let time = |k: usize| -> Result<Timestamp> {
            let raw = u64_at(b, 16 + k * 8);
            match tf {
                TimeFormat::LegacyNanos => Ok(from_legacy_nanos(raw)),
                TimeFormat::Secs => {
                    let nanos = u32_at(b, 112 + k * 4);
                    if nanos >= 1_000_000_000 {
                        return Err(Error::Corrupted);
                    }
                    Ok(Timestamp { secs: raw as i64, nanos })
                }
            }
        };
        let i = Self {
            kind: b[0],
            flags: b[1],
            acl: u32_at(b, 4),
            size: u64_at(b, 8),
            mtime: time(0)?,
            ctime: time(1)?,
            btime: time(2)?,
            atime: time(3)?,
            parent: u64_at(b, 48),
            generation: u64_at(b, 56),
            tree_bytes: u64_at(b, 64),
            tree_files: u64_at(b, 72),
            tree_dirs: u64_at(b, 80),
            owner: u64_at(b, 88),
            xattrs: u32_at(b, 96),
            version: u64_at(b, 104),
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

/// Snapshot: koren stromu v okamziku vytvoreni.
#[derive(Debug, Clone, PartialEq, Eq)]
///
/// ```text
/// 0   koren (BlockPtr 40 B)   40  generace u64
/// 48  pocet klonu u32         52  priznaky u32 (SNAP_HIDDEN)
/// 56  jmeno (UTF-8 do konce)
/// ```
pub struct SnapshotRecord {
    pub root: BlockPtr,
    pub generation: u64,
    /// Kolik klonu z tohoto snapshotu vzniklo a jeste existuje. Dokud > 0,
    /// snapshot nejde smazat doopravdy (jen skryt, `SNAP_HIDDEN`).
    pub clones: u32,
    pub flags: u32,
    pub name: String,
}

/// Snapshot je smazany z pohledu uzivatele, ale drzi bloky pro sve klony;
/// smaze se doopravdy se zanikem posledniho klonu.
pub const SNAP_HIDDEN: u32 = 1;
const SNAP_HEADER: usize = PTR_SIZE + 16;

impl SnapshotRecord {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = alloc::vec![0u8; SNAP_HEADER];
        self.root.put(&mut b[..PTR_SIZE]);
        b[PTR_SIZE..PTR_SIZE + 8].copy_from_slice(&self.generation.to_le_bytes());
        b[PTR_SIZE + 8..PTR_SIZE + 12].copy_from_slice(&self.clones.to_le_bytes());
        b[PTR_SIZE + 12..SNAP_HEADER].copy_from_slice(&self.flags.to_le_bytes());
        b.extend_from_slice(self.name.as_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < SNAP_HEADER {
            return Err(Error::Corrupted);
        }
        let name = core::str::from_utf8(&b[SNAP_HEADER..]).map_err(|_| Error::Corrupted)?;
        Ok(Self {
            root: BlockPtr::get(b),
            generation: u64_at(b, PTR_SIZE),
            clones: u32_at(b, PTR_SIZE + 8),
            flags: u32_at(b, PTR_SIZE + 12),
            name: name.into(),
        })
    }

    pub fn hidden(&self) -> bool {
        self.flags & SNAP_HIDDEN != 0
    }
}

/// Zaznam datasetu (krome korenoveho, ten ma stav v superbloku):
///
/// ```text
/// 0   koren stromu (BlockPtr 40 B)
/// 40  dalsi oid u64             48  dalsi snapshot u64
/// 56  generace posl. snapshotu  64  poradi deadlistu u64
/// 72  puvod: dataset u64        80  puvod: snapshot u64
/// 88  puvod: generace u64       96  priznaky u32
/// ```
///
/// Puvod (`origin`) ma jen klon: dataset a snapshot, ze ktereho vznikl,
/// a jeho generace. Bloky do teto generace drzi puvodni snapshot (klon je
/// sdili), klon vlastni jen bloky narozene pozdeji.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DatasetRecord {
    pub root: BlockPtr,
    pub next_oid: u64,
    pub next_snapshot: u64,
    pub snapshot_generation: u64,
    pub dead_seq: u64,
    /// (dataset, snapshot, generace) puvodu; dataset 0 = neni klon.
    pub origin_ds: u64,
    pub origin_snap: u64,
    pub origin_gen: u64,
    pub flags: u32,
}

pub const DATASET_RECORD_SIZE: usize = PTR_SIZE + 60;

impl DatasetRecord {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = alloc::vec![0u8; DATASET_RECORD_SIZE];
        self.root.put(&mut b[..PTR_SIZE]);
        for (k, v) in [
            self.next_oid,
            self.next_snapshot,
            self.snapshot_generation,
            self.dead_seq,
            self.origin_ds,
            self.origin_snap,
            self.origin_gen,
        ]
        .into_iter()
        .enumerate()
        {
            let o = PTR_SIZE + k * 8;
            b[o..o + 8].copy_from_slice(&v.to_le_bytes());
        }
        b[PTR_SIZE + 56..PTR_SIZE + 60].copy_from_slice(&self.flags.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() != DATASET_RECORD_SIZE {
            return Err(Error::Corrupted);
        }
        Ok(Self {
            root: BlockPtr::get(b),
            next_oid: u64_at(b, PTR_SIZE),
            next_snapshot: u64_at(b, PTR_SIZE + 8),
            snapshot_generation: u64_at(b, PTR_SIZE + 16),
            dead_seq: u64_at(b, PTR_SIZE + 24),
            origin_ds: u64_at(b, PTR_SIZE + 32),
            origin_snap: u64_at(b, PTR_SIZE + 40),
            origin_gen: u64_at(b, PTR_SIZE + 48),
            flags: u32_at(b, PTR_SIZE + 56),
        })
    }
}

/// Mrtvy usek: bloky, ktere zivy stav uz nepouziva, ale drzi je snapshot.
/// `copy` = zacatek druhe kopie stejne delky (0 = bez kopie); uvolni se
/// spolu s prvni (u sdilenych bloku az s posledni referenci).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeadExtent {
    pub start: u64,
    pub len: u64,
    pub birth: u64,
    pub copy: u64,
}

const DEAD_SIZE: usize = 32;
/// Kolik mrtvych useku se vejde do jedne polozky.
pub const DEAD_PER_ITEM: usize = 80;

pub fn encode_dead(list: &[DeadExtent]) -> Vec<u8> {
    let mut b = Vec::with_capacity(list.len() * DEAD_SIZE);
    for d in list {
        b.extend_from_slice(&d.start.to_le_bytes());
        b.extend_from_slice(&d.len.to_le_bytes());
        b.extend_from_slice(&d.birth.to_le_bytes());
        b.extend_from_slice(&d.copy.to_le_bytes());
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
            copy: u64_at(c, 24),
        })
        .collect())
}
