//! Dynamicke symboly (`DT_SYMTAB`, `DT_STRTAB`) a jejich vyhledani podle
//! jmena: GNU hash (`DT_GNU_HASH`, s Bloomovym filtrem - vetsina dotazu na
//! symbol, ktery modul nema, skonci jednim slovem filtru), jinak SysV hash
//! (`DT_HASH`). Verze symbolu (`DT_VERSYM`) se ctou, ale nevyhodnocuji
//! (`docs/cs/dynamicke-linkovani.md`): skryta verze (bit 0x8000) jen
//! vyradi symbol z vyhledavani jinymi moduly.

use crate::dynamic::Dynamic;
use crate::{ElfError, ElfFile, u16_at, u32_at, u64_at};

/// Velikost polozky `Elf64_Sym`.
const SYM_SIZE: usize = 24;
/// Nejvic symbolu v jednom modulu (ochrana pred vadnym hashem).
pub const MAX_SYMBOLS: usize = 1 << 22;

/// Vazba (`st_info >> 4`).
pub mod bind {
    pub const LOCAL: u8 = 0;
    pub const GLOBAL: u8 = 1;
    pub const WEAK: u8 = 2;
}

/// Druh (`st_info & 0xF`).
pub mod kind {
    pub const NOTYPE: u8 = 0;
    pub const OBJECT: u8 = 1;
    pub const FUNC: u8 = 2;
    pub const TLS: u8 = 6;
    pub const GNU_IFUNC: u8 = 10;
}

/// Viditelnost (`st_other & 3`).
pub mod vis {
    pub const DEFAULT: u8 = 0;
    pub const HIDDEN: u8 = 2;
    pub const PROTECTED: u8 = 3;
}

/// Index sekce "nedefinovano".
pub const SHN_UNDEF: u16 = 0;
/// Absolutni hodnota (neposouva se bazi).
pub const SHN_ABS: u16 = 0xFFF1;
/// Bit skryte verze v `DT_VERSYM`.
pub const VERSYM_HIDDEN: u16 = 0x8000;

/// Dynamicky symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Symbol<'a> {
    pub name: &'a str,
    /// Hodnota (u ET_DYN relativni k bazi; u TLS offset v sablone modulu).
    pub value: u64,
    pub size: u64,
    pub bind: u8,
    pub kind: u8,
    pub visibility: u8,
    pub shndx: u16,
}

impl Symbol<'_> {
    pub fn is_defined(&self) -> bool {
        self.shndx != SHN_UNDEF
    }

    pub fn is_tls(&self) -> bool {
        self.kind == kind::TLS
    }

    pub fn is_weak(&self) -> bool {
        self.bind == bind::WEAK
    }

    /// Definovany a viditelny pro ostatni moduly (globalni nebo slaby,
    /// viditelnost default nebo protected).
    pub fn is_exported(&self) -> bool {
        self.is_defined()
            && matches!(self.bind, bind::GLOBAL | bind::WEAK)
            && matches!(self.visibility, vis::DEFAULT | vis::PROTECTED)
    }
}

/// GNU hash jmena (djb2: h * 33 + c).
pub fn gnu_hash(name: &str) -> u32 {
    name.bytes().fold(5381u32, |h, c| h.wrapping_mul(33).wrapping_add(u32::from(c)))
}

/// SysV (ELF) hash jmena.
pub fn sysv_hash(name: &str) -> u32 {
    let mut h = 0u32;
    for c in name.bytes() {
        h = (h << 4).wrapping_add(u32::from(c));
        let g = h & 0xF000_0000;
        if g != 0 {
            h ^= g >> 24;
        }
        h &= !g;
    }
    h
}

/// Tabulka GNU hash.
struct GnuHash<'a> {
    nbuckets: u32,
    symoffset: u32,
    shift: u32,
    bloom: &'a [u8],
    buckets: &'a [u8],
    /// Retezy od `symoffset` (jeden u32 na symbol).
    chains: &'a [u8],
}

/// Tabulka SysV hash.
struct SysvHash<'a> {
    nbucket: u32,
    buckets: &'a [u8],
    chains: &'a [u8],
}

