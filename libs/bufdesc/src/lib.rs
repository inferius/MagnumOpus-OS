//! Popis sdileneho bufferu (obraz, snimek videa) nad objektem pameti.
//!
//! Sdileny buffer je VMO plus tento popis (rozhodnuti 6a,
//! `docs/cs/gpu-amd-nvidia.md` bod 2): format (fourcc), modifikator
//! rozlozeni (dlazdice, komprese), roviny `{offset, pitch}`, sirka, vyska,
//! umisteni (systemova pamet / pamet zarizeni) a typ cache. Kdo smi do
//! bufferu zapisovat, urcuji prava handlu VMO, ne popis. Synchronizace je
//! jen explicitni (ploty ve zpravach protokolu), popis ji nenese.
//!
//! Popis prichazi od jineho procesu: prijemce ho **vzdy** overi proti
//! velikosti VMO ([`BufDesc::validate`]) driv, nez z bufferu cte nebo ho
//! preda zarizeni. Overeni hlida preteceni aritmetiky, pocet rovin podle
//! formatu, minimalni krok radku a to, ze kazda rovina lezi cela ve VMO.
//!
//! Hodnoty fourcc a modifikatoru jsou stejne jako v DRM (Linux), aby se
//! daly predat Mese a ovladacum beze zmeny. Kodovani little endian pres
//! `wire` ([`BufDesc::encode`], [`BufDesc::decode`], 72 B).
//!
//! Popis: `docs/cs/sdilene-buffery.md`.

#![no_std]

extern crate alloc;

#[cfg(test)]
mod tests;

use alloc::vec::Vec;

use wire::{Reader, Writer};

/// Kod formatu ze ctyr znaku (little endian, jako `DRM_FORMAT_*`).
pub const fn fourcc(code: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*code)
}

/// Formaty, ktere umi [`BufDesc::validate`] presne spocitat.
pub mod format {
    use super::fourcc;

    /// 32 bitu na pixel, B G R A v pameti (`DRM_FORMAT_ARGB8888`): obvykly
    /// format oken a kompozitoru.
    pub const ARGB8888: u32 = fourcc(b"AR24");
    /// Jako `ARGB8888` bez alfy (`DRM_FORMAT_XRGB8888`).
    pub const XRGB8888: u32 = fourcc(b"XR24");
    /// R G B A v pameti (`DRM_FORMAT_ABGR8888`, Vulkan `R8G8B8A8`).
    pub const ABGR8888: u32 = fourcc(b"AB24");
    /// Jako `ABGR8888` bez alfy.
    pub const XBGR8888: u32 = fourcc(b"XB24");
    /// YUV 4:2:0, rovina Y a prokladana rovina UV (dekodery videa).
    pub const NV12: u32 = fourcc(b"NV12");
    /// YUV 4:2:0, tri roviny Y, U, V (`DRM_FORMAT_YUV420`, I420).
    pub const YUV420: u32 = fourcc(b"YU12");
    /// YUV 4:2:0, 10 bitu v 16 bitech, rovina Y a UV (HDR video).
    pub const P010: u32 = fourcc(b"P010");
}

/// Modifikatory rozlozeni (`DRM_FORMAT_MOD_*`).
pub mod modifier {
    /// Radky za sebou bez dlazdic.
    pub const LINEAR: u64 = 0;
    /// Neplatny / neznamy modifikator (`DRM_FORMAT_MOD_INVALID`).
    pub const INVALID: u64 = 0x00FF_FFFF_FFFF_FFFF;
}

/// Kde buffer lezi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Placement {
    /// Systemova pamet (bezna nebo pripnuta VMO).
    System = 0,
    /// Pamet zarizeni (VRAM, okno BAR): fyzicka VMO, ovladac ji muze odvolat
    /// (`VMO_REVOKE`).
    Device = 1,
}

/// Typ cache mapovani (stejne hodnoty jako `abi::vm::cache`).
pub mod cache {
    pub const UNCACHED: u8 = 0;
    pub const WRITE_COMBINING: u8 = 1;
    pub const CACHED: u8 = 2;
}

/// Nejvic rovin (jako DRM).
pub const MAX_PLANES: usize = 4;
/// Nejvetsi sirka a vyska (16K displeje a snimky videa se vejdou).
pub const MAX_DIM: u32 = 32768;

/// Jedna rovina obrazu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Plane {
    /// Zacatek roviny ve VMO.
    pub offset: u64,
    /// Krok radku v bajtech.
    pub pitch: u32,
}

