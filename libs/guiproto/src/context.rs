//! Rozsireni kontextu okna (`WINDOW_CONTEXT`) pro tablet a televizi
//! (design/tablet.html, design/televize.html): drzeni zarizeni, vzdalenost
//! pohledu, bezpecny okraj, ukazatele, klavesnice, prekazky, dlazdice,
//! nejmensi cil a velikost textu. Aplikace podle nich meni hustotu,
//! zkratky a rozmisteni; knihovna `ui` je pouzije sama, kde muze.

use alloc::vec::Vec;

use crate::{ReadExt, Reader, Rect, Result, WriteExt, Writer};

/// Jak se zarizeni drzi / pouziva.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Posture {
    #[default]
    Desktop,
    Laptop,
    Tablet,
    Phone,
    /// Televize: ovladani na dalku (fokus je jediny ukazatel).
    Tv,
}

/// Vzdalenost oci od obrazovky (velikost pisma a cilu).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Distance {
    /// Telefon, tablet v ruce.
    Near,
    #[default]
    Desk,
    /// Televize, prezentace.
    Room,
}

/// Dlazdice okna (ovladani k vnejsimu okraji).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tile {
    #[default]
    Floating,
    Full,
    Left,
    Right,
    Top,
    Bottom,
}

/// Druh ukazatele.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerKind {
    Mouse,
    Touchpad,
    Touch,
    Pen,
    /// Dalkovy ovladac nebo gamepad (jen fokus, bez souradnic).
    Remote,
}

/// Ukazatel dostupny oknu (dotyk a pero mohou byt soucasne).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pointer {
    pub kind: PointerKind,
    /// Presny (mys, pero) proti hrubemu (prst).
    pub precise: bool,
    /// Umi najeti (hover).
    pub hover: bool,
    pub pressure: bool,
}

/// Okraje v logickych px.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Insets {
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
    pub left: u16,
}

/// Druh prekazky nad oknem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObstacleKind {
    /// Klavesnice na obrazovce.
    Keyboard,
    Other,
}

/// Rozsireni kontextu okna.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextExt {
    pub posture: Posture,
    pub distance: Distance,
    /// Systemove listy prekryvajici okno.
    pub insets: Insets,
    /// Bezpecny okraj kvuli presahu obrazu (overscan): pozadi az k hrane,
    /// text a fokus jen uvnitr.
    pub safe: Insets,
    pub pointers: Vec<Pointer>,
    /// Pripojena hardwarova klavesnice (ukazat zkratky, nevyvolavat
    /// klavesnici na obrazovce).
    pub keyboard: bool,
    /// Prekazky v okne (souradnice obsahu okna), napr. klavesnice na
    /// obrazovce: pole s fokusem se posune nad ni.
    pub obscured: Vec<(ObstacleKind, Rect)>,
    pub tile: Tile,
    /// Nejmensi cil dotyku / kliknuti (px).
    pub min_hit: u16,
    /// Velikost textu v promile (1000 = 100 %).
    pub text_scale: u16,
}

impl Default for ContextExt {
    fn default() -> Self {
        Self {
            posture: Posture::Desktop,
            distance: Distance::Desk,
            insets: Insets::default(),
            safe: Insets::default(),
            pointers: alloc::vec![Pointer { kind: PointerKind::Mouse, precise: true, hover: true, pressure: false }],
            keyboard: true,
            obscured: Vec::new(),
            tile: Tile::Floating,
            min_hit: 32,
            text_scale: 1000,
        }
    }
}

/// Nejvic ukazatelu a prekazek ve zprave.
const MAX_ITEMS: usize = 16;

fn insets_w(w: &mut Writer, i: &Insets) {
    w.u16(i.top).u16(i.right).u16(i.bottom).u16(i.left);
}

fn insets_r(r: &mut Reader<'_>) -> Result<Insets> {
    Ok(Insets { top: r.u16()?, right: r.u16()?, bottom: r.u16()?, left: r.u16()? })
}

fn code<T: Copy + PartialEq>(all: &[T], v: T) -> u8 {
    all.iter().position(|&x| x == v).unwrap_or(0) as u8
}

fn from<T: Copy>(all: &[T], c: u8) -> Result<T> {
    all.get(c as usize).copied().ok_or(crate::Error::InvalidArgs)
}

const POSTURES: [Posture; 5] = [Posture::Desktop, Posture::Laptop, Posture::Tablet, Posture::Phone, Posture::Tv];
const DISTANCES: [Distance; 3] = [Distance::Near, Distance::Desk, Distance::Room];
const TILES: [Tile; 6] = [Tile::Floating, Tile::Full, Tile::Left, Tile::Right, Tile::Top, Tile::Bottom];
const KINDS: [PointerKind; 5] = [PointerKind::Mouse, PointerKind::Touchpad, PointerKind::Touch, PointerKind::Pen, PointerKind::Remote];
const OBSTACLES: [ObstacleKind; 2] = [ObstacleKind::Keyboard, ObstacleKind::Other];

impl ContextExt {
    pub fn encode(&self, w: &mut Writer) {
        w.u8(code(&POSTURES, self.posture)).u8(code(&DISTANCES, self.distance));
        insets_w(w, &self.insets);
        insets_w(w, &self.safe);
        let ps = &self.pointers[..self.pointers.len().min(MAX_ITEMS)];
        w.u8(ps.len() as u8);
        for p in ps {
            w.u8(code(&KINDS, p.kind)).u8(p.precise as u8 | (p.hover as u8) << 1 | (p.pressure as u8) << 2);
        }
        w.bool(self.keyboard);
        let os = &self.obscured[..self.obscured.len().min(MAX_ITEMS)];
        w.u8(os.len() as u8);
        for (k, r) in os {
            w.u8(code(&OBSTACLES, *k)).rect(r);
        }
        w.u8(code(&TILES, self.tile)).u16(self.min_hit).u16(self.text_scale);
    }

    pub fn decode(r: &mut Reader<'_>) -> Result<Self> {
        let posture = from(&POSTURES, r.u8()?)?;
        let distance = from(&DISTANCES, r.u8()?)?;
        let insets = insets_r(r)?;
        let safe = insets_r(r)?;
        let n = r.u8()? as usize;
        if n > MAX_ITEMS {
            return Err(crate::Error::InvalidArgs);
        }
        let mut pointers = Vec::with_capacity(n);
        for _ in 0..n {
            let kind = from(&KINDS, r.u8()?)?;
            let f = r.u8()?;
            pointers.push(Pointer { kind, precise: f & 1 != 0, hover: f & 2 != 0, pressure: f & 4 != 0 });
        }
        let keyboard = r.bool()?;
        let n = r.u8()? as usize;
        if n > MAX_ITEMS {
            return Err(crate::Error::InvalidArgs);
        }
        let mut obscured = Vec::with_capacity(n);
        for _ in 0..n {
            let k = from(&OBSTACLES, r.u8()?)?;
            obscured.push((k, r.rect()?));
        }
        let tile = from(&TILES, r.u8()?)?;
        Ok(Self { posture, distance, insets, safe, pointers, keyboard, obscured, tile, min_hit: r.u16()?, text_scale: r.u16()? })
    }
}
