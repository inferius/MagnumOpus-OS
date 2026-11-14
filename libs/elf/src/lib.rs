//! Parser ELF64 (little endian) pro x86_64 a aarch64.
//!
//! - **Staticke programy (`ET_EXEC`)**: [`ElfFile::parse`] pro architekturu,
//!   pro kterou je crate prelozeny. Pouziva ho UEFI loader (jadro), jadro
//!   (proces init) a libos (dalsi procesy).
//! - **Sdilene knihovny a PIE (`ET_DYN`)**: [`ElfFile::parse_object`] pro
//!   zadanou architekturu (host testy umi obe): programove hlavicky
//!   (`PT_INTERP`, `PT_TLS`, `PT_GNU_RELRO`, `PT_DYNAMIC`), dynamicka sekce
//!   ([`dynamic`]), symboly s GNU a SysV hashem ([`symbols`]) a relokace
//!   RELA a RELR ([`reloc`]). Pouzije ho zavadec `ld.so`
//!   (`docs/cs/dynamicke-linkovani.md`).
//!
//! Vsechna data se ctou s kontrolou mezi souboru; nic nepanikari na vadnem
//! vstupu (soubor je neduveryhodny).

#![no_std]

pub mod dynamic;
pub mod reloc;
pub mod symbols;

#[cfg(test)]
mod tests;

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;

/// Typy programovych hlavicek.
pub mod pt {
    pub const LOAD: u32 = 1;
    pub const DYNAMIC: u32 = 2;
    pub const INTERP: u32 = 3;
    pub const PHDR: u32 = 6;
    pub const TLS: u32 = 7;
    pub const GNU_STACK: u32 = 0x6474_E551;
    pub const GNU_RELRO: u32 = 0x6474_E552;
}

pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;
pub const PF_R: u32 = 4;

/// Architektura (`e_machine`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Machine {
    X86_64,
    Aarch64,
}

impl Machine {
    /// Architektura, pro kterou je crate prelozeny.
    #[cfg(target_arch = "x86_64")]
    pub const NATIVE: Machine = Machine::X86_64;
    /// Architektura, pro kterou je crate prelozeny.
    #[cfg(target_arch = "aarch64")]
    pub const NATIVE: Machine = Machine::Aarch64;

    fn code(self) -> u16 {
        match self {
            Self::X86_64 => 0x3E,
            Self::Aarch64 => 0xB7,
        }
    }

    fn expected(self) -> &'static str {
        match self {
            Self::X86_64 => "ocekavan x86_64",
            Self::Aarch64 => "ocekavan aarch64",
        }
    }
}

/// Druh souboru.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfType {
    /// Staticky program na pevnych adresach.
    Exec,
    /// Sdilena knihovna nebo PIE (adresy relativni k bazi).
    Dyn,
}

/// Programova hlavicka (libovolny typ).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Phdr {
    pub p_type: u32,
    pub flags: u32,
    pub offset: u64,
    pub vaddr: u64,
    pub file_size: u64,
    pub mem_size: u64,
    pub align: u64,
}

/// Nahratelny segment (PT_LOAD).
#[derive(Debug, Clone, Copy)]
pub struct Segment {
    pub vaddr: u64,
    pub mem_size: u64,
    pub file_offset: u64,
    pub file_size: u64,
    pub flags: u32,
    /// Zarovnani (`p_align`); zavadec ET_DYN vola bazi s timto zarovnanim.
    pub align: u64,
}

/// Sablona TLS modulu (`PT_TLS`): inicializovana data `file_size` B ze
/// souboru, zbytek do `mem_size` nuly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tls {
    pub vaddr: u64,
    pub file_size: u64,
    pub mem_size: u64,
    pub align: u64,
}

/// Oblast, ktera je po relokaci jen pro cteni (`PT_GNU_RELRO`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Relro {
    pub vaddr: u64,
    pub mem_size: u64,
}