/// Popis bufferu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufDesc {
    pub fourcc: u32,
    pub modifier: u64,
    pub width: u32,
    pub height: u32,
    /// Roviny v poradi formatu (1 az [`MAX_PLANES`]).
    pub planes: Vec<Plane>,
    pub placement: Placement,
    /// Typ cache ([`cache`]).
    pub cache: u8,
}

/// Proc popis neprosel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescError {
    /// Nulova nebo prilis velka sirka ci vyska.
    Size,
    /// Neznamy format (s modifikatorem `LINEAR`), nebo neplatny modifikator.
    Format,
    /// Pocet rovin neodpovida formatu.
    Planes,
    /// Krok radku roviny je mensi nez radek pixelu.
    Pitch { plane: u8 },
    /// Rovina nelezi cela ve VMO (nebo pretekla aritmetika).
    Bounds { plane: u8 },
    /// Neplatne umisteni nebo typ cache, nebo chyba kodovani.
    Encoding,
}

/// Rozlozeni formatu: pro kazdou rovinu (bajty na prvek, deleni sirky,
/// deleni vysky).
fn layout(fourcc: u32) -> Option<&'static [(u32, u32, u32)]> {
    Some(match fourcc {
        format::ARGB8888 | format::XRGB8888 | format::ABGR8888 | format::XBGR8888 => &[(4, 1, 1)],
        // UV prokladane: 2 bajty na dvojici pixelu na sirku, pul vysky.
        format::NV12 => &[(1, 1, 1), (2, 2, 2)],
        format::YUV420 => &[(1, 1, 1), (1, 2, 2), (1, 2, 2)],
        format::P010 => &[(2, 1, 1), (4, 2, 2)],
        _ => return None,
    })
}

impl BufDesc {
    /// Linearni buffer formatu `fourcc` s tesne navazujicimi rovinami
    /// (krok radku zarovnany na `align` bajtu, mocnina dvou). Pro
    /// producenta, ktery buffer alokuje sam.
    pub fn linear(fourcc: u32, width: u32, height: u32, align: u32) -> Result<Self, DescError> {
        if width == 0 || height == 0 || width > MAX_DIM || height > MAX_DIM {
            return Err(DescError::Size);
        }
        let layout = layout(fourcc).ok_or(DescError::Format)?;
        let align = align.max(1);
        if !align.is_power_of_two() {
            return Err(DescError::Encoding);
        }
        let mut planes = Vec::new();
        let mut offset = 0u64;
        for &(bpp, dx, dy) in layout {
            let pitch = (width.div_ceil(dx) * bpp).next_multiple_of(align);
            planes.push(Plane { offset, pitch });
            offset += pitch as u64 * height.div_ceil(dy) as u64;
        }
        Ok(Self {
            fourcc,
            modifier: modifier::LINEAR,
            width,
            height,
            planes,
            placement: Placement::System,
            cache: cache::CACHED,
        })
    }

    /// Kolik bajtu VMO buffer potrebuje (konec posledni roviny); jen pro
    /// znamy format s `LINEAR`.
    pub fn min_size(&self) -> Result<u64, DescError> {
        let layout = layout(self.fourcc).ok_or(DescError::Format)?;
        let mut end = 0u64;
        for (i, (p, &(bpp, dx, dy))) in self.planes.iter().zip(layout).enumerate() {
            end = end.max(plane_end(p, self.width.div_ceil(dx) as u64 * bpp as u64, self.height.div_ceil(dy) as u64, i)?);
        }
        Ok(end)
    }

