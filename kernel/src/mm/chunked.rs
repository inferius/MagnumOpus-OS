//! Dvouurovnove pole: prvky v blocich pevne velikosti (vychozi 4 KiB).
//!
//! Heap jadra vraci souvisle bloky nejvys 4 MiB (`phys::MAX_ORDER`) a obycejny
//! `Vec` pri rustu kopiruje vse do dvakrat vetsiho bloku. Velke tabulky
//! (sloty handlu, stranky objektu pameti) proto lezi v blocich: rust nikdy
//! nekopiruje cela data a nepotrebuje velky souvisly blok. Posledni blok
//! se alokuje jen tak velky, kolik je potreba (maly objekt nezabere cely
//! blok).
//!
//! Alokace jsou omylne (`try_reserve`), pri nedostatku pameti vraci
//! `NoMemory` misto paniky.

use alloc::vec::Vec;

use crate::error::Error;

pub struct ChunkedVec<T, const CHUNK_BYTES: usize = 4096> {
    chunks: Vec<Vec<T>>,
    len: usize,
}

impl<T, const C: usize> Default for ChunkedVec<T, C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const CHUNK_BYTES: usize> ChunkedVec<T, CHUNK_BYTES> {
    /// Prvku v jednom bloku.
    pub const PER_CHUNK: usize = {
        let size = core::mem::size_of::<T>();
        let n = if size == 0 { CHUNK_BYTES } else { CHUNK_BYTES / size };
        if n == 0 { 1 } else { n }
    };

    pub const fn new() -> Self {
        Self {
            chunks: Vec::new(),
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// Pripravi misto pro `additional` dalsich prvku (pak `push` nealokuje).
    pub fn try_reserve(&mut self, additional: usize) -> Result<(), Error> {
        let per = Self::PER_CHUNK;
        let need = self.len.checked_add(additional).ok_or(Error::NoMemory)?;
        let chunks = need.div_ceil(per);
        if chunks > self.chunks.len() {
            self.chunks
                .try_reserve(chunks - self.chunks.len())
                .map_err(|_| Error::NoMemory)?;
            while self.chunks.len() < chunks {
                self.chunks.push(Vec::new());
            }
        }
        // Bloky od aktualniho konce az po posledni potrebny.
        for i in self.len / per..chunks {
            let target = (need - i * per).min(per);
            let c = &mut self.chunks[i];
            if c.capacity() < target {
                let more = target - c.len();
                // Plny blok presne, castecny s rezervou na dalsi push (ale
                // nikdy pres velikost bloku).
                let r = if target == per {
                    c.try_reserve_exact(more)
                } else {
                    c.try_reserve(more.max(c.len()).min(per - c.len()))
                };
                r.map_err(|_| Error::NoMemory)?;
            }
        }
        Ok(())
    }

    pub fn push(&mut self, value: T) -> Result<(), Error> {
        self.try_reserve(1)?;
        self.chunks[self.len / Self::PER_CHUNK].push(value);
        self.len += 1;
        Ok(())
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len {
            return None;
        }
        self.chunks[index / Self::PER_CHUNK].get(index % Self::PER_CHUNK)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.len {
            return None;
        }
        self.chunks[index / Self::PER_CHUNK].get_mut(index % Self::PER_CHUNK)
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.chunks.iter().flat_map(|c| c.iter())
    }
}

impl<T, const C: usize> core::ops::Index<usize> for ChunkedVec<T, C> {
    type Output = T;
    fn index(&self, index: usize) -> &T {
        self.get(index).expect("index mimo ChunkedVec")
    }
}

impl<T, const C: usize> core::ops::IndexMut<usize> for ChunkedVec<T, C> {
    fn index_mut(&mut self, index: usize) -> &mut T {
        self.get_mut(index).expect("index mimo ChunkedVec")
    }
}
