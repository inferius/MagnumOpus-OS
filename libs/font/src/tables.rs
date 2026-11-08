//! Tabulky TrueType: adresar tabulek, `head`, `hhea`, `maxp`, `cmap`
//! (formaty 4 a 12), `loca`, `hmtx`, `kern` (format 0).
//!
//! Cisla jsou big-endian. Kazde cteni jde pres [`Data`], ktera mimo meze
//! vraci `None`.

use crate::FontError;

/// Cteni big-endian s kontrolou mezi.
#[derive(Clone, Copy)]
pub struct Data<'a>(pub &'a [u8]);

impl<'a> Data<'a> {
    pub fn u8(&self, off: usize) -> Option<u8> {
        self.0.get(off).copied()
    }

    pub fn u16(&self, off: usize) -> Option<u16> {
        let b = self.0.get(off..off.checked_add(2)?)?;
        Some(u16::from_be_bytes([b[0], b[1]]))
    }

    pub fn i16(&self, off: usize) -> Option<i16> {
        self.u16(off).map(|v| v as i16)
    }

    pub fn u32(&self, off: usize) -> Option<u32> {
        let b = self.0.get(off..off.checked_add(4)?)?;
        Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn slice(&self, off: usize, len: usize) -> Option<Data<'a>> {
        self.0.get(off..off.checked_add(len)?).map(Data)
    }

