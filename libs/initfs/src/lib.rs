//! Format initfs: jednoduchy archiv souboru, ktery loader nahraje do pameti
//! spolu s jadrem. Obsahuje programy potrebne pred existenci souboroveho
//! systemu (init, ovladace, ...).
//!
//! ```text
//! 0      Header (32 B)
//! 32     Entry * count (32 B kazda)
//! ...    jmena souboru (UTF-8, bez ukonceni)
//! ...    data souboru, kazdy zarovnany na 4 KiB (lze je primo mapovat)
//! ```
//!
//! Vsechna cisla little-endian. Cteni (`Initfs`) je `no_std` bez alokace,
//! zapis (`build`) je za feature `std` (pouziva xtask).

#![cfg_attr(not(feature = "std"), no_std)]

pub const MAGIC: [u8; 8] = *b"OSINITFS";
pub const VERSION: u32 = 1;
pub const HEADER_SIZE: usize = 32;
pub const ENTRY_SIZE: usize = 32;
pub const DATA_ALIGN: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitfsError {
    BadMagic,
    UnsupportedVersion(u32),
    Truncated,
    BadName,
}

impl core::fmt::Display for InitfsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadMagic => f.write_str("neni to initfs (chybi magicke cislo)"),
            Self::UnsupportedVersion(v) => write!(f, "nepodporovana verze initfs {v}"),
            Self::Truncated => f.write_str("initfs je zkraceny nebo ukazuje mimo sebe"),
            Self::BadName => f.write_str("jmeno souboru v initfs neni UTF-8"),
        }
    }
}

fn u32_at(b: &[u8], off: usize) -> Result<u32, InitfsError> {
    let s = b.get(off..off + 4).ok_or(InitfsError::Truncated)?;
    Ok(u32::from_le_bytes(s.try_into().unwrap()))
}

fn u64_at(b: &[u8], off: usize) -> Result<u64, InitfsError> {
    let s = b.get(off..off + 8).ok_or(InitfsError::Truncated)?;
    Ok(u64::from_le_bytes(s.try_into().unwrap()))
}

/// Pohled na obraz initfs v pameti.
#[derive(Clone, Copy)]
pub struct Initfs<'a> {
    data: &'a [u8],
    count: usize,
}

/// Jeden soubor.
#[derive(Debug, Clone, Copy)]
pub struct File<'a> {
    pub name: &'a str,
    /// Offset dat od zacatku obrazu (zarovnany na `DATA_ALIGN`).
    pub offset: u64,
    pub data: &'a [u8],
}

impl<'a> Initfs<'a> {
    /// Cely obraz (jak ho dostal `parse`).
    pub fn bytes(&self) -> &'a [u8] {
        self.data
    }

    pub fn parse(data: &'a [u8]) -> Result<Self, InitfsError> {
        if data.get(0..8) != Some(&MAGIC[..]) {
            return Err(InitfsError::BadMagic);
        }
        let version = u32_at(data, 8)?;
        if version != VERSION {
            return Err(InitfsError::UnsupportedVersion(version));
        }
        let count = u32_at(data, 12)? as usize;
        let fs = Self { data, count };
        // Overit vsechny polozky predem, at je pak iterace bez chyb.
        for i in 0..count {
            fs.entry(i)?;
        }
        Ok(fs)
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn entry(&self, i: usize) -> Result<File<'a>, InitfsError> {
        let base = HEADER_SIZE + i * ENTRY_SIZE;
        let name_off = u32_at(self.data, base)? as usize;
        let name_len = u32_at(self.data, base + 4)? as usize;
        let data_off = u64_at(self.data, base + 8)?;
        let data_len = u64_at(self.data, base + 16)?;
        let name = self
            .data
            .get(
                name_off
                    ..name_off
                        .checked_add(name_len)
                        .ok_or(InitfsError::Truncated)?,
            )
            .ok_or(InitfsError::Truncated)?;
        let name = core::str::from_utf8(name).map_err(|_| InitfsError::BadName)?;
        let start = usize::try_from(data_off).map_err(|_| InitfsError::Truncated)?;
        let end = start
            .checked_add(usize::try_from(data_len).map_err(|_| InitfsError::Truncated)?)
            .ok_or(InitfsError::Truncated)?;
        let data = self.data.get(start..end).ok_or(InitfsError::Truncated)?;
        Ok(File {
            name,
            offset: data_off,
            data,
        })
    }

    pub fn files(&self) -> impl Iterator<Item = File<'a>> + '_ {
        // Polozky jsou overene v `parse`.
        (0..self.count).filter_map(|i| self.entry(i).ok())
    }

    pub fn find(&self, name: &str) -> Option<File<'a>> {
        self.files().find(|f| f.name == name)
    }
}

/// Sestavi obraz initfs ze seznamu (jmeno, data).
#[cfg(feature = "std")]
pub fn build(files: &[(&str, &[u8])]) -> Vec<u8> {
    let names_start = HEADER_SIZE + files.len() * ENTRY_SIZE;
    let names_len: usize = files.iter().map(|(n, _)| n.len()).sum();
    let mut data_off = ((names_start + names_len) as u64).next_multiple_of(DATA_ALIGN);

    let mut out = vec![0u8; names_start];
    out[0..8].copy_from_slice(&MAGIC);
    out[8..12].copy_from_slice(&VERSION.to_le_bytes());
    out[12..16].copy_from_slice(&(files.len() as u32).to_le_bytes());

    let mut name_off = names_start;
    let mut offsets = Vec::new();
    for (i, (name, data)) in files.iter().enumerate() {
        let e = HEADER_SIZE + i * ENTRY_SIZE;
        out[e..e + 4].copy_from_slice(&(name_off as u32).to_le_bytes());
        out[e + 4..e + 8].copy_from_slice(&(name.len() as u32).to_le_bytes());
        out[e + 8..e + 16].copy_from_slice(&data_off.to_le_bytes());
        out[e + 16..e + 24].copy_from_slice(&(data.len() as u64).to_le_bytes());
        offsets.push(data_off);
        name_off += name.len();
        data_off = (data_off + data.len() as u64).next_multiple_of(DATA_ALIGN);
    }
    for (name, _) in files {
        out.extend_from_slice(name.as_bytes());
    }
    for ((_, data), off) in files.iter().zip(offsets) {
        out.resize(off as usize, 0);
        out.extend_from_slice(data);
    }
    out.resize((out.len() as u64).next_multiple_of(DATA_ALIGN) as usize, 0);
    out
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let img = build(&[("init", b"abc"), ("dir/echo", &[1u8; 5000])]);
        let fs = Initfs::parse(&img).unwrap();
        assert_eq!(fs.len(), 2);
        let init = fs.find("init").unwrap();
        assert_eq!(init.data, b"abc");
        assert_eq!(init.offset % DATA_ALIGN, 0);
        assert_eq!(fs.find("dir/echo").unwrap().data.len(), 5000);
        assert!(fs.find("nic").is_none());
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(Initfs::parse(b"nesmysl").err(), Some(InitfsError::BadMagic));
        let mut img = build(&[("a", b"x")]);
        img.truncate(40);
        assert_eq!(Initfs::parse(&img).err(), Some(InitfsError::Truncated));
    }
}
