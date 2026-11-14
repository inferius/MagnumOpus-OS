//! Alokator rozsahu adres pro zarizeni (IOVA) v jedne domene IOMMU.
//!
//! Jadro mapuje pamet do domeny zarizeni dvema zpusoby:
//! - identicky (adresa pro zarizeni = fyzicka): souvisla pamet pro DMA
//!   (`VMO_CREATE_CONTIGUOUS`); zabira adresy, kde lezi RAM,
//! - na zvolenou adresu (`DMA_MAP`): nesouvisle stranky za sebou v jednom
//!   souvislem rozsahu IOVA. Rozsahy vybira tento alokator z okna nad
//!   koncem RAM, takze se s identickymi mapovanimi nikdy neprekryji.
//!
//! Pravidla:
//! - vse po strankach (4 KiB), rozsah nikdy neprekroci konec okna,
//! - za kazdym obsazenym rozsahem zustava pri hledani volna ochranna
//!   stranka (zarizeni, ktere pretece konec bufferu, narazi na chybu
//!   prekladu misto dat sousedniho bufferu),
//! - rozsah zvoleny ovladacem (napoveda) se jen overi na prekryv,
//! - uvolneni podle zacatku; neznamy zacatek vrati `None`.
//!
//! Hledani je first-fit pres serazene obsazene rozsahy (O(n) v poctu
//! mapovani domeny; ovladac jich ma desitky az stovky).

#![no_std]

extern crate alloc;

#[cfg(test)]
mod tests;

use alloc::collections::BTreeMap;

/// Velikost stranky IOVA.
pub const PAGE: u64 = 4096;

/// Proc alokace selhala.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IovaError {
    /// Nulova delka, nezarovnany zacatek nebo delka, rozsah mimo okno.
    Invalid,
    /// Napoveda se prekryva s obsazenym rozsahem.
    Overlap,
    /// V okne neni dost mista.
    Full,
}

/// Obsazene rozsahy jednoho okna `[base, end)`.
#[derive(Debug)]
pub struct IovaSpace {
    base: u64,
    end: u64,
    /// Zacatek -> delka (bez ochranne stranky).
    used: BTreeMap<u64, u64>,
}

impl IovaSpace {
    /// Prazdne okno `[base, end)`; meze se zarovnaji dovnitr na stranky.
    pub fn new(base: u64, end: u64) -> Self {
        let base = base.div_ceil(PAGE).saturating_mul(PAGE);
        let end = (end / PAGE) * PAGE;
        Self {
            base,
            end: end.max(base),
            used: BTreeMap::new(),
        }
    }

    pub fn base(&self) -> u64 {
        self.base
    }

    pub fn end(&self) -> u64 {
        self.end
    }

    /// Pocet obsazenych rozsahu.
    pub fn len(&self) -> usize {
        self.used.len()
    }

    pub fn is_empty(&self) -> bool {
        self.used.is_empty()
    }

    fn check(&self, len: u64) -> Result<(), IovaError> {
        if len == 0 || !len.is_multiple_of(PAGE) || len > self.end - self.base {
            return Err(IovaError::Invalid);
        }
        Ok(())
    }

    /// Vybere rozsah `len` bajtu zarovnany na `align` (mocnina dvou,
    /// aspon stranka) s volnou ochrannou strankou za koncem.
    pub fn alloc(&mut self, len: u64, align: u64) -> Result<u64, IovaError> {
        self.check(len)?;
        if !align.is_power_of_two() {
            return Err(IovaError::Invalid);
        }
        let align = align.max(PAGE);
        // Potrebne misto vcetne ochranne stranky.
        let need = len.checked_add(PAGE).ok_or(IovaError::Invalid)?;
        let mut cursor = self.base;
        for (&start, &l) in &self.used {
            if let Some(at) = fit(cursor, start, need, align) {
                self.used.insert(at, len);
                return Ok(at);
            }
            // Za obsazenym rozsahem take ochranna stranka (i za napovedou).
            cursor = cursor.max(start.saturating_add(l).saturating_add(PAGE));
        }
        let at = fit(cursor, self.end, need, align).ok_or(IovaError::Full)?;
        self.used.insert(at, len);
        Ok(at)
    }

    /// Obsadi rozsah zvoleny volajicim (napoveda ovladace).
    pub fn reserve(&mut self, start: u64, len: u64) -> Result<(), IovaError> {
        self.check(len)?;
        let end = start.checked_add(len).ok_or(IovaError::Invalid)?;
        if !start.is_multiple_of(PAGE) || start < self.base || end > self.end {
            return Err(IovaError::Invalid);
        }
        // Predchozi rozsah nesmi zasahovat za `start`, nasledujici zacinat
        // pred `end`.
        if let Some((&s, &l)) = self.used.range(..=start).next_back()
            && s + l > start
        {
            return Err(IovaError::Overlap);
        }
        if self.used.range(start..end).next().is_some() {
            return Err(IovaError::Overlap);
        }
        self.used.insert(start, len);
        Ok(())
    }

    /// Uvolni rozsah zacinajici na `start`; vrati jeho delku.
    pub fn free(&mut self, start: u64) -> Option<u64> {
        self.used.remove(&start)
    }

    /// Lezi adresa v nekterem obsazenem rozsahu?
    pub fn contains(&self, addr: u64) -> bool {
        self.used
            .range(..=addr)
            .next_back()
            .is_some_and(|(&s, &l)| addr < s + l)
    }
}

/// Prvni zarovnana adresa od `from`, kde se `need` bajtu vejde pred `limit`.
fn fit(from: u64, limit: u64, need: u64, align: u64) -> Option<u64> {
    let at = from.checked_next_multiple_of(align)?;
    (at.checked_add(need)? <= limit).then_some(at)
}
