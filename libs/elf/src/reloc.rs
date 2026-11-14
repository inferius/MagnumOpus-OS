//! Relokace dynamickych modulu: RELA (`DT_RELA`, `DT_JMPREL`) a kompaktni
//! RELR (`DT_RELR`, jen relativni). Typy obou architektur se prevedou na
//! spolecny [`Kind`]; hodnotu pro zapis spocita zavadec ([`Rela::value`]
//! pro obycejne druhy, TLS podle sveho rozlozeni TLS).
//!
//! Linker v systemu dela vzdy `BIND_NOW` (zadne line PLT): `JUMP_SLOT` se
//! resi hned jako `GLOB_DAT`.

use crate::dynamic::{Dynamic, Table};
use crate::{ElfError, ElfFile, Machine, u64_at};

/// Druh relokace (spolecny pro x86_64 a aarch64).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    None,
    /// B + A (baze modulu + addend).
    Relative,
    /// S + A (x86_64 `R_X86_64_64`, aarch64 `R_AARCH64_ABS64`).
    Absolute,
    /// S + A do GOT.
    GlobDat,
    /// S + A do GOT PLT (resi se hned).
    JumpSlot,
    /// Kopie dat symbolu do programu - nepodporovano (programy jsou PIE
    /// a s `-fPIC` copy relokace nevznikaji).
    Copy,
    /// Vysledek funkce (B + A)() - IFUNC.
    IRelative,
    /// Cislo modulu TLS (x86_64 `DTPMOD64`, aarch64 `TLS_DTPMOD`).
    TlsModule,
    /// Offset v bloku TLS modulu: S + A (`DTPOFF64` / `TLS_DTPREL`).
    TlsOffset,
    /// Offset od ukazatele vlakna ve staticke casti TLS (`TPOFF64` /
    /// `TLS_TPREL`).
    TlsTpOffset,
    /// Deskriptor TLS (2 slova: funkce a argument; `TLSDESC`).
    TlsDesc,
    /// Neznamy typ (cislo podle architektury) - zavadec modul odmitne.
    Unknown(u32),
}

impl Kind {
    pub fn from_raw(machine: Machine, t: u32) -> Kind {
        match machine {
            Machine::X86_64 => match t {
                0 => Kind::None,
                1 => Kind::Absolute,
                5 => Kind::Copy,
                6 => Kind::GlobDat,
                7 => Kind::JumpSlot,
                8 => Kind::Relative,
                16 => Kind::TlsModule,
                17 => Kind::TlsOffset,
                18 => Kind::TlsTpOffset,
                36 => Kind::TlsDesc,
                37 => Kind::IRelative,
                _ => Kind::Unknown(t),
            },
            Machine::Aarch64 => match t {
                0 => Kind::None,
                257 => Kind::Absolute,
                1024 => Kind::Copy,
                1025 => Kind::GlobDat,
                1026 => Kind::JumpSlot,
                1027 => Kind::Relative,
                1028 => Kind::TlsModule,
                1029 => Kind::TlsOffset,
                1030 => Kind::TlsTpOffset,
                1031 => Kind::TlsDesc,
                1032 => Kind::IRelative,
                _ => Kind::Unknown(t),
            },
        }
    }

    /// Potrebuje relokace symbol (index != 0 dava smysl)?
    pub fn uses_symbol(self) -> bool {
        matches!(
            self,
            Kind::Absolute | Kind::GlobDat | Kind::JumpSlot | Kind::Copy | Kind::TlsModule | Kind::TlsOffset | Kind::TlsTpOffset | Kind::TlsDesc
        )
    }
}

/// Relokace RELA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rela {
    /// Kam zapsat (adresa relativni k bazi u ET_DYN).
    pub offset: u64,
    pub kind: Kind,
    /// Index symbolu (0 = bez symbolu).
    pub sym: u32,
    pub addend: i64,
}

