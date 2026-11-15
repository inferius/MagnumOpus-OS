//! Kurzor okna a zamek ukazatele (zpravy `SET_CURSOR`, `POINTER_LOCK`,
//! `WINDOW_STATE`; docs/cs/gui.md, kapitola Kurzor a zamek ukazatele).
//!
//! Aplikace si nastavi tvar kurzoru nad obsahem sveho okna: systemovy
//! tvar (kresli kompozitor v jednotnem vzhledu), skryty, nebo vlastni
//! obrazek B G R A nejvys [`MAX_CUSTOM`] x [`MAX_CUSTOM`] s aktivnim bodem.
//! Nad ramem, listou a systemovymi dialogy plati vzdy systemovy kurzor.

use alloc::vec::Vec;

use crate::{Error, Reader, Result, Writer};

/// Nejvetsi vlastni kurzor (strana v px).
pub const MAX_CUSTOM: u16 = 64;

/// Systemove tvary kurzoru (hodnoty jsou soucasti protokolu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Shape {
    Default = 0,
    /// Textovy kurzor (I).
    Text = 1,
    Wait = 2,
    Crosshair = 3,
    /// Prace na pozadi (sipka s cekanim).
    Progress = 4,
    /// Zmena velikosti: zleva nahore doprava dolu.
    ResizeNwse = 5,
    /// Zmena velikosti: zprava nahore doleva dolu.
    ResizeNesw = 6,
    ResizeEw = 7,
    ResizeNs = 8,
    Move = 9,
    NotAllowed = 10,
    /// Ruka (odkaz, tlacitko).
    Pointer = 11,
    /// Kurzor neni videt (nad obsahem okna).
    Hidden = 12,
}

impl Shape {
    pub const ALL: [Shape; 13] = [
        Shape::Default,
        Shape::Text,
        Shape::Wait,
        Shape::Crosshair,
        Shape::Progress,
        Shape::ResizeNwse,
        Shape::ResizeNesw,
        Shape::ResizeEw,
        Shape::ResizeNs,
        Shape::Move,
        Shape::NotAllowed,
        Shape::Pointer,
        Shape::Hidden,
    ];

    pub fn from_u8(v: u8) -> Result<Self> {
        Self::ALL.get(v as usize).copied().ok_or(Error::InvalidArgs)
    }
}

/// Vlastni kurzor: pixely B G R A (u32 0xAARRGGBB) po radcich bez mezer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Custom {
    pub w: u16,
    pub h: u16,
    /// Aktivni bod (uvnitr obrazku).
    pub hot_x: u16,
    pub hot_y: u16,
    pub pixels: Vec<u32>,
}

impl Custom {
    /// Kontrola neduveryhodneho vstupu: rozmery 1..=64, aktivni bod uvnitr,
    /// presne `w * h` pixelu.
    pub fn validate(&self) -> Result<()> {
        let ok = (1..=MAX_CUSTOM).contains(&self.w)
            && (1..=MAX_CUSTOM).contains(&self.h)
            && self.hot_x < self.w
            && self.hot_y < self.h
            && self.pixels.len() == self.w as usize * self.h as usize;
        if ok { Ok(()) } else { Err(Error::InvalidArgs) }
    }
}

/// Kurzor nad obsahem okna.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cursor {
    Shape(Shape),
    Custom(Custom),
}

/// Znacka vlastniho kurzoru v kodovani (za systemovymi tvary).
const CUSTOM_TAG: u8 = 0xFF;

impl Cursor {
    pub(crate) fn write(&self, w: &mut Writer) {
        match self {
            Cursor::Shape(s) => {
                w.u8(*s as u8);
            }
            Cursor::Custom(c) => {
                w.u8(CUSTOM_TAG).u16(c.w).u16(c.h).u16(c.hot_x).u16(c.hot_y);
                for p in &c.pixels {
                    w.u32(*p);
                }
            }
        }
    }

    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self> {
        let tag = r.u8()?;
        if tag != CUSTOM_TAG {
            return Shape::from_u8(tag).map(Cursor::Shape);
        }
        let (w, h, hot_x, hot_y) = (r.u16()?, r.u16()?, r.u16()?, r.u16()?);
        if !(1..=MAX_CUSTOM).contains(&w) || !(1..=MAX_CUSTOM).contains(&h) {
            return Err(Error::InvalidArgs);
        }
        let n = w as usize * h as usize;
        let mut pixels = Vec::with_capacity(n);
        for _ in 0..n {
            pixels.push(r.u32()?);
        }
        let c = Custom { w, h, hot_x, hot_y, pixels };
        c.validate()?;
        Ok(Cursor::Custom(c))
    }
}
