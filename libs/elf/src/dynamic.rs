//! Dynamicka sekce (`PT_DYNAMIC`): seznam `(tag, hodnota)` az do `DT_NULL`.
//!
//! Adresy (`DT_STRTAB`, `DT_SYMTAB`, `DT_RELA` ...) jsou virtualni adresy
//! souboru (u `ET_DYN` relativni k bazi). Ctou se pres
//! [`crate::ElfFile::at_vaddr`]. Ulozi se jen to, co zavadec potrebuje;
//! neznamy tag se preskoci (GNU a dalsi rozsireni).

use crate::{ElfError, u64_at};

/// Tagy dynamicke sekce.
pub mod dt {
    pub const NULL: u64 = 0;
    pub const NEEDED: u64 = 1;
    pub const PLTRELSZ: u64 = 2;
    pub const PLTGOT: u64 = 3;
    pub const HASH: u64 = 4;
    pub const STRTAB: u64 = 5;
    pub const SYMTAB: u64 = 6;
    pub const RELA: u64 = 7;
    pub const RELASZ: u64 = 8;
    pub const RELAENT: u64 = 9;
    pub const STRSZ: u64 = 10;
    pub const SYMENT: u64 = 11;
    pub const INIT: u64 = 12;
    pub const FINI: u64 = 13;
    pub const SONAME: u64 = 14;
    pub const RPATH: u64 = 15;
    pub const SYMBOLIC: u64 = 16;
    pub const REL: u64 = 17;
    pub const PLTREL: u64 = 20;
    pub const TEXTREL: u64 = 22;
    pub const JMPREL: u64 = 23;
    pub const BIND_NOW: u64 = 24;
    pub const INIT_ARRAY: u64 = 25;
    pub const FINI_ARRAY: u64 = 26;
    pub const INIT_ARRAYSZ: u64 = 27;
    pub const FINI_ARRAYSZ: u64 = 28;
    pub const RUNPATH: u64 = 29;
    pub const FLAGS: u64 = 30;
    pub const PREINIT_ARRAY: u64 = 32;
    pub const PREINIT_ARRAYSZ: u64 = 33;
    pub const RELRSZ: u64 = 35;
    pub const RELR: u64 = 36;
    pub const RELRENT: u64 = 37;
    pub const GNU_HASH: u64 = 0x6FFF_FEF5;
    pub const VERSYM: u64 = 0x6FFF_FFF0;
    pub const FLAGS_1: u64 = 0x6FFF_FFFB;
    pub const VERDEF: u64 = 0x6FFF_FFFC;
    pub const VERDEFNUM: u64 = 0x6FFF_FFFD;
    pub const VERNEED: u64 = 0x6FFF_FFFE;
    pub const VERNEEDNUM: u64 = 0x6FFF_FFFF;
}

/// Bity `DT_FLAGS`.
pub mod df {
    pub const SYMBOLIC: u64 = 0x2;
    pub const TEXTREL: u64 = 0x4;
    pub const BIND_NOW: u64 = 0x8;
    pub const STATIC_TLS: u64 = 0x10;
}

/// Bity `DT_FLAGS_1`.
pub mod df1 {
    pub const NOW: u64 = 0x1;
    pub const PIE: u64 = 0x0800_0000;
}

/// Nejvic zavislosti (`DT_NEEDED`) jednoho modulu (FFmpeg a Wine maji
/// mene nez 20); vic = soubor se odmitne.
pub const MAX_NEEDED: usize = 32;

/// Rozsah tabulky (`adresa`, `velikost v B`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Table {
    pub addr: u64,
    pub size: u64,
}

/// Obsah dynamicke sekce.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Dynamic {
    /// Offsety jmen zavislosti v tabulce retezcu (poradi jako v souboru).
    pub needed: [u64; MAX_NEEDED],
    pub needed_count: usize,
    pub soname: Option<u64>,
    /// `DT_RUNPATH` (nebo `DT_RPATH`, kdyz RUNPATH chybi): offset retezce.
    pub runpath: Option<u64>,
    pub strtab: Option<u64>,
    pub strsz: u64,
    pub symtab: Option<u64>,
    pub hash: Option<u64>,
    pub gnu_hash: Option<u64>,
    pub rela: Table,
    pub relr: Table,
    /// Relokace PLT (`DT_JMPREL`, `DT_PLTRELSZ`; vzdy RELA).
    pub jmprel: Table,
    pub init: Option<u64>,
    pub fini: Option<u64>,
    pub init_array: Table,
    pub fini_array: Table,
    pub preinit_array: Table,
    pub flags: u64,
    pub flags_1: u64,
    pub versym: Option<u64>,
    pub verdef: Option<u64>,
    pub verdefnum: u64,
    pub verneed: Option<u64>,
    pub verneednum: u64,
}