impl Rela {
    /// Hodnota pro obycejne druhy: `Relative` = B + A, `Absolute`,
    /// `GlobDat`, `JumpSlot` = S + A (`s` = adresa symbolu uz s bazi
    /// modulu, ktery ho definuje). TLS, IFUNC a Copy pocita zavadec sam.
    pub fn value(&self, base: u64, s: u64) -> Option<u64> {
        match self.kind {
            Kind::Relative => Some(base.wrapping_add_signed(self.addend)),
            Kind::Absolute | Kind::GlobDat | Kind::JumpSlot => Some(s.wrapping_add_signed(self.addend)),
            _ => None,
        }
    }
}

/// Velikost polozky RELA.
const RELA_SIZE: usize = 24;

impl<'a> ElfFile<'a> {
    fn table(&self, t: Table) -> Result<&'a [u8], ElfError> {
        if t.size == 0 {
            return Ok(&[]);
        }
        self.at_vaddr(t.addr, t.size)
    }

    /// Relokace RELA: nejdriv `DT_RELA`, pak PLT (`DT_JMPREL`).
    pub fn relas(&self, d: &Dynamic) -> Result<impl Iterator<Item = Result<Rela, ElfError>> + 'a, ElfError> {
        let a = self.table(d.rela)?;
        let b = self.table(d.jmprel)?;
        let machine = self.machine;
        Ok(a.chunks_exact(RELA_SIZE).chain(b.chunks_exact(RELA_SIZE)).map(move |e| {
            let info = u64_at(e, 8)?;
            Ok(Rela {
                offset: u64_at(e, 0)?,
                kind: Kind::from_raw(machine, info as u32),
                sym: (info >> 32) as u32,
                addend: u64_at(e, 16)? as i64,
            })
        }))
    }

    /// Adresy relativnich relokaci RELR (zapsat B + puvodni slovo).
    pub fn relrs(&self, d: &Dynamic) -> Result<Relr<'a>, ElfError> {
        Ok(Relr::decode(self.table(d.relr)?))
    }
}

/// Dekoder RELR: sude slovo = adresa (a dalsi slovo za ni), liche =
/// bitmapa 63 nasledujicich slov (bit 1 = slovo `base + 8 * (i - 1)`).
pub struct Relr<'a> {
    entries: &'a [u8],
    pos: usize,
    /// Adresa prvniho slova, ke kteremu se vztahuje aktualni bitmapa.
    base: u64,
    /// Zbyvajici bity aktualni bitmapy (bez bitu 0).
    bits: u64,
    /// Index bitu v bitmape (0 = slovo `base`).
    bit: u32,
    /// Zpracovava se bitmapa (po ni se `base` posune o 63 slov, i prazdne).
    in_bitmap: bool,
}

impl<'a> Relr<'a> {
    /// Dekoder nad surovymi polozkami RELR (little endian, 8 B).
    pub fn decode(entries: &'a [u8]) -> Self {
        Relr { entries, pos: 0, base: 0, bits: 0, bit: 0, in_bitmap: false }
    }
}

impl Iterator for Relr<'_> {
    type Item = Result<u64, ElfError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            while self.bits != 0 {
                let b = self.bits & 1;
                self.bits >>= 1;
                let at = self.base.checked_add(8 * u64::from(self.bit));
                self.bit += 1;
                if b != 0 {
                    return Some(at.ok_or(ElfError::BadDynamic("RELR pretece")));
                }
            }
            if self.in_bitmap {
                // Bitmapa vycerpana: dalsi pokracuje o 63 slov dal.
                self.base = match self.base.checked_add(63 * 8) {
                    Some(b) => b,
                    None => return Some(Err(ElfError::BadDynamic("RELR pretece"))),
                };
                self.bit = 0;
                self.in_bitmap = false;
            }
            let e = match u64_at(self.entries, self.pos) {
                Ok(e) => e,
                Err(_) => return None,
            };
            self.pos += 8;
            if e & 1 == 0 {
                // Adresa: relokuje se ona a bitmapy dal plati od dalsiho slova.
                self.base = e.wrapping_add(8);
                self.bit = 0;
                return Some(Ok(e));
            }
            if self.pos == 8 {
                return Some(Err(ElfError::BadDynamic("RELR zacina bitmapou")));
            }
            self.bits = e >> 1;
            self.bit = 0;
            self.in_bitmap = true;
        }
    }
}