/// Dynamicke symboly modulu.
pub struct SymbolTable<'a> {
    symtab: &'a [u8],
    strtab: &'a [u8],
    count: usize,
    gnu: Option<GnuHash<'a>>,
    sysv: Option<SysvHash<'a>>,
    versym: Option<&'a [u8]>,
}

impl<'a> ElfFile<'a> {
    /// Dynamicke symboly (potrebuje `DT_GNU_HASH` nebo `DT_HASH` - jen
    /// z nich se zjisti pocet symbolu).
    pub fn symbols(&self, d: &Dynamic) -> Result<SymbolTable<'a>, ElfError> {
        let strtab = self.strtab(d)?;
        let symaddr = d.symtab.ok_or(ElfError::BadDynamic("chybi DT_SYMTAB"))?;
        let mut sysv = None;
        let mut count = 0usize;
        if let Some(addr) = d.hash {
            let h = self.at_vaddr(addr, 8)?;
            let (nbucket, nchain) = (u32_at(h, 0)?, u32_at(h, 4)?);
            if nchain as usize > MAX_SYMBOLS || nbucket == 0 {
                return Err(ElfError::BadDynamic("vadny DT_HASH"));
            }
            let all = self.at_vaddr(addr, 8 + 4 * (u64::from(nbucket) + u64::from(nchain)))?;
            let b_end = 8 + 4 * nbucket as usize;
            sysv = Some(SysvHash { nbucket, buckets: &all[8..b_end], chains: &all[b_end..] });
            count = nchain as usize;
        }
        let mut gnu = None;
        if let Some(addr) = d.gnu_hash {
            let h = self.at_vaddr(addr, 16)?;
            let (nbuckets, symoffset, bloom_size, shift) = (u32_at(h, 0)?, u32_at(h, 4)?, u32_at(h, 8)?, u32_at(h, 12)?);
            if nbuckets == 0 || bloom_size == 0 || !bloom_size.is_power_of_two() || symoffset as usize > MAX_SYMBOLS {
                return Err(ElfError::BadDynamic("vadny DT_GNU_HASH"));
            }
            let bloom_len = 8 * u64::from(bloom_size);
            let head = self.at_vaddr(addr, 16 + bloom_len + 4 * u64::from(nbuckets))?;
            let bloom = &head[16..16 + bloom_len as usize];
            let buckets = &head[16 + bloom_len as usize..];
            let chains_addr = addr + 16 + bloom_len + 4 * u64::from(nbuckets);
            // Pocet symbolu: konec retezu nejvyssiho kose.
            let mut last = 0u32;
            for i in 0..nbuckets as usize {
                last = last.max(u32_at(buckets, i * 4)?);
            }
            let gcount = if last < symoffset {
                symoffset as usize
            } else {
                let mut i = last;
                loop {
                    let c = u32_at(self.at_vaddr(chains_addr + 4 * u64::from(i - symoffset), 4)?, 0)?;
                    if c & 1 != 0 {
                        break i as usize + 1;
                    }
                    i += 1;
                    if i as usize >= MAX_SYMBOLS {
                        return Err(ElfError::BadDynamic("retez DT_GNU_HASH bez konce"));
                    }
                }
            };
            let chains = self.at_vaddr(chains_addr, 4 * (gcount - symoffset as usize) as u64)?;
            gnu = Some(GnuHash { nbuckets, symoffset, shift, bloom, buckets, chains });
            count = count.max(gcount);
        }
        if sysv.is_none() && gnu.is_none() {
            return Err(ElfError::BadDynamic("chybi DT_GNU_HASH i DT_HASH"));
        }
        let symtab = self.at_vaddr(symaddr, (count * SYM_SIZE) as u64)?;
        let versym = match d.versym {
            Some(a) => Some(self.at_vaddr(a, (count * 2) as u64)?),
            None => None,
        };
        Ok(SymbolTable { symtab, strtab, count, gnu, sysv, versym })
    }
}