pub struct ElfFile<'a> {
    data: &'a [u8],
    /// `data` je modul nahrany v pameti od baze (adresa `vaddr` = index
    /// `vaddr`), ne soubor: zavadec cte program, ktery namapoval `spawn`.
    loaded: bool,
    pub entry: u64,
    pub elf_type: ElfType,
    pub machine: Machine,
    phoff: u64,
    phentsize: u16,
    phnum: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfError {
    TooShort,
    BadMagic,
    Unsupported(&'static str),
    BadSegment,
    /// Vadna dynamicka cast (tabulka mimo segmenty, spatna velikost
    /// polozky, chybejici povinna polozka).
    BadDynamic(&'static str),
    /// Relokace v kodu (`DT_TEXTREL`): zapis do spustitelne pameti,
    /// odporuje W^X - odmitnuto.
    TextRelocations,
}

impl core::fmt::Display for ElfError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort => f.write_str("soubor je kratsi nez hlavicky ELF"),
            Self::BadMagic => f.write_str("neni to ELF (chybi magicke cislo)"),
            Self::Unsupported(what) => write!(f, "nepodporovany ELF: {what}"),
            Self::BadSegment => f.write_str("segment ELF lezi mimo soubor nebo pretece"),
            Self::BadDynamic(what) => write!(f, "vadna dynamicka cast ELF: {what}"),
            Self::TextRelocations => f.write_str("relokace v kodu (TEXTREL) - odporuje W^X, prelozte s -fPIC"),
        }
    }
}

pub(crate) fn bytes<const N: usize>(d: &[u8], off: usize) -> Result<[u8; N], ElfError> {
    off.checked_add(N)
        .and_then(|end| d.get(off..end))
        .map(|b| b.try_into().unwrap())
        .ok_or(ElfError::TooShort)
}

pub(crate) fn u16_at(d: &[u8], off: usize) -> Result<u16, ElfError> {
    bytes(d, off).map(u16::from_le_bytes)
}

pub(crate) fn u32_at(d: &[u8], off: usize) -> Result<u32, ElfError> {
    bytes(d, off).map(u32::from_le_bytes)
}

pub(crate) fn u64_at(d: &[u8], off: usize) -> Result<u64, ElfError> {
    bytes(d, off).map(u64::from_le_bytes)
}

impl<'a> ElfFile<'a> {
    /// Staticky program (`ET_EXEC`) pro tuto architekturu.
    pub fn parse(data: &'a [u8]) -> Result<Self, ElfError> {
        let elf = Self::parse_object(data, Machine::NATIVE)?;
        if elf.elf_type != ElfType::Exec {
            return Err(ElfError::Unsupported("ocekavan ET_EXEC (staticky, ne-PIE)"));
        }
        Ok(elf)
    }

    /// Program nebo sdilena knihovna (`ET_EXEC` i `ET_DYN`) pro `machine`.
    pub fn parse_object(data: &'a [u8], machine: Machine) -> Result<Self, ElfError> {
        if data.len() < 64 {
            return Err(ElfError::TooShort);
        }
        if data[0..4] != ELF_MAGIC {
            return Err(ElfError::BadMagic);
        }
        if data[4] != ELFCLASS64 || data[5] != ELFDATA2LSB {
            return Err(ElfError::Unsupported("ocekavan ELF64 little-endian"));
        }
        let elf_type = match u16_at(data, 16)? {
            ET_EXEC => ElfType::Exec,
            ET_DYN => ElfType::Dyn,
            _ => return Err(ElfError::Unsupported("ocekavan ET_EXEC nebo ET_DYN")),
        };
        if u16_at(data, 18)? != machine.code() {
            return Err(ElfError::Unsupported(machine.expected()));
        }
        let elf = Self {
            data,
            loaded: false,
            entry: u64_at(data, 24)?,
            elf_type,
            machine,
            phoff: u64_at(data, 32)?,
            phentsize: u16_at(data, 54)?,
            phnum: u16_at(data, 56)?,
        };
        if (elf.phentsize as usize) < 56 {
            return Err(ElfError::Unsupported("prilis maly e_phentsize"));
        }
        Ok(elf)
    }

    /// Modul uz nahrany v pameti: `mem` zacina bazi modulu (adresa 0
    /// souboru) a pokryva vsechny jeho segmenty. Hlavicky ELF musi byt
    /// v prvnim segmentu (offset 0 na adrese 0, jak to dela lld). Data se
    /// ctou primo z `mem[vaddr..]` (relokace uz mohly obsah zmenit).
    pub fn parse_loaded(mem: &'a [u8], machine: Machine) -> Result<Self, ElfError> {
        let mut elf = Self::parse_object(mem, machine)?;
        if elf.elf_type != ElfType::Dyn {
            return Err(ElfError::Unsupported("nahrany modul musi byt ET_DYN"));
        }
        elf.loaded = true;
        Ok(elf)
    }