    /// Overi popis od jineho procesu proti VMO o velikosti `vmo_size`.
    ///
    /// - znamy format s `LINEAR`: pocet rovin podle formatu, krok radku aspon
    ///   radek pixelu, posledni radek kazde roviny konci ve VMO,
    /// - jiny modifikator (dlazdice ovladace): format se nekontroluje (zna
    ///   ho ovladac), ale kazda rovina `pitch * vyska` musi lezet ve VMO,
    /// - `INVALID` modifikator, nulove nebo obri rozmery, neznamy format
    ///   s `LINEAR` = chyba.
    pub fn validate(&self, vmo_size: u64) -> Result<(), DescError> {
        if self.width == 0 || self.height == 0 || self.width > MAX_DIM || self.height > MAX_DIM {
            return Err(DescError::Size);
        }
        if self.planes.is_empty() || self.planes.len() > MAX_PLANES {
            return Err(DescError::Planes);
        }
        if self.cache > cache::CACHED {
            return Err(DescError::Encoding);
        }
        if self.modifier == modifier::INVALID {
            return Err(DescError::Format);
        }
        if self.modifier != modifier::LINEAR {
            // Rozlozeni dlazdic zna jen ovladac; jadro VMO ale nedovoli cist
            // mimo objekt, takze staci hrube meze: cela rovina ve VMO.
            for (i, p) in self.planes.iter().enumerate() {
                if p.pitch == 0 {
                    return Err(DescError::Pitch { plane: i as u8 });
                }
                let end = (p.pitch as u64)
                    .checked_mul(self.height as u64)
                    .and_then(|n| n.checked_add(p.offset))
                    .ok_or(DescError::Bounds { plane: i as u8 })?;
                if end > vmo_size {
                    return Err(DescError::Bounds { plane: i as u8 });
                }
            }
            return Ok(());
        }
        let layout = layout(self.fourcc).ok_or(DescError::Format)?;
        if layout.len() != self.planes.len() {
            return Err(DescError::Planes);
        }
        for (i, (p, &(bpp, dx, dy))) in self.planes.iter().zip(layout).enumerate() {
            let row = self.width.div_ceil(dx) as u64 * bpp as u64;
            if (p.pitch as u64) < row {
                return Err(DescError::Pitch { plane: i as u8 });
            }
            if plane_end(p, row, self.height.div_ceil(dy) as u64, i)? > vmo_size {
                return Err(DescError::Bounds { plane: i as u8 });
            }
        }
        Ok(())
    }

    /// Zakoduje popis (72 B).
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        self.write(&mut w);
        w.finish()
    }

    /// Zapise popis do vetsi zpravy (protokoly oken a renderu).
    pub fn write(&self, w: &mut Writer) {
        w.u32(self.fourcc).u64(self.modifier).u32(self.width).u32(self.height);
        w.u8(self.planes.len() as u8).u8(self.placement as u8).u8(self.cache).u8(0);
        for i in 0..MAX_PLANES {
            let p = self.planes.get(i).copied().unwrap_or_default();
            w.u64(p.offset).u32(p.pitch);
        }
    }

    /// Rozebere popis z celeho bufferu.
    pub fn decode(data: &[u8]) -> Result<Self, DescError> {
        let mut r = Reader::new(data);
        let d = Self::read(&mut r)?;
        if !r.is_empty() {
            return Err(DescError::Encoding);
        }
        Ok(d)
    }

    /// Precte popis ze zpravy. Nekontroluje meze vuci VMO (to dela
    /// [`BufDesc::validate`]), jen tvar.
    pub fn read(r: &mut Reader) -> Result<Self, DescError> {
        let e = |_| DescError::Encoding;
        let fourcc = r.u32().map_err(e)?;
        let modifier = r.u64().map_err(e)?;
        let width = r.u32().map_err(e)?;
        let height = r.u32().map_err(e)?;
        let count = r.u8().map_err(e)? as usize;
        let placement = match r.u8().map_err(e)? {
            0 => Placement::System,
            1 => Placement::Device,
            _ => return Err(DescError::Encoding),
        };
        let cache = r.u8().map_err(e)?;
        if r.u8().map_err(e)? != 0 || count == 0 || count > MAX_PLANES {
            return Err(DescError::Encoding);
        }
        let mut planes = Vec::new();
        for i in 0..MAX_PLANES {
            let p = Plane {
                offset: r.u64().map_err(e)?,
                pitch: r.u32().map_err(e)?,
            };
            if i < count {
                planes.push(p);
            } else if p != Plane::default() {
                // Nepouzite roviny musi byt nulove (jinak dve kodovani
                // tehoz popisu).
                return Err(DescError::Encoding);
            }
        }
        Ok(Self {
            fourcc,
            modifier,
            width,
            height,
            planes,
            placement,
            cache,
        })
    }
}

/// Konec roviny: `offset + pitch * (rows - 1) + row` (posledni radek nemusi
/// mit vypln), s kontrolou preteceni.
fn plane_end(p: &Plane, row: u64, rows: u64, i: usize) -> Result<u64, DescError> {
    (p.pitch as u64)
        .checked_mul(rows - 1)
        .and_then(|n| n.checked_add(row))
        .and_then(|n| n.checked_add(p.offset))
        .ok_or(DescError::Bounds { plane: i as u8 })
}
