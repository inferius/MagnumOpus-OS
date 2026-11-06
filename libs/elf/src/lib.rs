//! Minimalni parser ELF64 pro x86_64: staticke spustitelne soubory
//! (ET_EXEC). Pouziva ho UEFI loader (jadro), jadro (proces init) a libos
//! (dalsi procesy). Overuje, ze vsechna data lezi uvnitr souboru.

#![no_std]

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const ET_EXEC: u16 = 2;
const EM_X86_64: u16 = 0x3E;
const PT_LOAD: u32 = 1;

pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;
pub const PF_R: u32 = 4;

/// Nahratelny segment (PT_LOAD).
#[derive(Debug, Clone, Copy)]
pub struct Segment {
    pub vaddr: u64,
    pub mem_size: u64,
    pub file_offset: u64,
    pub file_size: u64,
    pub flags: u32,
}

pub struct ElfFile<'a> {
    data: &'a [u8],
    pub entry: u64,
    phoff: u64,
    phentsize: u16,
    phnum: u16,
}

#[derive(Debug)]
pub enum ElfError {
    TooShort,
    BadMagic,
    Unsupported(&'static str),
    BadSegment,
}

impl core::fmt::Display for ElfError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort => f.write_str("soubor je kratsi nez hlavicky ELF"),
            Self::BadMagic => f.write_str("neni to ELF (chybi magicke cislo)"),
            Self::Unsupported(what) => write!(f, "nepodporovany ELF: {what}"),
            Self::BadSegment => f.write_str("segment ELF lezi mimo soubor nebo pretece"),
        }
    }
}

fn bytes<const N: usize>(d: &[u8], off: usize) -> Result<[u8; N], ElfError> {
    off.checked_add(N)
        .and_then(|end| d.get(off..end))
        .map(|b| b.try_into().unwrap())
        .ok_or(ElfError::TooShort)
}

fn u16_at(d: &[u8], off: usize) -> Result<u16, ElfError> {
    bytes(d, off).map(u16::from_le_bytes)
}

fn u32_at(d: &[u8], off: usize) -> Result<u32, ElfError> {
    bytes(d, off).map(u32::from_le_bytes)
}

fn u64_at(d: &[u8], off: usize) -> Result<u64, ElfError> {
    bytes(d, off).map(u64::from_le_bytes)
}

impl<'a> ElfFile<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, ElfError> {
        if data.len() < 64 {
            return Err(ElfError::TooShort);
        }
        if data[0..4] != ELF_MAGIC {
            return Err(ElfError::BadMagic);
        }
        if data[4] != ELFCLASS64 || data[5] != ELFDATA2LSB {
            return Err(ElfError::Unsupported("ocekavan ELF64 little-endian"));
        }
        if u16_at(data, 16)? != ET_EXEC {
            return Err(ElfError::Unsupported("ocekavan ET_EXEC (staticky, ne-PIE)"));
        }
        if u16_at(data, 18)? != EM_X86_64 {
            return Err(ElfError::Unsupported("ocekavan x86_64"));
        }
        let elf = Self {
            data,
            entry: u64_at(data, 24)?,
            phoff: u64_at(data, 32)?,
            phentsize: u16_at(data, 54)?,
            phnum: u16_at(data, 56)?,
        };
        if (elf.phentsize as usize) < 56 {
            return Err(ElfError::Unsupported("prilis maly e_phentsize"));
        }
        Ok(elf)
    }

    /// Iteruje nahratelne segmenty a overi, ze jejich data lezi uvnitr souboru.
    pub fn load_segments(&self) -> impl Iterator<Item = Result<Segment, ElfError>> + '_ {
        (0..self.phnum as usize).filter_map(move |i| self.parse_phdr(i).transpose())
    }

    fn parse_phdr(&self, i: usize) -> Result<Option<Segment>, ElfError> {
        let off = (self.phoff as usize)
            .checked_add(i * self.phentsize as usize)
            .ok_or(ElfError::TooShort)?;
        if u32_at(self.data, off)? != PT_LOAD {
            return Ok(None);
        }
        let seg = Segment {
            flags: u32_at(self.data, off + 4)?,
            file_offset: u64_at(self.data, off + 8)?,
            vaddr: u64_at(self.data, off + 16)?,
            file_size: u64_at(self.data, off + 32)?,
            mem_size: u64_at(self.data, off + 40)?,
        };
        let file_end = seg.file_offset.checked_add(seg.file_size);
        if seg.file_size > seg.mem_size
            || file_end.is_none_or(|e| e > self.data.len() as u64)
            || seg.vaddr.checked_add(seg.mem_size).is_none()
        {
            return Err(ElfError::BadSegment);
        }
        Ok(Some(seg))
    }

    /// Data segmentu ulozena v souboru (rozsah overen v `load_segments`).
    pub fn segment_data(&self, seg: &Segment) -> &'a [u8] {
        &self.data[seg.file_offset as usize..(seg.file_offset + seg.file_size) as usize]
    }
}