impl<'a> SymbolTable<'a> {
    /// Pocet symbolu (vcetne nulteho prazdneho).
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count <= 1
    }

    /// Symbol s indexem `i` (relokace odkazuji indexem).
    pub fn get(&self, i: usize) -> Result<Symbol<'a>, ElfError> {
        if i >= self.count {
            return Err(ElfError::BadDynamic("index symbolu mimo tabulku"));
        }
        let o = i * SYM_SIZE;
        let s = self.symtab;
        let name_off = u32_at(s, o)?;
        let info = s[o + 4];
        let name = if name_off == 0 { "" } else { cstr(self.strtab, name_off as usize)? };
        Ok(Symbol {
            name,
            value: u64_at(s, o + 8)?,
            size: u64_at(s, o + 16)?,
            bind: info >> 4,
            kind: info & 0xF,
            visibility: s[o + 5] & 3,
            shndx: u16_at(s, o + 6)?,
        })
    }

    /// Surova verze symbolu z `DT_VERSYM` (`None` = soubor verze nema).
    pub fn version(&self, i: usize) -> Option<u16> {
        let v = self.versym?;
        u16_at(v, i * 2).ok()
    }

    /// Najde symbol podle jmena (index a symbol). Vraci jen symboly
    /// viditelne ostatnim modulum (`is_exported`, verze neni skryta).
    pub fn lookup(&self, name: &str) -> Result<Option<(usize, Symbol<'a>)>, ElfError> {
        let found = match (&self.gnu, &self.sysv) {
            (Some(g), _) => self.lookup_gnu(g, name)?,
            (None, Some(h)) => self.lookup_sysv(h, name)?,
            (None, None) => None,
        };
        Ok(found.filter(|(i, s)| s.is_exported() && self.version(*i).is_none_or(|v| v & VERSYM_HIDDEN == 0)))
    }

    fn lookup_gnu(&self, g: &GnuHash<'a>, name: &str) -> Result<Option<(usize, Symbol<'a>)>, ElfError> {
        let h = gnu_hash(name);
        let words = g.bloom.len() / 8;
        let word = u64_at(g.bloom, ((h as usize / 64) % words) * 8)?;
        let mask = (1u64 << (h % 64)) | (1u64 << ((h >> (g.shift % 32)) % 64));
        if word & mask != mask {
            return Ok(None);
        }
        let mut i = u32_at(g.buckets, ((h % g.nbuckets) * 4) as usize)?;
        if i < g.symoffset {
            return Ok(None);
        }
        loop {
            let c = u32_at(g.chains, ((i - g.symoffset) * 4) as usize)?;
            if (c | 1) == (h | 1) {
                let s = self.get(i as usize)?;
                if s.name == name {
                    return Ok(Some((i as usize, s)));
                }
            }
            if c & 1 != 0 {
                return Ok(None);
            }
            i += 1;
        }
    }

    fn lookup_sysv(&self, t: &SysvHash<'a>, name: &str) -> Result<Option<(usize, Symbol<'a>)>, ElfError> {
        let h = sysv_hash(name);
        let mut i = u32_at(t.buckets, ((h % t.nbucket) * 4) as usize)? as usize;
        // Nejvys `count` kroku (vadny retez s cyklem).
        for _ in 0..self.count {
            if i == 0 {
                return Ok(None);
            }
            let s = self.get(i)?;
            if s.name == name {
                return Ok(Some((i, s)));
            }
            i = u32_at(t.chains, i * 4)? as usize;
        }
        Err(ElfError::BadDynamic("cyklus v retezu DT_HASH"))
    }
}

/// Retezec zakonceny nulou na offsetu `off` tabulky.
fn cstr(tab: &[u8], off: usize) -> Result<&str, ElfError> {
    let rest = tab.get(off..).ok_or(ElfError::BadDynamic("jmeno mimo tabulku retezcu"))?;
    let end = rest.iter().position(|&b| b == 0).ok_or(ElfError::BadDynamic("jmeno bez nuly"))?;
    core::str::from_utf8(&rest[..end]).map_err(|_| ElfError::BadDynamic("jmeno neni UTF-8"))
}
