//! Tabulka sdileni bloku (faze 3): pocet odkazu na datove useky, na ktere
//! ukazuje vic extentu (okamzite kopie, deduplikace).
//!
//! Zaznam (`item::SHARE`, strom korenoveho datasetu) pokryva souvisly
//! fyzicky usek s poctem odkazu. Usek bez zaznamu ma jedineho vlastnika
//! (dosavadni pravidla, `Store::release`). Kazdy odkaz se jednou
//! definitivne uvolni (z extentu nebo z deadlistu) - pak se v `flush_unref`
//! pocet snizi a usek se uvolni az s poslednim odkazem (i s kopii).
//!
//! Zaznamy se neprekryvaji a nejsou delsi nez `MAX_EXTENT` (vznikaji
//! z extentu), takze prekryti useku najde rozsah klicu od
//! `start - (MAX_EXTENT - 1)`. Castecne snizeni zaznam rozdeli; indexovany
//! zaznam (deduplikace) pri rozdeleni ztrati polozku indexu (kus uz neni
//! cely).

use alloc::vec::Vec;

use abi::Result;

use super::Fs;
use crate::btree;
use crate::datarec::{ShareEntry, decode_dedup, dedup_key, encode_dedup};
use crate::device::BlockDevice;
use crate::hash::Csum;
use crate::layout::{Key, MAX_EXTENT, META_OID, ROOT_DATASET, item};

fn share_key(start: u64) -> Key {
    Key::new(META_OID, item::SHARE, start)
}

