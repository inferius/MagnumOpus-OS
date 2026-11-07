//! Kopie poskozenych obdelniku z plochy kompozitoru do framebufferu z
//! firmwaru (UEFI GOP), pro ovladac `fbdisplay`. Bez zavislosti na jadre,
//! testuje se na hostiteli (`cargo test -p fbcopy`).
//!
//! Zdroj je vzdy B8G8R8A8 (bajty B, G, R, A; jako `u32` little-endian
//! `0xAARRGGBB`), cil 32 bitu na pixel ve formatu firmwaru:
//!
//! - [`Format::Bgr`]: bajty B, G, R, rezerva - stejne poradi, jen se
//!   vynuluje alfa (rezerva),
//! - [`Format::Rgb`]: bajty R, G, B, rezerva - prohodi se R a B.
//!
//! Kopiruje se po radcich a do cile se jen zapisuje (framebuffer je
//! mapovany write-combining: cteni z nej je velmi pomale).

#![cfg_attr(not(test), no_std)]

#[cfg(test)]
mod tests;

/// Format pixelu cile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Bajty v pameti: R, G, B, rezerva.
    Rgb,
    /// Bajty v pameti: B, G, R, rezerva.
    Bgr,
}

impl Format {
    /// Z hodnoty `abi::bootstrap::pixel` (1 = RGB, 2 = BGR).
    pub fn from_abi(v: u32) -> Option<Self> {
        match v {
            1 => Some(Self::Rgb),
            2 => Some(Self::Bgr),
            _ => None,
        }
    }

    /// Jmeno pro konfiguraci a vypisy (`rgb`, `bgr`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Rgb => "rgb",
            Self::Bgr => "bgr",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "rgb" => Some(Self::Rgb),
            "bgr" => Some(Self::Bgr),
            _ => None,
        }
    }
}

/// Nejvetsi sirka a vyska (jako `guiproto::MAX_DIM`).
pub const MAX_DIM: u32 = 16384;

/// Geometrie framebufferu: konfigurace ovladace `fbdisplay` od initu,
/// text `fb <sirka> <vyska> <krok> <rgb|bgr>` (krok v pixelech).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub width: u32,
    pub height: u32,
    /// Krok radku v pixelech (>= `width`).
    pub stride: u32,
    pub format: Format,
}

impl Geometry {
    /// Rozebere konfiguraci; slova navic nebo hodnoty mimo rozsah = `None`.
    pub fn parse(text: &str) -> Option<Self> {
        let mut p = text.split_whitespace();
        if p.next()? != "fb" {
            return None;
        }
        let mut num = || p.next()?.parse::<u32>().ok();
        let (width, height, stride) = (num()?, num()?, num()?);
        let format = Format::parse(p.next()?)?;
        let ok = p.next().is_none()
            && (1..=MAX_DIM).contains(&width)
            && (1..=MAX_DIM).contains(&height)
            && (width..=MAX_DIM * 2).contains(&stride);
        ok.then_some(Self {
            width,
            height,
            stride,
            format,
        })
    }

    /// Zapise konfiguraci (opak [`Geometry::parse`]).
    pub fn write(&self, out: &mut impl core::fmt::Write) -> core::fmt::Result {
        write!(out, "fb {} {} {} {}", self.width, self.height, self.stride, self.format.name())
    }

    /// Velikost framebufferu v bajtech.
    pub fn bytes(&self) -> u64 {
        self.stride as u64 * self.height as u64 * 4
    }
}

/// Prevod jednoho pixelu B8G8R8A8 do formatu cile (rezerva = 0).
#[inline(always)]
pub fn convert(p: u32, format: Format) -> u32 {
    match format {
        Format::Bgr => p & 0x00FF_FFFF,
        Format::Rgb => ((p >> 16) & 0xFF) | (p & 0xFF00) | ((p & 0xFF) << 16),
    }
}

/// Obdelnik v pixelech (muze presahovat plochu, [`clip`] ho orizne).
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
}

/// Orizne obdelnik na plochu `width x height`; prazdny vysledek = `None`.
pub fn clip(r: Rect, width: u32, height: u32) -> Option<Rect> {
    let x0 = (r.x as i64).max(0);
    let y0 = (r.y as i64).max(0);
    let x1 = (r.x as i64 + r.w as i64).min(width as i64);
    let y1 = (r.y as i64 + r.h as i64).min(height as i64);
    (x0 < x1 && y0 < y1).then(|| Rect::new(x0 as i32, y0 as i32, (x1 - x0) as u32, (y1 - y0) as u32))
}

/// Plocha v pameti: zacatek, rozmery v pixelech a krok radku v bajtech
/// (nasobek 4, aspon `width * 4`).
#[derive(Debug, Clone, Copy)]
pub struct Surface {
    pub base: *mut u8,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
}

impl Surface {
    /// Plocha ma smysluplnou geometrii (zarovnany zacatek a krok).
    pub fn is_valid(&self) -> bool {
        !self.base.is_null()
            && (self.base as usize).is_multiple_of(4)
            && self.stride.is_multiple_of(4)
            && self.stride as u64 >= self.width as u64 * 4
    }

    /// Velikost v bajtech (`stride * height`).
    pub fn bytes(&self) -> u64 {
        self.stride as u64 * self.height as u64
    }
}

/// Zkopiruje obdelnik `r` ze `src` (B8G8R8A8) do `dst` (format `format`)
/// na stejne souradnice. Obdelnik se orizne na prunik obou ploch; vraci
/// skutecne zkopirovany obdelnik.
///
/// # Safety
///
/// Obe plochy musi byt platne ([`Surface::is_valid`]) a namapovane celou
/// velikosti (`stride * height` bajtu), `dst` pro zapis. Zdroj smi jiny
/// proces soucasne menit (cte se bez referenci, nejhorsi vysledek je
/// roztrzeny snimek); cil nesmi nikdo jiny cist ani psat pres reference.
pub unsafe fn copy_rect(src: &Surface, dst: &Surface, r: Rect, format: Format) -> Option<Rect> {
    let r = clip(r, src.width.min(dst.width), src.height.min(dst.height))?;
    for y in r.y as u32..r.y as u32 + r.h {
        // SAFETY: obdelnik je orezany na obe plochy, takze radek
        // [x, x + w) lezi uvnitr mapovani zdroje i cile (viz kontrakt).
        unsafe {
            let s = src.base.add(y as usize * src.stride as usize + r.x as usize * 4) as *const u32;
            let d = dst.base.add(y as usize * dst.stride as usize + r.x as usize * 4) as *mut u32;
            copy_row(s, d, r.w as usize, format);
        }
    }
    Some(r)
}

/// Jeden radek po pixelech. Zvlast vetve pro oba formaty, at prekladac
/// kazdou smycku vektorizuje bez vetveni uvnitr.
///
/// # Safety
///
/// `src` a `dst` ukazuji na `n` zarovnanych pixelu (viz [`copy_rect`]).
#[inline(always)]
unsafe fn copy_row(src: *const u32, dst: *mut u32, n: usize, format: Format) {
    // SAFETY: predano volajicim.
    unsafe {
        match format {
            Format::Bgr => {
                for i in 0..n {
                    dst.add(i).write(convert(src.add(i).read(), Format::Bgr));
                }
            }
            Format::Rgb => {
                for i in 0..n {
                    dst.add(i).write(convert(src.add(i).read(), Format::Rgb));
                }
            }
        }
    }
}