    /// Cely soubor (nebo pamet modulu u `parse_loaded`).
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    /// Pocet programovych hlavicek (`e_phnum`).
    pub fn phnum(&self) -> u16 {
        self.phnum
    }

    /// Adresa programovych hlavicek v pameti modulu (relativni k bazi):
    /// `PT_PHDR`, jinak `e_phoff` (lld dava hlavicky do prvniho segmentu,
    /// offset 0 = adresa 0).
    pub fn phdr_vaddr(&self) -> Result<u64, ElfError> {
        Ok(self.find(pt::PHDR)?.map_or(self.phoff, |p| p.vaddr))
    }

    /// Vsechny programove hlavicky.
    pub fn program_headers(&self) -> impl Iterator<Item = Result<Phdr, ElfError>> + '_ {
        (0..self.phnum as usize).map(move |i| self.phdr(i))
    }

    fn phdr(&self, i: usize) -> Result<Phdr, ElfError> {
        let off = (self.phoff as usize)
            .checked_add(i * self.phentsize as usize)
            .ok_or(ElfError::TooShort)?;
        Ok(Phdr {
            p_type: u32_at(self.data, off)?,
            flags: u32_at(self.data, off + 4)?,
            offset: u64_at(self.data, off + 8)?,
            vaddr: u64_at(self.data, off + 16)?,
            file_size: u64_at(self.data, off + 32)?,
            mem_size: u64_at(self.data, off + 40)?,
            align: u64_at(self.data, off + 48)?,
        })
    }

    /// Iteruje nahratelne segmenty a overi, ze jejich data lezi uvnitr souboru.
    pub fn load_segments(&self) -> impl Iterator<Item = Result<Segment, ElfError>> + '_ {
        self.program_headers().filter_map(move |p| match p {
            Ok(p) if p.p_type == pt::LOAD => Some(self.check_segment(p)),
            Ok(_) => None,
            Err(e) => Some(Err(e)),
        })
    }

    fn check_segment(&self, p: Phdr) -> Result<Segment, ElfError> {
        let seg = Segment {
            flags: p.flags,
            file_offset: p.offset,
            vaddr: p.vaddr,
            file_size: p.file_size,
            mem_size: p.mem_size,
            align: p.align,
        };
        // V souboru lezi data od `file_offset`, v nahranem modulu od `vaddr`.
        let at = if self.loaded { seg.vaddr } else { seg.file_offset };
        let file_end = at.checked_add(seg.file_size);
        if seg.file_size > seg.mem_size
            || file_end.is_none_or(|e| e > self.data.len() as u64)
            || seg.vaddr.checked_add(seg.mem_size).is_none()
        {
            return Err(ElfError::BadSegment);
        }
        Ok(seg)
    }

    /// Data segmentu ulozena v souboru (rozsah overen v `load_segments`).
    pub fn segment_data(&self, seg: &Segment) -> &'a [u8] {
        let at = if self.loaded { seg.vaddr } else { seg.file_offset } as usize;
        &self.data[at..at + seg.file_size as usize]
    }

    /// Prvni programova hlavicka typu `p_type`.
    fn find(&self, p_type: u32) -> Result<Option<Phdr>, ElfError> {
        for p in self.program_headers() {
            let p = p?;
            if p.p_type == p_type {
                return Ok(Some(p));
            }
        }
        Ok(None)
    }

    /// Bajty souboru v rozsahu programove hlavicky (`offset`, `file_size`).
    fn file_range(&self, p: &Phdr) -> Result<&'a [u8], ElfError> {
        let at = if self.loaded { p.vaddr } else { p.offset };
        let start = usize::try_from(at).map_err(|_| ElfError::BadSegment)?;
        let len = usize::try_from(p.file_size).map_err(|_| ElfError::BadSegment)?;
        start.checked_add(len).and_then(|end| self.data.get(start..end)).ok_or(ElfError::BadSegment)
    }

    /// Cesta zavadece (`PT_INTERP`, bez koncove nuly); `None` = staticky
    /// nebo knihovna.
    pub fn interp(&self) -> Result<Option<&'a str>, ElfError> {
        let Some(p) = self.find(pt::INTERP)? else { return Ok(None) };
        let b = self.file_range(&p)?;
        let b = b.strip_suffix(&[0]).unwrap_or(b);
        core::str::from_utf8(b).map(Some).map_err(|_| ElfError::BadDynamic("PT_INTERP neni UTF-8"))
    }

    /// Sablona TLS (`PT_TLS`); `None` = modul TLS nema.
    pub fn tls(&self) -> Result<Option<Tls>, ElfError> {
        let Some(p) = self.find(pt::TLS)? else { return Ok(None) };
        self.file_range(&p)?;
        if p.file_size > p.mem_size || (p.align > 1 && !p.align.is_power_of_two()) {
            return Err(ElfError::BadSegment);
        }
        Ok(Some(Tls { vaddr: p.vaddr, file_size: p.file_size, mem_size: p.mem_size, align: p.align.max(1) }))
    }

    /// Inicializovana data sablony TLS.
    pub fn tls_data(&self, tls: &Tls) -> Result<&'a [u8], ElfError> {
        self.at_vaddr(tls.vaddr, tls.file_size)
    }

    /// Oblast RELRO (`PT_GNU_RELRO`).
    pub fn relro(&self) -> Result<Option<Relro>, ElfError> {
        Ok(self.find(pt::GNU_RELRO)?.map(|p| Relro { vaddr: p.vaddr, mem_size: p.mem_size }))
    }

    /// Dynamicka sekce (`PT_DYNAMIC`); `None` = staticky soubor. Odmitne
    /// `DT_TEXTREL` a `DT_REL` (x86_64 a aarch64 pouzivaji RELA).
    pub fn dynamic(&self) -> Result<Option<dynamic::Dynamic>, ElfError> {
        let Some(p) = self.find(pt::DYNAMIC)? else { return Ok(None) };
        let b = self.at_vaddr(p.vaddr, p.file_size)?;
        dynamic::Dynamic::parse(b).map(Some)
    }

    /// `len` B souboru na virtualni adrese `vaddr` (relativni k bazi u
    /// ET_DYN): musi lezet v datech ze souboru jednoho segmentu PT_LOAD.
    pub fn at_vaddr(&self, vaddr: u64, len: u64) -> Result<&'a [u8], ElfError> {
        if self.loaded {
            let end = vaddr.checked_add(len).ok_or(ElfError::BadDynamic("adresa pretece"))?;
            return usize::try_from(end)
                .ok()
                .and_then(|e| self.data.get(vaddr as usize..e))
                .ok_or(ElfError::BadDynamic("adresa mimo modul"));
        }
        for seg in self.load_segments() {
            let seg = seg?;
            let end = vaddr.checked_add(len).ok_or(ElfError::BadDynamic("adresa pretece"))?;
            if vaddr >= seg.vaddr && end <= seg.vaddr + seg.file_size {
                let off = (seg.file_offset + (vaddr - seg.vaddr)) as usize;
                return Ok(&self.data[off..off + len as usize]);
            }
        }
        Err(ElfError::BadDynamic("adresa mimo data segmentu"))
    }

    /// Retezec zakonceny nulou na offsetu `off` tabulky retezcu.
    pub(crate) fn cstr_at(&self, strtab: &'a [u8], off: u64) -> Result<&'a str, ElfError> {
        let start = usize::try_from(off).map_err(|_| ElfError::BadDynamic("retezec mimo tabulku"))?;
        let rest = strtab.get(start..).ok_or(ElfError::BadDynamic("retezec mimo tabulku"))?;
        let end = rest.iter().position(|&b| b == 0).ok_or(ElfError::BadDynamic("retezec bez nuly"))?;
        core::str::from_utf8(&rest[..end]).map_err(|_| ElfError::BadDynamic("retezec neni UTF-8"))
    }

    /// Tabulka retezcu dynamicke casti (`DT_STRTAB`, `DT_STRSZ`).
    pub fn strtab(&self, d: &dynamic::Dynamic) -> Result<&'a [u8], ElfError> {
        let (addr, size) = (d.strtab.ok_or(ElfError::BadDynamic("chybi DT_STRTAB"))?, d.strsz);
        self.at_vaddr(addr, size)
    }

    /// Jmeno z tabulky retezcu dynamicke casti.
    pub fn dyn_str(&self, d: &dynamic::Dynamic, off: u64) -> Result<&'a str, ElfError> {
        let s = self.strtab(d)?;
        self.cstr_at(s, off)
    }
}