impl Dynamic {
    /// Precte dynamickou sekci (`bytes` = obsah PT_DYNAMIC). Odmitne
    /// TEXTREL (W^X), REL (jen RELA), vadne velikosti polozek a chybejici
    /// tabulky retezcu a symbolu.
    pub fn parse(bytes: &[u8]) -> Result<Self, ElfError> {
        let mut d = Self::default();
        let (mut relaent, mut relrent, mut syment, mut pltrel) = (24, 8, 24, dt::RELA);
        let mut rpath = None;
        let mut i = 0;
        loop {
            let tag = u64_at(bytes, i).map_err(|_| ElfError::BadDynamic("chybi DT_NULL"))?;
            let val = u64_at(bytes, i + 8).map_err(|_| ElfError::BadDynamic("chybi DT_NULL"))?;
            i += 16;
            match tag {
                dt::NULL => break,
                dt::NEEDED => {
                    if d.needed_count == MAX_NEEDED {
                        return Err(ElfError::BadDynamic("prilis mnoho DT_NEEDED"));
                    }
                    d.needed[d.needed_count] = val;
                    d.needed_count += 1;
                }
                dt::SONAME => d.soname = Some(val),
                dt::RUNPATH => d.runpath = Some(val),
                dt::RPATH => rpath = Some(val),
                dt::STRTAB => d.strtab = Some(val),
                dt::STRSZ => d.strsz = val,
                dt::SYMTAB => d.symtab = Some(val),
                dt::SYMENT => syment = val,
                dt::HASH => d.hash = Some(val),
                dt::GNU_HASH => d.gnu_hash = Some(val),
                dt::RELA => d.rela.addr = val,
                dt::RELASZ => d.rela.size = val,
                dt::RELAENT => relaent = val,
                dt::RELR => d.relr.addr = val,
                dt::RELRSZ => d.relr.size = val,
                dt::RELRENT => relrent = val,
                dt::JMPREL => d.jmprel.addr = val,
                dt::PLTRELSZ => d.jmprel.size = val,
                dt::PLTREL => pltrel = val,
                dt::REL => return Err(ElfError::Unsupported("relokace REL (jen RELA)")),
                dt::TEXTREL => return Err(ElfError::TextRelocations),
                dt::INIT => d.init = Some(val),
                dt::FINI => d.fini = Some(val),
                dt::INIT_ARRAY => d.init_array.addr = val,
                dt::INIT_ARRAYSZ => d.init_array.size = val,
                dt::FINI_ARRAY => d.fini_array.addr = val,
                dt::FINI_ARRAYSZ => d.fini_array.size = val,
                dt::PREINIT_ARRAY => d.preinit_array.addr = val,
                dt::PREINIT_ARRAYSZ => d.preinit_array.size = val,
                dt::FLAGS => d.flags = val,
                dt::FLAGS_1 => d.flags_1 = val,
                dt::VERSYM => d.versym = Some(val),
                dt::VERDEF => d.verdef = Some(val),
                dt::VERDEFNUM => d.verdefnum = val,
                dt::VERNEED => d.verneed = Some(val),
                dt::VERNEEDNUM => d.verneednum = val,
                _ => {}
            }
        }
        if d.flags & df::TEXTREL != 0 {
            return Err(ElfError::TextRelocations);
        }
        if relaent != 24 || relrent != 8 || syment != 24 {
            return Err(ElfError::BadDynamic("nestandardni velikost polozky"));
        }
        if d.jmprel.size != 0 && pltrel != dt::RELA {
            return Err(ElfError::Unsupported("relokace PLT jinak nez RELA"));
        }
        for t in [d.rela, d.jmprel] {
            if t.size % 24 != 0 {
                return Err(ElfError::BadDynamic("velikost tabulky RELA neni nasobek polozky"));
            }
        }
        for t in [d.relr, d.init_array, d.fini_array, d.preinit_array] {
            if t.size % 8 != 0 {
                return Err(ElfError::BadDynamic("velikost tabulky neni nasobek 8 B"));
            }
        }
        if d.strtab.is_none() || d.symtab.is_none() {
            return Err(ElfError::BadDynamic("chybi DT_STRTAB nebo DT_SYMTAB"));
        }
        if d.runpath.is_none() {
            d.runpath = rpath;
        }
        Ok(d)
    }

    /// Offsety jmen zavislosti (`DT_NEEDED`).
    pub fn needed(&self) -> &[u64] {
        &self.needed[..self.needed_count]
    }

    /// Vsechny symboly se resi hned (`BIND_NOW`, `DF_1_NOW`). Zavadec
    /// line PLT nema - resi vzdy hned; tohle jen rika, zda to soubor ceka.
    pub fn bind_now(&self) -> bool {
        self.flags & df::BIND_NOW != 0 || self.flags_1 & df1::NOW != 0
    }

    /// Soubor je program PIE (`DF_1_PIE`).
    pub fn pie(&self) -> bool {
        self.flags_1 & df1::PIE != 0
    }

    /// Modul chce TLS ve staticke casti (initial-exec, `DF_STATIC_TLS`):
    /// nejde nahrat pozdeji pres `dlopen`, pokud v staticke casti neni misto.
    pub fn static_tls(&self) -> bool {
        self.flags & df::STATIC_TLS != 0
    }
}
