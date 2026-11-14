//! Oblasti adresniho prostoru: rezervace, castecne odmapovani, zmena prav
//! a dotaz (`VM_RESERVE`, `VM_UNMAP`, `VM_PROTECT`, `VM_QUERY`).
//!
//! Oblast je mapovani objektu pameti nebo rezervace. Operace na rozsahu
//! uvnitr oblasti ji nejdriv rozdeli na hranicich rozsahu (`split`); kazdy
//! kus ma vlastni strzeni z rozpoctu jadra jobu. Rozdeleni muze selhat na
//! rozpoctu - pak operace skonci chybou a nic dalsiho se nezmeni (uz
//! provedene rozdeleni nevadi, kusy dohromady znamenaji totez).

use alloc::sync::Arc;
use alloc::vec::Vec;

use abi::vm::{MAP_FIXED, RESERVE_GUARD, RegionInfo, UNMAP_DECOMMIT, USER_MAX, USER_MIN, region};

use super::{AddressSpace, Inner, MAPPING_COST, Mapping, PROT, page_aligned, region_cost};
use crate::arch::paging::{PAGE_SIZE, PageTableOps};
use crate::error::Error;
use crate::mm::{VirtAddr, tlb};
use crate::object::job::{Charge, Job, Pool};

fn charge(job: &Option<Arc<Job>>, bytes: u64) -> Result<Charge, Error> {
    match job {
        Some(j) => j.charge(Pool::Kernel, bytes),
        None => Ok(Charge::none()),
    }
}

/// Overi zarovnany rozsah v pameti procesu; vraci konec.
fn user_range(addr: u64, len: u64) -> Result<u64, Error> {
    if len == 0 || !page_aligned(addr) || !page_aligned(len) {
        return Err(Error::InvalidArgs);
    }
    let end = addr.checked_add(len).ok_or(Error::InvalidArgs)?;
    if addr < USER_MIN || end > USER_MAX {
        return Err(Error::InvalidArgs);
    }
    Ok(end)
}

impl Inner {
    /// Rozdeli oblast obsahujici `at` (uvnitr, ne na zacatku) na dve.
    pub(super) fn split(&mut self, at: u64, job: &Option<Arc<Job>>) -> Result<(), Error> {
        let Some((start, m)) = self.mapping_at(at) else {
            return Ok(());
        };
        if start == at {
            return Ok(());
        }
        let (first, second) = (at - start, start + m.len - at);
        let mapped = m.vmo.is_some();
        let new_charge = charge(job, region_cost(second, mapped))?;
        let m = self.mappings.get_mut(&start).expect("nalezeno vyse");
        let tail = Mapping {
            len: second,
            vmo: m.vmo.clone(),
            vmo_offset: m.vmo_offset + first,
            flags: m.flags,
            max_flags: m.max_flags,
            guard: m.guard,
            in_reservation: m.in_reservation,
            charge: new_charge,
        };
        let old_cost = region_cost(m.len, mapped);
        m.len = first;
        m.charge.shrink(old_cost.saturating_sub(region_cost(first, mapped)));
        self.mappings.insert(at, tail);
        Ok(())
    }

    /// Pokryvaji oblasti cely rozsah bez der? Vraci zacatky oblasti, ktere
    /// rozsah protina (prvni muze zacinat pred `addr`).
    fn covering(&self, addr: u64, end: u64) -> Option<Vec<u64>> {
        let mut starts = Vec::new();
        let mut pos = addr;
        while pos < end {
            let (start, m) = self.mapping_at(pos)?;
            starts.push(start);
            pos = start + m.len;
        }
        Some(starts)
    }

    /// Lezi cely rozsah v rezervacich (souvisle)?
    pub(super) fn all_reserved(&self, addr: u64, end: u64) -> bool {
        self.covering(addr, end)
            .is_some_and(|s| s.iter().all(|st| self.mappings[st].vmo.is_none()))
    }
}

impl AddressSpace {
    /// Zarezervuje `len` bajtu adres (bez pameti). `MAP_FIXED` = presne na
    /// `addr`, jinak volne misto (`addr` napoveda); `RESERVE_GUARD` =
    /// ochranna oblast (pristup se hlasi jako preteceni zasobniku). Stoji
    /// jen zaznam oblasti z rozpoctu jadra, tabulky stranek ani pamet ne.
    pub fn reserve(&self, addr: u64, len: u64, flags: u32) -> Result<u64, Error> {
        if flags & !(MAP_FIXED | RESERVE_GUARD) != 0 || len == 0 || !page_aligned(len) {
            return Err(Error::InvalidArgs);
        }
        let charge = charge(&self.job, MAPPING_COST)?;
        let mut inner = self.inner.lock();
        if inner.table.is_none() {
            return Err(Error::BadState);
        }
        let start = if flags & MAP_FIXED != 0 {
            user_range(addr, len)?;
            if inner.overlaps(addr, len) {
                return Err(Error::BadState);
            }
            addr
        } else {
            inner.find_free(len, addr).ok_or(Error::NoResources)?
        };
        inner
            .mappings
            .insert(start, Mapping::reservation(len, flags & RESERVE_GUARD != 0, charge));
        Ok(start)
    }