impl<D: BlockDevice> Fs<D> {
    /// Provede `f` s aktivnim korenovym datasetem (tabulka sdileni a index
    /// lezi v jeho strome) a vrati puvodni aktivni dataset.
    pub(super) fn in_root<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let back = self.cur_ds;
        self.activate(ROOT_DATASET)?;
        let r = f(self);
        self.activate(back)?;
        r
    }

    /// Ma tabulka sdileni nejaky zaznam? (pri pripojeni: `share_live`)
    pub(super) fn share_any(&mut self) -> Result<bool> {
        self.in_root(|fs| {
            let root = fs.s.root;
            let mut any = false;
            btree::range(
                &mut fs.s,
                &root,
                &share_key(0),
                &Key::new(META_OID, item::SHARE + 1, 0),
                &mut |_, _| {
                    any = true;
                    Ok(false)
                },
            )?;
            Ok(any)
        })
    }

    /// Zaznamy prekryvajici `start..start+len` (aktivni = korenovy dataset).
    fn share_overlaps(&mut self, start: u64, len: u64) -> Result<Vec<(u64, ShareEntry)>> {
        let root = self.s.root;
        let end = start + len;
        let mut out = Vec::new();
        let mut bad = false;
        btree::range(
            &mut self.s,
            &root,
            &share_key(start.saturating_sub(MAX_EXTENT - 1)),
            &share_key(end),
            &mut |k, v| {
                match ShareEntry::decode(v) {
                    Ok(e) if k.offset + e.len as u64 > start => out.push((k.offset, e)),
                    Ok(_) => {}
                    Err(_) => {
                        bad = true;
                        return Ok(false);
                    }
                }
                Ok(true)
            },
        )?;
        if bad {
            return Err(abi::Error::Corrupted);
        }
        Ok(out)
    }

    /// Odebere zaznam `s` (a jeho polozku indexu deduplikace).
    fn share_remove(&mut self, s: u64, e: &ShareEntry) -> Result<()> {
        self.del(&share_key(s))?;
        if let Some(h) = e.hash {
            self.dedup_unindex(&h, s)?;
        }
        Ok(())
    }

    /// Zapise cast `s..s+len` zaznamu `e` s poctem `refs` (bez hashe -
    /// rozdeleny kus uz neni indexovatelny).
    fn share_put_part(&mut self, s: u64, len: u64, e: &ShareEntry, base: u64, refs: u32) -> Result<()> {
        let part = ShareEntry {
            len: len as u32,
            refs,
            copy: if e.copy != 0 { e.copy + (s - base) } else { 0 },
            hash: None,
        };
        self.put(share_key(s), part.encode())
    }

    /// Zmeni pocet odkazu useku `start..start+len` o `delta` (+1 nova kopie,
    /// -1 uvolneni odkazu). Nepokryte casti: +1 zalozi zaznam s 2 odkazy,
    /// -1 je vlastni jediny odkaz - vraci se k uvolneni. Vraci useky
    /// (zacatek, delka, kopie), ktere uz nikdo nepouziva. `copy` = kopie
    /// useku (0 = neni).
    fn share_adjust(&mut self, start: u64, len: u64, copy: u64, delta: i32) -> Result<Vec<(u64, u64, u64)>> {
        let mut free = Vec::new();
        let end = start + len;
        let mut pos = start;
        let cp = |p: u64| if copy != 0 { copy + (p - start) } else { 0 };
        for (s, e) in self.share_overlaps(start, len)? {
            let se = s + e.len as u64;
            // Nepokryty kus pred zaznamem.
            if pos < s {
                let gap = s.min(end) - pos;
                self.share_gap(pos, gap, cp(pos), delta, &mut free)?;
            }
            let lo = s.max(start);
            let hi = se.min(end);
            let refs = (e.refs as i64 + delta as i64).max(0) as u32;
            if lo == s && hi == se {
                // Cely zaznam.
                if refs == 0 {
                    self.share_remove(s, &e)?;
                    free.push((s, e.len as u64, e.copy));
                } else {
                    let mut n = e;
                    n.refs = refs;
                    self.put(share_key(s), n.encode())?;
                }
            } else {
                // Rozdeleni: [s, lo) a [hi, se) s puvodnim poctem.
                self.share_remove(s, &e)?;
                if s < lo {
                    self.share_put_part(s, lo - s, &e, s, e.refs)?;
                }
                if refs == 0 {
                    let c = if e.copy != 0 { e.copy + (lo - s) } else { 0 };
                    free.push((lo, hi - lo, c));
                } else {
                    self.share_put_part(lo, hi - lo, &e, s, refs)?;
                }
                if hi < se {
                    self.share_put_part(hi, se - hi, &e, s, e.refs)?;
                }
            }
            pos = hi;
        }
        if pos < end {
            self.share_gap(pos, end - pos, cp(pos), delta, &mut free)?;
        }
        Ok(free)
    }

    fn share_gap(&mut self, s: u64, len: u64, copy: u64, delta: i32, free: &mut Vec<(u64, u64, u64)>) -> Result<()> {
        if delta < 0 {
            free.push((s, len, copy));
        } else {
            let e = ShareEntry {
                len: len as u32,
                refs: 1 + delta as u32,
                copy,
                hash: None,
            };
            self.put(share_key(s), e.encode())?;
            self.s.share_live = true;
        }
        Ok(())
    }

    /// Prida odkaz na datovy usek (nova okamzita kopie nebo deduplikace).
    pub(super) fn share_ref(&mut self, start: u64, len: u64, copy: u64) -> Result<()> {
        self.in_root(|fs| {
            fs.share_adjust(start, len, copy, 1)?;
            Ok(())
        })
    }

    /// Zpracuje frontu definitivne uvolnenych datovych useku: snizi pocty
    /// v tabulce sdileni a uvolni, co uz nikdo nepouziva. Volat pred
    /// commitem (`commit_all`).
    pub(super) fn flush_unref(&mut self) -> Result<()> {
        if self.s.unref.is_empty() {
            return Ok(());
        }
        self.in_root(|fs| {
            // Zmena tabulky samotne uvolnuje jen uzly (ty nejsou sdilene),
            // presto ve smycce: fronta se mohla doplnit.
            while !fs.s.unref.is_empty() {
                let list = core::mem::take(&mut fs.s.unref);
                for (start, len, copy) in list {
                    for (s, l, c) in fs.share_adjust(start, len, copy, -1)? {
                        let txg = fs.s.txg;
                        fs.s.space.free(s, l, txg, false);
                        if c != 0 {
                            fs.s.space.free(c, l, txg, false);
                        }
                    }
                }
            }
            Ok(())
        })
    }

    /// Zalozi indexovany zaznam pro cely kus `start..start+len` s jedinym
    /// odkazem (deduplikace: kus je kandidat pro dalsi shody).
    pub(super) fn share_index(&mut self, start: u64, len: u64, copy: u64, hash: Csum) -> Result<()> {
        self.in_root(|fs| {
            if !fs.share_overlaps(start, len)?.is_empty() {
                return Ok(());
            }
            let e = ShareEntry {
                len: len as u32,
                refs: 1,
                copy,
                hash: Some(hash),
            };
            fs.put(share_key(start), e.encode())?;
            fs.s.share_live = true;
            let k = Key::new(META_OID, item::DEDUP, dedup_key(&hash));
            let root = fs.s.root;
            let mut list = match btree::get(&mut fs.s, &root, &k)? {
                Some(v) => decode_dedup(&v)?,
                None => Vec::new(),
            };
            list.push((hash, start));
            fs.put(k, encode_dedup(&list))
        })
    }

    /// Kus se stejnym hashem v indexu: (zacatek, zaznam). Aktivni dataset
    /// muze byt libovolny.
    pub(super) fn dedup_lookup(&mut self, hash: &Csum) -> Result<Option<(u64, ShareEntry)>> {
        self.in_root(|fs| {
            let root = fs.s.root;
            let Some(v) = btree::get(&mut fs.s, &root, &Key::new(META_OID, item::DEDUP, dedup_key(hash)))? else {
                return Ok(None);
            };
            for (h, start) in decode_dedup(&v)? {
                if h != *hash {
                    continue;
                }
                let Some(raw) = btree::get(&mut fs.s, &root, &share_key(start))? else {
                    return Err(abi::Error::Corrupted);
                };
                let e = ShareEntry::decode(&raw)?;
                if e.hash != Some(h) {
                    return Err(abi::Error::Corrupted);
                }
                return Ok(Some((start, e)));
            }
            Ok(None)
        })
    }

    /// Odebere `start` z polozky indexu pro `hash` (aktivni = korenovy).
    fn dedup_unindex(&mut self, hash: &Csum, start: u64) -> Result<()> {
        let k = Key::new(META_OID, item::DEDUP, dedup_key(hash));
        let root = self.s.root;
        let Some(v) = btree::get(&mut self.s, &root, &k)? else {
            return Ok(());
        };
        let mut list = decode_dedup(&v)?;
        list.retain(|(h, s)| !(h == hash && *s == start));
        if list.is_empty() {
            self.del(&k)?;
            Ok(())
        } else {
            self.put(k, encode_dedup(&list))
        }
    }
}
