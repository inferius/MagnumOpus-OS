//! Sdileny kruhovy buffer streamu v objektu pameti (VMO).
//!
//! Vytvari ho sluzba (`audiod`) - velikost a rozlozeni tedy urcuje ona,
//! ne klient. Klient i sluzba si ho namapuji; pres kanal jdou jen prikazy,
//! vzorky nikdy (zadne kopirovani pres jadro).
//!
//! Rozlozeni (little-endian):
//!
//! ```text
//! 0     magic u32 "AUR1", verze u32
//! 8     kapacita (snimky) u32, bajtu na snimek u32
//! 16    offset dat u32 (= 4096), smer u32
//! 64    write_pos u64   - pise jen producent (cache line pro sebe)
//! 128   read_pos u64    - pise jen konzument
//! 192   underruns u64   - sluzba: kolikrat chybela data (prehravani)
//! 200   overruns u64    - sluzba: kolikrat nebylo misto (nahravani)
//! 208   played u64      - sluzba: snimky streamu, ktere uz zazneli
//! 216   latency_us u64  - sluzba: odhad latence od zapisu po reproduktor
//! 4096  data: kapacita * bajtu na snimek
//! ```
//!
//! Pozice jsou monotonni pocty snimku (u64 nepretece); index v datech je
//! `pos % kapacita`. Producent: zapise data, pak `write_pos` (Release).
//! Konzument: precte `write_pos` (Acquire), data, pak `read_pos` (Release).
//!
//! Druha strana neni duveryhodna: pozice od ni se vzdy omezi
//! ([`Ring::readable`], [`Ring::writable`]) - nesmyslna pozice znamena
//! "nic", nikdy cteni mimo buffer.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::{Direction, Error, Result};

pub const MAGIC: u32 = u32::from_le_bytes(*b"AUR1");
pub const VERSION: u32 = 1;
/// Offset dat od zacatku VMO (hlavicka ma vlastni stranku).
pub const DATA_OFFSET: usize = 4096;
/// Nejvetsi kapacita v bajtech (1 s pri 192 kHz, 8 kanalech, 32 bitech je
/// 6 MiB; tolik nikdo nepotrebuje - 4 MiB staci na ~20 s CD kvality).
pub const MAX_DATA: usize = 4 << 20;

const OFF_MAGIC: usize = 0;
const OFF_VERSION: usize = 4;
const OFF_CAPACITY: usize = 8;
const OFF_FRAME: usize = 12;
const OFF_DATA: usize = 16;
const OFF_DIR: usize = 20;
const OFF_WRITE: usize = 64;
const OFF_READ: usize = 128;
const OFF_UNDERRUNS: usize = 192;
const OFF_OVERRUNS: usize = 200;
const OFF_PLAYED: usize = 208;
const OFF_LATENCY: usize = 216;

/// Velikost VMO pro buffer s `capacity` snimky po `frame_bytes`.
pub const fn vmo_size(capacity: u32, frame_bytes: u32) -> usize {
    DATA_OFFSET + capacity as usize * frame_bytes as usize
}

/// Pohled na namapovany buffer. Neplati za pamet - mapovani drzi volajici
/// (typicky `libos::Mapping`) po celou dobu zivota `Ring`.
pub struct Ring {
    base: *mut u8,
    capacity: u32,
    frame_bytes: u32,
}

// SAFETY: vsechny pristupy jsou atomicke nebo pres surove ukazatele bez
// odkazu; pamet sdili i jine procesy, takze dalsi vlakno nic nezhorsi.
unsafe impl Send for Ring {}