    /// Zrusi oblasti v rozsahu (i jejich casti). Rozsah musi byt cely
    /// pokryty oblastmi (mapovani i rezervace), jinak `NotFound`.
    /// `UNMAP_DECOMMIT`: mapovani se zmeni na rezervaci (adresy zustanou
    /// zabrane, pamet se vrati), rezervace zustanou.
    pub fn unmap(&self, addr: u64, len: u64, flags: u32) -> Result<(), Error> {
        if flags & !UNMAP_DECOMMIT != 0 {
            return Err(Error::InvalidArgs);
        }
        let decommit = flags & UNMAP_DECOMMIT != 0;
        let end = addr.checked_add(len).ok_or(Error::InvalidArgs)?;
        if len == 0 || !page_aligned(addr) || !page_aligned(len) {
            return Err(Error::InvalidArgs);
        }
        let mut inner = self.inner.lock();
        inner.covering(addr, end).ok_or(Error::NotFound)?;
        inner.split(addr, &self.job)?;
        inner.split(end, &self.job)?;
        let starts: Vec<u64> = inner.mappings.range(addr..end).map(|(&s, _)| s).collect();
        let mut removed = Vec::new();
        let mut flush = false;
        for s in starts {
            let m = inner.mappings.remove(&s).expect("nalezeno vyse");
            let Mapping { len, vmo, guard, mut charge, .. } = m;
            match vmo {
                Some(vmo) => {
                    let table = inner.table.as_mut().ok_or(Error::BadState)?;
                    for off in (0..len).step_by(PAGE_SIZE as usize) {
                        table.unmap_4k(VirtAddr::new(s + off));
                    }
                    flush = true;
                    removed.push(vmo);
                    if decommit {
                        charge.shrink(region_cost(len, true) - region_cost(len, false));
                        inner.mappings.insert(s, Mapping::reservation(len, false, charge));
                    }
                }
                None if decommit => {
                    inner.mappings.insert(s, Mapping::reservation(len, guard, charge));
                }
                None => {}
            }
        }
        drop(inner);
        // Stranky (objekty v `removed`) se smi uvolnit az kdyz je zadne CPU
        // nema v TLB.
        if flush {
            tlb::shootdown();
        }
        drop(removed);
        Ok(())
    }

    /// Zmeni prava mapovani v rozsahu (i casti). `prot` je podmnozina
    /// `MAP_READ | MAP_WRITE | MAP_EXECUTE` (0 = zadny pristup), nikdy W+X
    /// a nikdy nad prava pri `VM_MAP` (`AccessDenied`). Rozsah musi byt cely
    /// v mapovanich (diry `NotFound`, rezervace `BadState`). Namapovane
    /// stranky se odmapuji a vypadek je doplni s novymi pravy.
    /// Spusteni na nezapecetenem objektu jen s `jit` (opravneni procesu);
    /// vraci, zda k tomu doslo (audit).
    pub fn protect(&self, addr: u64, len: u64, prot: u32, jit: bool) -> Result<bool, Error> {
        if prot & !PROT != 0 || (prot & abi::vm::MAP_WRITE != 0 && prot & abi::vm::MAP_EXECUTE != 0) {
            return Err(Error::InvalidArgs);
        }
        let end = user_range(addr, len)?;
        let mut inner = self.inner.lock();
        let starts = inner.covering(addr, end).ok_or(Error::NotFound)?;
        let mut jit_used = false;
        for s in &starts {
            let m = &inner.mappings[s];
            let Some(vmo) = &m.vmo else {
                return Err(Error::BadState);
            };
            if prot & !m.max_flags != 0 {
                return Err(Error::AccessDenied);
            }
            if prot & abi::vm::MAP_EXECUTE != 0 && !vmo.is_sealed() {
                if !jit {
                    return Err(Error::AccessDenied);
                }
                jit_used = true;
            }
        }
        inner.split(addr, &self.job)?;
        inner.split(end, &self.job)?;
        let starts: Vec<u64> = inner.mappings.range(addr..end).map(|(&s, _)| s).collect();
        let Inner { table, mappings } = &mut *inner;
        let table = table.as_mut().ok_or(Error::BadState)?;
        for s in starts {
            let m = mappings.get_mut(&s).expect("nalezeno vyse");
            m.flags = prot;
            for off in (0..m.len).step_by(PAGE_SIZE as usize) {
                table.unmap_4k(VirtAddr::new(s + off));
            }
        }
        drop(inner);
        tlb::shootdown();
        Ok(jit_used)
    }

    /// Oblast na adrese `addr`: mapovani, rezervace, nebo volne misto mezi
    /// sousednimi oblastmi.
    pub fn query(&self, addr: u64) -> Result<RegionInfo, Error> {
        if !(USER_MIN..USER_MAX).contains(&addr) {
            return Err(Error::InvalidArgs);
        }
        let inner = self.inner.lock();
        if let Some((start, m)) = inner.mapping_at(addr) {
            let mut attrs = 0;
            if m.guard {
                attrs |= region::ATTR_GUARD;
            }
            if m.in_reservation {
                attrs |= region::ATTR_IN_RESERVATION;
            }
            return Ok(RegionInfo {
                start,
                len: m.len,
                kind: if m.vmo.is_some() { region::MAPPED } else { region::RESERVED },
                flags: m.flags,
                max_flags: m.max_flags,
                attrs,
                vmo_offset: m.vmo_offset,
            });
        }
        let start = inner
            .mappings
            .range(..addr)
            .next_back()
            .map_or(USER_MIN, |(&s, m)| s + m.len);
        let end = inner.mappings.range(addr..).next().map_or(USER_MAX, |(&s, _)| s);
        Ok(RegionInfo {
            start,
            len: end - start,
            kind: region::FREE,
            ..RegionInfo::default()
        })
    }

    /// Lezi `addr` v ochranne rezervaci (`RESERVE_GUARD`)?
    pub fn is_guard(&self, addr: u64) -> bool {
        self.inner
            .lock()
            .mapping_at(addr)
            .is_some_and(|(_, m)| m.vmo.is_none() && m.guard)
    }
}