    pub fn from(&self, off: usize) -> Option<Data<'a>> {
        self.0.get(off..).map(Data)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

#[derive(Clone, Copy)]
enum Cmap {
    /// Format 4 (BMP): data podtabulky.
    Format4(usize),
    /// Format 12 (cele Unicode).
    Format12(usize),
}

pub struct Tables<'a> {
    pub units_per_em: u16,
    /// `indexToLocFormat`: 0 = kratke (u16 * 2), 1 = dlouhe (u32).
    long_loca: bool,
    pub num_glyphs: u16,
    num_hmetrics: u16,
    pub ascender: i16,
    pub descender: i16,
    pub line_gap: i16,
    cmap: Cmap,
    cmap_data: Data<'a>,
    loca: Data<'a>,
    pub glyf: Data<'a>,
    hmtx: Data<'a>,
    /// Dvojice kerningu (format 0): data par a jejich pocet.
    kern: Option<(Data<'a>, usize)>,
    /// Tabulky OpenType rozvrzeni (nepovinne).
    pub gpos: Option<Data<'a>>,
    pub gsub: Option<Data<'a>>,
}

fn c<T>(v: Option<T>) -> Result<T, FontError> {
    v.ok_or(FontError::Corrupted)
}

impl<'a> Tables<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, FontError> {
        let d = Data(bytes);
        let version = d.u32(0).ok_or(FontError::NotTrueType)?;
        if version != 0x0001_0000 && version != u32::from_be_bytes(*b"true") {
            return Err(if version == u32::from_be_bytes(*b"OTTO") {
                FontError::Unsupported
            } else {
                FontError::NotTrueType
            });
        }
        let num_tables = d.u16(4).ok_or(FontError::NotTrueType)? as usize;
        let find = |tag: &[u8; 4]| -> Option<Data<'a>> {
            for i in 0..num_tables {
                let rec = 12 + i * 16;
                if d.slice(rec, 4)?.0 == tag {
                    let off = d.u32(rec + 8)? as usize;
                    let len = d.u32(rec + 12)? as usize;
                    return d.slice(off, len);
                }
            }
            None
        };
        let req = |tag: &[u8; 4]| find(tag).ok_or(FontError::NotTrueType);
        let head = req(b"head")?;
        let hhea = req(b"hhea")?;
        let maxp = req(b"maxp")?;
        let cmap_data = req(b"cmap")?;
        let loca = req(b"loca")?;
        let glyf = req(b"glyf")?;
        let hmtx = req(b"hmtx")?;
        let units_per_em = c(head.u16(18))?;
        if !(16..=16384).contains(&units_per_em) {
            return Err(FontError::Corrupted);
        }
        let long_loca = match c(head.i16(50))? {
            0 => false,
            1 => true,
            _ => return Err(FontError::Corrupted),
        };
        let num_glyphs = c(maxp.u16(4))?;
        let num_hmetrics = c(hhea.u16(34))?;
        if num_hmetrics == 0 || hmtx.len() < num_hmetrics as usize * 4 {
            return Err(FontError::Corrupted);
        }
        let cmap = Self::pick_cmap(cmap_data)?;
        let kern = find(b"kern").and_then(Self::parse_kern);
        Ok(Self {
            units_per_em,
            long_loca,
            num_glyphs,
            num_hmetrics,
            ascender: c(hhea.i16(4))?,
            descender: c(hhea.i16(6))?,
            line_gap: c(hhea.i16(8))?,
            cmap,
            cmap_data,
            loca,
            glyf,
            hmtx,
            kern,
            gpos: find(b"GPOS"),
            gsub: find(b"GSUB"),
        })
    }

    /// Vybere podtabulku Unicode: (3,10) format 12 > (0,4) format 12 >
    /// (3,1) nebo (0,*) format 4.
    fn pick_cmap(cm: Data<'a>) -> Result<Cmap, FontError> {
        let n = c(cm.u16(2))? as usize;
        let mut best: Option<(u8, Cmap)> = None;
        for i in 0..n {
            let rec = 4 + i * 8;
            let (Some(platform), Some(encoding), Some(off)) = (cm.u16(rec), cm.u16(rec + 2), cm.u32(rec + 4)) else {
                break;
            };
            let off = off as usize;
            let Some(format) = cm.u16(off) else { continue };
            let unicode = platform == 0 || (platform == 3 && (encoding == 1 || encoding == 10));
            if !unicode {
                continue;
            }
            let cand = match format {
                12 => (2, Cmap::Format12(off)),
                4 => (1, Cmap::Format4(off)),
                _ => continue,
            };
            if best.is_none_or(|(p, _)| cand.0 > p) {
                best = Some(cand);
            }
        }
        best.map(|(_, m)| m).ok_or(FontError::Unsupported)
    }

    fn parse_kern(k: Data<'a>) -> Option<(Data<'a>, usize)> {
        // Verze 0: u16 verze, u16 pocet podtabulek; podtabulka: u16 verze,
        // u16 delka, u16 coverage (format v hornim bajtu, bit 0 = vodorovny).
        if k.u16(0)? != 0 {
            return None;
        }
        let n = k.u16(2)? as usize;
        let mut off = 4;
        for _ in 0..n {
            let len = k.u16(off + 2)? as usize;
            let coverage = k.u16(off + 4)?;
            if coverage >> 8 == 0 && coverage & 1 == 1 && coverage & 0x4 == 0 {
                let pairs = k.u16(off + 6)? as usize;
                let data = k.slice(off + 14, pairs * 6)?;
                return Some((data, pairs));
            }
            off += len.max(6);
        }
        None
    }

    pub fn cmap_lookup(&self, cp: u32) -> u16 {
        match self.cmap {
            Cmap::Format4(off) => self.cmap4(off, cp).unwrap_or(0),
            Cmap::Format12(off) => self.cmap12(off, cp).unwrap_or(0),
        }
    }

    fn cmap4(&self, off: usize, cp: u32) -> Option<u16> {
        if cp > 0xFFFF {
            return Some(0);
        }
        let cp = cp as u16;
        let t = self.cmap_data.from(off)?;
        let segx2 = t.u16(6)? as usize;
        let seg = segx2 / 2;
        let ends = 14;
        let starts = ends + segx2 + 2;
        let deltas = starts + segx2;
        let ranges = deltas + segx2;
        // Binarni hledani segmentu s end >= cp.
        let (mut lo, mut hi) = (0usize, seg);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if t.u16(ends + mid * 2)? < cp {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo >= seg {
            return Some(0);
        }
        let start = t.u16(starts + lo * 2)?;
        if cp < start {
            return Some(0);
        }
        let delta = t.u16(deltas + lo * 2)?;
        let ro_pos = ranges + lo * 2;
        let ro = t.u16(ro_pos)? as usize;
        if ro == 0 {
            return Some(cp.wrapping_add(delta));
        }
        let gpos = ro_pos + ro + (cp - start) as usize * 2;
        let g = t.u16(gpos)?;
        Some(if g == 0 { 0 } else { g.wrapping_add(delta) })
    }

    fn cmap12(&self, off: usize, cp: u32) -> Option<u16> {
        let t = self.cmap_data.from(off)?;
        let n = t.u32(12)? as usize;
        let (mut lo, mut hi) = (0usize, n);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let g = 16 + mid * 12;
            let start = t.u32(g)?;
            let end = t.u32(g + 4)?;
            if cp < start {
                hi = mid;
            } else if cp > end {
                lo = mid + 1;
            } else {
                let gid = t.u32(g + 8)?.checked_add(cp - start)?;
                return Some(u16::try_from(gid).unwrap_or(0));
            }
        }
        Some(0)
    }

    pub fn advance(&self, gid: u16) -> u16 {
        let i = gid.min(self.num_hmetrics - 1) as usize;
        self.hmtx.u16(i * 4).unwrap_or(0)
    }

    pub fn has_kern_table(&self) -> bool {
        self.kern.is_some()
    }

    pub fn kern(&self, left: u16, right: u16) -> i16 {
        let Some((d, n)) = self.kern else { return 0 };
        let key = (left as u32) << 16 | right as u32;
        let (mut lo, mut hi) = (0usize, n);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let Some(k) = d.u32(mid * 6) else { return 0 };
            if k < key {
                lo = mid + 1;
            } else if k > key {
                hi = mid;
            } else {
                return d.i16(mid * 6 + 4).unwrap_or(0);
            }
        }
        0
    }

    /// Data glyfu v tabulce `glyf` (prazdna = glyf bez obrysu, napr. mezera).
    pub fn glyph_data(&self, gid: u16) -> Option<Data<'a>> {
        if gid >= self.num_glyphs {
            return None;
        }
        let i = gid as usize;
        let (start, end) = if self.long_loca {
            (self.loca.u32(i * 4)? as usize, self.loca.u32(i * 4 + 4)? as usize)
        } else {
            (self.loca.u16(i * 2)? as usize * 2, self.loca.u16(i * 2 + 2)? as usize * 2)
        };
        if end < start {
            return None;
        }
        self.glyf.slice(start, end - start)
    }
}