impl Ring {
    /// Zalozi hlavicku v nove pameti (sluzba).
    ///
    /// # Safety
    ///
    /// `base` ukazuje na zapisovatelnou pamet o delce `len`, zarovnanou na
    /// 8 bajtu, platnou po celou dobu zivota vysledku.
    pub unsafe fn init(base: *mut u8, len: usize, capacity: u32, frame_bytes: u32, dir: Direction) -> Result<Self> {
        let need = vmo_size(capacity, frame_bytes);
        if capacity == 0 || frame_bytes == 0 || need > len || need - DATA_OFFSET > MAX_DATA {
            return Err(Error::InvalidArgs);
        }
        let r = Self { base, capacity, frame_bytes };
        r.u32(OFF_VERSION).store(VERSION, Ordering::Relaxed);
        r.u32(OFF_CAPACITY).store(capacity, Ordering::Relaxed);
        r.u32(OFF_FRAME).store(frame_bytes, Ordering::Relaxed);
        r.u32(OFF_DATA).store(DATA_OFFSET as u32, Ordering::Relaxed);
        r.u32(OFF_DIR).store(dir as u32, Ordering::Relaxed);
        for off in [OFF_WRITE, OFF_READ, OFF_UNDERRUNS, OFF_OVERRUNS, OFF_PLAYED, OFF_LATENCY] {
            r.u64(off).store(0, Ordering::Relaxed);
        }
        r.u32(OFF_MAGIC).store(MAGIC, Ordering::Release);
        Ok(r)
    }

    /// Pripoji se k existujici hlavicce (klient). Overi magic, verzi
    /// a ze se data vejdou do `len`.
    ///
    /// # Safety
    ///
    /// Jako u [`Ring::init`].
    pub unsafe fn attach(base: *mut u8, len: usize) -> Result<Self> {
        if len < DATA_OFFSET {
            return Err(Error::InvalidArgs);
        }
        let probe = Self { base, capacity: 0, frame_bytes: 0 };
        if probe.u32(OFF_MAGIC).load(Ordering::Acquire) != MAGIC || probe.u32(OFF_VERSION).load(Ordering::Relaxed) != VERSION {
            return Err(Error::Corrupted);
        }
        let capacity = probe.u32(OFF_CAPACITY).load(Ordering::Relaxed);
        let frame_bytes = probe.u32(OFF_FRAME).load(Ordering::Relaxed);
        let data = probe.u32(OFF_DATA).load(Ordering::Relaxed) as usize;
        if capacity == 0 || frame_bytes == 0 || data != DATA_OFFSET {
            return Err(Error::Corrupted);
        }
        let need = (capacity as usize).checked_mul(frame_bytes as usize).and_then(|n| n.checked_add(DATA_OFFSET));
        if need.is_none_or(|n| n > len) {
            return Err(Error::Corrupted);
        }
        Ok(Self { base, capacity, frame_bytes })
    }

    fn u32(&self, off: usize) -> &AtomicU32 {
        // SAFETY: offset lezi v hlavicce (prvni stranka), zarovnany na 4.
        unsafe { &*(self.base.add(off) as *const AtomicU32) }
    }

