//! Protokol GUI (verze 1).
//!
//! - [`window`]: aplikace <-> kompozitor `compd`: okna, buffery, strom
//!   vrstev, strom pristupnosti, transakce a snimky, vstup v oknech.
//! - [`a11y`]: uzly stromu pristupnosti.
//! - [`display`]: kompozitor <-> ovladac grafiky (`virtio-gpu`).
//! - [`action`]: akce aplikaci deklarovane v manifestu, typovane hodnoty.
//! - [`control`]: ridici prikazy kompozitoru (od initu) a systemove dialogy.
//! - [`replay`]: zaznam platneho stavu okna pro obnovu po padu kompozitoru.
//!
//! Zprava kanalu = kod operace `u32` + pole kodovana `libs/wire`. Cteni
//! neplatne zpravy vrati `INVALID_ARGS`, nikdy paniku: kompozitor tak
//! smi cist zpravy od kohokoli.
//!
//! Knihovna je `no_std` bez zavislosti na jadre, testuje se na hostiteli
//! (`cargo test -p guiproto`). Dokumentace: `docs/cs/gui.md`,
//! `docs/en/gui.md`.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod a11y;
pub mod action;
pub mod context;
pub mod control;
pub mod display;
pub mod menu;
pub mod modes;
pub mod replay;
pub mod window;

#[cfg(test)]
mod replay_tests;
#[cfg(test)]
mod tests;

pub use abi::{Error, Result};
pub use wire::{Reader, Writer};

/// Verze protokolu okna a displeje (2: `WINDOW_SIZE`, `WINDOW_CONTEXT`;
/// 3: rozsireni kontextu pro tablet a TV, `context::ContextExt`;
/// 4: seriove cislo gesta v `InputRecord`; 5: plocha odznaku duvery
/// v kontextu okna, `ContextExt::title_badge`).
pub const VERSION: u32 = 5;

/// Nejvetsi sirka nebo vyska bufferu, okna a vystupu v pixelech.
pub const MAX_DIM: u32 = 16384;

/// Nejvetsi data jedne zpravy, ktera knihovna sestavi (rezerva pod
/// limitem kanalu 64 KiB).
pub const MAX_MESSAGE: usize = 60 * 1024;

/// Obdelnik v pixelech (`x`, `y` muzou byt zaporne - vrstva cast mimo
/// rodice).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    pub const fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// Pravy okraj (vylucne) jako i64 - bez preteceni.
    pub const fn right(&self) -> i64 {
        self.x as i64 + self.w as i64
    }

    pub const fn bottom(&self) -> i64 {
        self.y as i64 + self.h as i64
    }

    pub const fn contains(&self, x: i32, y: i32) -> bool {
        (x as i64) >= self.x as i64
            && (x as i64) < self.right()
            && (y as i64) >= self.y as i64
            && (y as i64) < self.bottom()
    }

    /// Prunik (prazdny obdelnik, kdyz se neprekryvaji).
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x0 = (self.x as i64).max(o.x as i64);
        let y0 = (self.y as i64).max(o.y as i64);
        let x1 = self.right().min(o.right());
        let y1 = self.bottom().min(o.bottom());
        if x1 <= x0 || y1 <= y0 {
            return Rect::default();
        }
        Rect::from_edges(x0, y0, x1, y1)
    }

    /// Nejmensi obdelnik obsahujici oba (prazdny se ignoruje).
    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let x0 = (self.x as i64).min(o.x as i64);
        let y0 = (self.y as i64).min(o.y as i64);
        let x1 = self.right().max(o.right());
        let y1 = self.bottom().max(o.bottom());
        Rect::from_edges(x0, y0, x1, y1)
    }

    /// Posunuty obdelnik (nasycene, bez preteceni).
    pub fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x.saturating_add(dx), self.y.saturating_add(dy), self.w, self.h)
    }

    fn from_edges(x0: i64, y0: i64, x1: i64, y1: i64) -> Rect {
        let c = |v: i64| v.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        let (x, y) = (c(x0), c(y0));
        let w = (x1 - x as i64).clamp(0, u32::MAX as i64) as u32;
        let h = (y1 - y as i64).clamp(0, u32::MAX as i64) as u32;
        Rect::new(x, y, w, h)
    }
}

/// Zapis a cteni spolecnych typu ve zpravach.
pub trait WriteExt {
    fn rect(&mut self, r: &Rect) -> &mut Self;
    fn bool(&mut self, v: bool) -> &mut Self;
    fn i64(&mut self, v: i64) -> &mut Self;
}

impl WriteExt for Writer {
    fn rect(&mut self, r: &Rect) -> &mut Self {
        self.i32(r.x).i32(r.y).u32(r.w).u32(r.h)
    }

    fn bool(&mut self, v: bool) -> &mut Self {
        self.u8(v as u8)
    }

    fn i64(&mut self, v: i64) -> &mut Self {
        self.u64(v as u64)
    }
}

pub trait ReadExt {
    fn rect(&mut self) -> Result<Rect>;
    fn bool(&mut self) -> Result<bool>;
    fn i64(&mut self) -> Result<i64>;
}

impl ReadExt for Reader<'_> {
    fn rect(&mut self) -> Result<Rect> {
        Ok(Rect::new(self.i32()?, self.i32()?, self.u32()?, self.u32()?))
    }

    fn bool(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::InvalidArgs),
        }
    }

    fn i64(&mut self) -> Result<i64> {
        Ok(self.u64()? as i64)
    }
}

/// Pocet polozek seznamu ve zprave delky `msg_len`: kazda polozka zabere
/// aspon `min_size` bajtu, vic jich zprava nest nemuze (ochrana pred obri
/// alokaci podle podvrzeneho poctu).
pub(crate) fn read_count(r: &mut Reader<'_>, min_size: usize, msg_len: usize) -> Result<usize> {
    let n = r.u32()? as usize;
    if n.saturating_mul(min_size.max(1)) > msg_len {
        return Err(Error::InvalidArgs);
    }
    Ok(n)
}
