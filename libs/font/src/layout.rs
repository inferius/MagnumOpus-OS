//! OpenType rozvrzeni bez shapingu: parovy kerning z `GPOS` (lookup typu 2,
//! formaty 1 a 2, i zabaleny v rozsireni typu 9) a jednoduche zameny
//! z `GSUB` (typ 1, i v rozsireni typu 7) pro zapnute vlastnosti (`tnum`,
//! `cv11`).
//!
//! Tabulky jsou neduveryhodny vstup: kazde cteni jde pres [`Data`]
//! s kontrolou mezi, chyba = zadna uprava. Pocet lookupu a podtabulek je
//! omezeny, takze ani zamerne poskozeny soubor nezpusobi dlouhy beh.

use alloc::vec::Vec;

use crate::tables::Data;

/// Nejvic lookupu jedne vlastnosti a podtabulek jednoho lookupu.
const MAX_LOOKUPS: usize = 64;
const MAX_SUBTABLES: usize = 256;

/// Tabulka `GPOS` nebo `GSUB` s vybranymi lookupy.
#[derive(Clone, Default)]
pub struct Layout<'a> {
    data: Option<Data<'a>>,
    /// Indexy lookupu (serazene, bez opakovani).
    lookups: Vec<u16>,
}

impl<'a> Layout<'a> {
    /// Lookupy vlastnosti `tags` z tabulky `GPOS`/`GSUB` (`None` = tabulka
    /// chybi nebo je poskozena).
    pub fn new(data: Option<Data<'a>>, tags: &[[u8; 4]]) -> Self {
        let Some(d) = data else { return Self::default() };
        let mut lookups = Vec::new();
        let _ = (|| -> Option<()> {
            let feats = d.from(d.u16(6)? as usize)?;
            let n = feats.u16(0)? as usize;
            for i in 0..n.min(1024) {
                let rec = 2 + i * 6;
                let tag = feats.slice(rec, 4)?.0;
                if !tags.iter().any(|t| t == tag) {
                    continue;
                }
                let f = feats.from(feats.u16(rec + 4)? as usize)?;
                let k = f.u16(2)? as usize;
                for j in 0..k.min(MAX_LOOKUPS) {
                    let li = f.u16(4 + j * 2)?;
                    if !lookups.contains(&li) && lookups.len() < MAX_LOOKUPS {
                        lookups.push(li);
                    }
                }
            }
            Some(())
        })();
        lookups.sort_unstable();
        Self { data: Some(d), lookups }
    }

    pub fn is_empty(&self) -> bool {
        self.lookups.is_empty()
    }

    /// Podtabulky lookupu `li`: (typ, data podtabulky) s rozbalenym
    /// rozsirenim (`ext` = typ rozsireni: 9 v GPOS, 7 v GSUB).
    fn subtables(&self, li: u16, ext: u16, mut f: impl FnMut(u16, Data<'a>) -> bool) -> Option<()> {
        let d = self.data?;
        let list = d.from(d.u16(8)? as usize)?;
        if li >= list.u16(0)? {
            return None;
        }
        let lookup = list.from(list.u16(2 + li as usize * 2)? as usize)?;
        let ty = lookup.u16(0)?;
        let n = lookup.u16(4)? as usize;
        for i in 0..n.min(MAX_SUBTABLES) {
            let st = lookup.from(lookup.u16(6 + i * 2)? as usize)?;
            let (ty, st) = if ty == ext {
                if st.u16(0)? != 1 {
                    continue;
                }
                (st.u16(2)?, st.from(st.u32(4)? as usize)?)
            } else {
                (ty, st)
            };
            if f(ty, st) {
                break;
            }
        }
        Some(())
    }

    /// Soucet uprav posunu dvojice (jednotky pisma) ze vsech lookupu
    /// kerningu; v kazdem lookupu plati prvni podtabulka, ktera dvojici zna.
    pub fn pair(&self, left: u16, right: u16) -> i32 {
        let mut total = 0i32;
        for &li in &self.lookups {
            let _ = self.subtables(li, 9, |ty, st| {
                if ty != 2 {
                    return false;
                }
                match pair_pos(st, left, right) {
                    Some(v) => {
                        total += v;
                        true
                    }
                    None => false,
                }
            });
        }
        total.clamp(i16::MIN as i32, i16::MAX as i32)
    }

    /// Glyf po jednoduchych zamenach vsech vybranych lookupu.
    pub fn single(&self, mut gid: u16) -> u16 {
        for &li in &self.lookups {
            let _ = self.subtables(li, 7, |ty, st| {
                if ty != 1 {
                    return false;
                }
                match single_subst(st, gid) {
                    Some(g) => {
                        gid = g;
                        true
                    }
                    None => false,
                }
            });
        }
        gid
    }
}

/// Index glyfu v pokryti (`None` = neni pokryty).
fn coverage(c: Data<'_>, gid: u16) -> Option<usize> {
    match c.u16(0)? {
        1 => {
            let n = c.u16(2)? as usize;
            let (mut lo, mut hi) = (0usize, n);
            while lo < hi {
                let mid = (lo + hi) / 2;
                let g = c.u16(4 + mid * 2)?;
                if g < gid {
                    lo = mid + 1;
                } else if g > gid {
                    hi = mid;
                } else {
                    return Some(mid);
                }
            }
            None
        }
        2 => {
            let n = c.u16(2)? as usize;
            let (mut lo, mut hi) = (0usize, n);
            while lo < hi {
                let mid = (lo + hi) / 2;
                let r = 4 + mid * 6;
                let (start, end) = (c.u16(r)?, c.u16(r + 2)?);
                if end < gid {
                    lo = mid + 1;
                } else if start > gid {
                    hi = mid;
                } else {
                    return Some(c.u16(r + 4)? as usize + (gid - start) as usize);
                }
            }
            None
        }
        _ => None,
    }
}

/// Trida glyfu (0 = zadna).
fn class(c: Data<'_>, gid: u16) -> u16 {
    let f = || -> Option<u16> {
        match c.u16(0)? {
            1 => {
                let start = c.u16(2)?;
                let n = c.u16(4)?;
                if gid < start || gid - start >= n {
                    return Some(0);
                }
                c.u16(6 + (gid - start) as usize * 2)
            }
            2 => {
                let n = c.u16(2)? as usize;
                let (mut lo, mut hi) = (0usize, n);
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    let r = 4 + mid * 6;
                    let (start, end) = (c.u16(r)?, c.u16(r + 2)?);
                    if end < gid {
                        lo = mid + 1;
                    } else if start > gid {
                        hi = mid;
                    } else {
                        return c.u16(r + 4);
                    }
                }
                Some(0)
            }
            _ => Some(0),
        }
    };
    f().unwrap_or(0)
}