    fn u64(&self, off: usize) -> &AtomicU64 {
        // SAFETY: offset lezi v hlavicce, zarovnany na 8.
        unsafe { &*(self.base.add(off) as *const AtomicU64) }
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn frame_bytes(&self) -> u32 {
        self.frame_bytes
    }

    pub fn write_pos(&self) -> u64 {
        self.u64(OFF_WRITE).load(Ordering::Acquire)
    }

    pub fn read_pos(&self) -> u64 {
        self.u64(OFF_READ).load(Ordering::Acquire)
    }

    /// Snimky pripravene ke cteni. Pozice druhe strany mimo meze (pred
    /// ctenim nebo vic nez kapacita napred) = 0: buffer se nikdy necte
    /// mimo platna data.
    pub fn readable(&self) -> u32 {
        let w = self.write_pos();
        let r = self.read_pos();
        match w.checked_sub(r) {
            Some(n) if n <= self.capacity as u64 => n as u32,
            _ => 0,
        }
    }

    /// Volne misto pro zapis (snimky); nesmyslne pozice = 0.
    pub fn writable(&self) -> u32 {
        let w = self.write_pos();
        let r = self.read_pos();
        match w.checked_sub(r) {
            Some(n) if n <= self.capacity as u64 => self.capacity - n as u32,
            _ => 0,
        }
    }

    /// Jsou pozice v poradku? (`false` = druha strana zapsala nesmysl.)
    pub fn consistent(&self) -> bool {
        self.write_pos().checked_sub(self.read_pos()).is_some_and(|n| n <= self.capacity as u64)
    }

    fn data(&self) -> *mut u8 {
        // SAFETY: data lezi za hlavickou v delce overene pri init/attach.
        unsafe { self.base.add(DATA_OFFSET) }
    }

    /// Zapise cele snimky z `src` (producent); vrati pocet zapsanych snimku.
    /// Necele snimky na konci se ignoruji.
    pub fn write(&self, src: &[u8]) -> u32 {
        let fb = self.frame_bytes as usize;
        let frames = ((src.len() / fb) as u32).min(self.writable());
        let w = self.write_pos();
        self.copy_in(w, &src[..frames as usize * fb]);
        self.u64(OFF_WRITE).store(w + frames as u64, Ordering::Release);
        frames
    }

    /// Precte az `dst.len() / frame_bytes` snimku (konzument), posune
    /// `read_pos`; vrati pocet snimku.
    pub fn read(&self, dst: &mut [u8]) -> u32 {
        let n = self.peek(dst);
        self.consume(n);
        n
    }

    /// Jako [`Ring::read`], ale bez posunu `read_pos`.
    pub fn peek(&self, dst: &mut [u8]) -> u32 {
        let fb = self.frame_bytes as usize;
        let frames = ((dst.len() / fb) as u32).min(self.readable());
        self.copy_out(self.read_pos(), &mut dst[..frames as usize * fb]);
        frames
    }

    /// Posune `read_pos` o `frames` (nejvys o to, co je ke cteni).
    pub fn consume(&self, frames: u32) {
        let n = frames.min(self.readable());
        let r = self.read_pos();
        self.u64(OFF_READ).store(r + n as u64, Ordering::Release);
    }

    /// Zahodi vsechno neprectene (konzument; napr. po stop).
    pub fn discard(&self) {
        let w = self.write_pos();
        if self.consistent() {
            self.u64(OFF_READ).store(w, Ordering::Release);
        }
    }

    /// Srovna pozice po nesmyslne pozici od druhe strany (konzument):
    /// prazdny buffer na pozici producenta.
    pub fn resync(&self) {
        self.u64(OFF_READ).store(self.write_pos(), Ordering::Release);
    }

    fn copy_in(&self, pos: u64, src: &[u8]) {
        let fb = self.frame_bytes as usize;
        let cap = self.capacity as usize;
        let start = (pos % cap as u64) as usize;
        let first = src.len().min((cap - start) * fb);
        // SAFETY: start * fb + first <= cap * fb (velikost dat overena);
        // zbytek zacina od nuly a je kratsi nez buffer (frames <= cap).
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr(), self.data().add(start * fb), first);
            core::ptr::copy_nonoverlapping(src.as_ptr().add(first), self.data(), src.len() - first);
        }
    }

    fn copy_out(&self, pos: u64, dst: &mut [u8]) {
        let fb = self.frame_bytes as usize;
        let cap = self.capacity as usize;
        let start = (pos % cap as u64) as usize;
        let first = dst.len().min((cap - start) * fb);
        // SAFETY: jako copy_in. Druha strana muze data soucasne menit -
        // vysledkem jsou jen jine vzorky, ne cteni mimo buffer.
        unsafe {
            core::ptr::copy_nonoverlapping(self.data().add(start * fb), dst.as_mut_ptr(), first);
            core::ptr::copy_nonoverlapping(self.data(), dst.as_mut_ptr().add(first), dst.len() - first);
        }
    }

    // ---- Udaje od sluzby ----

    pub fn underruns(&self) -> u64 {
        self.u64(OFF_UNDERRUNS).load(Ordering::Relaxed)
    }

    pub fn add_underrun(&self) {
        self.u64(OFF_UNDERRUNS).fetch_add(1, Ordering::Relaxed);
    }

    pub fn overruns(&self) -> u64 {
        self.u64(OFF_OVERRUNS).load(Ordering::Relaxed)
    }

    pub fn add_overrun(&self) {
        self.u64(OFF_OVERRUNS).fetch_add(1, Ordering::Relaxed);
    }

    /// Snimky streamu, ktere uz zazneli (odhad podle pozice DMA).
    pub fn played(&self) -> u64 {
        self.u64(OFF_PLAYED).load(Ordering::Acquire)
    }

    pub fn set_played(&self, frames: u64) {
        self.u64(OFF_PLAYED).store(frames, Ordering::Release);
    }

    pub fn latency_us(&self) -> u64 {
        self.u64(OFF_LATENCY).load(Ordering::Relaxed)
    }

    pub fn set_latency_us(&self, us: u64) {
        self.u64(OFF_LATENCY).store(us, Ordering::Relaxed);
    }
}