/// Velikost zaznamu hodnot (bajty) podle `ValueFormat`.
fn value_size(vf: u16) -> usize {
    (vf & 0xFF).count_ones() as usize * 2
}

/// Posun (XAdvance) a umisteni (XPlacement) ze zaznamu hodnot.
fn value_x(d: Data<'_>, off: usize, vf: u16) -> Option<(i32, i32)> {
    let pla = if vf & 1 != 0 { d.i16(off)? as i32 } else { 0 };
    let adv = if vf & 4 != 0 { d.i16(off + 2 * (vf & 3).count_ones() as usize)? as i32 } else { 0 };
    Some((adv, pla))
}

/// Parova uprava jedne podtabulky (`None` = dvojici nezna).
fn pair_pos(st: Data<'_>, left: u16, right: u16) -> Option<i32> {
    let format = st.u16(0)?;
    let ci = coverage(st.from(st.u16(2)? as usize)?, left)?;
    let (vf1, vf2) = (st.u16(4)?, st.u16(6)?);
    let (s1, s2) = (value_size(vf1), value_size(vf2));
    let rec = |d: Data<'_>, off: usize| -> Option<i32> {
        let (adv1, _) = value_x(d, off, vf1)?;
        let (_, pla2) = value_x(d, off + s1, vf2)?;
        Some(adv1 + pla2)
    };
    match format {
        1 => {
            let sets = st.u16(8)? as usize;
            if ci >= sets {
                return None;
            }
            let set = st.from(st.u16(10 + ci * 2)? as usize)?;
            let n = set.u16(0)? as usize;
            let size = 2 + s1 + s2;
            let (mut lo, mut hi) = (0usize, n);
            while lo < hi {
                let mid = (lo + hi) / 2;
                let r = 2 + mid * size;
                let g = set.u16(r)?;
                if g < right {
                    lo = mid + 1;
                } else if g > right {
                    hi = mid;
                } else {
                    return rec(set, r + 2);
                }
            }
            None
        }
        2 => {
            let cd1 = st.from(st.u16(8)? as usize)?;
            let cd2 = st.from(st.u16(10)? as usize)?;
            let (n1, n2) = (st.u16(12)? as usize, st.u16(14)? as usize);
            let (c1, c2) = (class(cd1, left) as usize, class(cd2, right) as usize);
            if c1 >= n1 || c2 >= n2 {
                return None;
            }
            let off = 16 + (c1 * n2 + c2) * (s1 + s2);
            rec(st, off)
        }
        _ => None,
    }
}

/// Jednoducha zamena jedne podtabulky (`None` = glyf nepokryva).
fn single_subst(st: Data<'_>, gid: u16) -> Option<u16> {
    let ci = coverage(st.from(st.u16(2)? as usize)?, gid)?;
    match st.u16(0)? {
        1 => Some(gid.wrapping_add(st.u16(4)?)),
        2 => {
            if ci >= st.u16(4)? as usize {
                return None;
            }
            st.u16(6 + ci * 2)
        }
        _ => None,
    }
}
