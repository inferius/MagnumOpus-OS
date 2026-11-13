//! Nahled vybraneho souboru: text (prvnich [`TEXT_BYTES`] bajtu UTF-8),
//! obrazek BMP (hlavicka; pixely pro uzel obrazku, az ho `libs/ui` bude
//! mit) a druh podle pripony. Obsah souboru je nedoveryhodny vstup:
//! zadna panika, omezene rozmery.

use alloc::string::String;
use alloc::vec::Vec;

/// Kolik bajtu se cte pro nahled textu.
pub const TEXT_BYTES: usize = 64 * 1024;
/// Nejvic radku textu v nahledu.
pub const TEXT_LINES: usize = 200;
/// Nejvetsi obrazek BMP k dekodovani (pixelu).
pub const MAX_PIXELS: u64 = 4096 * 4096;

fn ext(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => name[i + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// Druh souboru podle pripony (pro sloupec Druh).
pub fn type_name(name: &str) -> &'static str {
    match ext(name).as_str() {
        "txt" | "md" | "log" | "csv" => "Text",
        "toml" | "json" | "ini" | "conf" | "cfg" => "Nastavení",
        "rs" | "c" | "h" | "cpp" | "py" | "sh" | "js" => "Zdrojový kód",
        "bmp" => "Obrázek BMP",
        "png" => "Obrázek PNG",
        "jpg" | "jpeg" => "Obrázek JPEG",
        "wav" | "mp3" | "ogg" | "flac" => "Zvuk",
        "mp4" | "mkv" | "webm" => "Video",
        "zip" | "tar" | "gz" | "zst" => "Archiv",
        "pdf" => "Dokument PDF",
        "elf" | "" => "Soubor",
        _ => "Soubor",
    }
}

/// Ikona podle pripony.
pub fn icon(name: &str) -> &'static str {
    match type_name(name) {
        "Text" | "Nastavení" | "Dokument PDF" => "file-text",
        "Zdrojový kód" => "code",
        "Obrázek BMP" | "Obrázek PNG" | "Obrázek JPEG" => "image",
        "Zvuk" => "file-audio",
        "Video" => "file-video",
        "Archiv" => "archive",
        _ => "file",
    }
}

/// Hlavicka obrazku BMP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BmpInfo {
    pub width: u32,
    pub height: u32,
    pub bpp: u16,
    /// Radky shora dolu (zaporna vyska v souboru).
    pub top_down: bool,
    /// Posun pixelu od zacatku souboru.
    pub offset: u32,
    /// Bez komprese (0) nebo bitova pole (3) - umime dekodovat.
    pub plain: bool,
}

fn le16(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn le32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// Precte hlavicku BMP (`BM` + BITMAPINFOHEADER a novejsi).
pub fn bmp_info(b: &[u8]) -> Option<BmpInfo> {
    if b.get(..2)? != b"BM" {
        return None;
    }
    let offset = le32(b, 10)?;
    let dib = le32(b, 14)?;
    if dib < 40 {
        return None;
    }
    let w = le32(b, 18)? as i32;
    let h = le32(b, 22)? as i32;
    let planes = le16(b, 26)?;
    let bpp = le16(b, 28)?;
    let comp = le32(b, 30)?;
    if w <= 0 || h == 0 || h == i32::MIN || planes != 1 {
        return None;
    }
    Some(BmpInfo { width: w as u32, height: h.unsigned_abs(), bpp, top_down: h < 0, offset, plain: comp == 0 || (comp == 3 && bpp == 32) })
}

/// Pixely BMP (24 a 32 bitu bez komprese) jako 0xAARRGGBB po radcich
/// shora; `None` pro jiny format, prilis velky obrazek nebo kratka data.
pub fn bmp_pixels(b: &[u8]) -> Option<(u32, u32, Vec<u32>)> {
    let i = bmp_info(b)?;
    if !i.plain || !(i.bpp == 24 || i.bpp == 32) || i.width as u64 * i.height as u64 > MAX_PIXELS {
        return None;
    }
    let bytes = (i.bpp / 8) as usize;
    let stride = (i.width as usize * bytes).div_ceil(4) * 4;
    let need = i.offset as usize + stride * i.height as usize;
    if b.len() < need {
        return None;
    }
    let mut out = Vec::with_capacity(i.width as usize * i.height as usize);
    for y in 0..i.height as usize {
        let row = if i.top_down { y } else { i.height as usize - 1 - y };
        let base = i.offset as usize + row * stride;
        for x in 0..i.width as usize {
            let p = &b[base + x * bytes..base + x * bytes + 3];
            out.push(0xff00_0000 | (p[2] as u32) << 16 | (p[1] as u32) << 8 | p[0] as u32);
        }
    }
    Some((i.width, i.height, out))
}

/// Text pro nahled: platne UTF-8 bez nulovych bajtu (konec muze byt
/// useknuty uprostred znaku). Radky nejvys [`TEXT_LINES`]; CRLF se cte
/// tolerantne (bez `\r`).
pub fn text_lines(b: &[u8]) -> Option<(Vec<String>, bool)> {
    if b.contains(&0) {
        return None;
    }
    let s = match core::str::from_utf8(b) {
        Ok(s) => s,
        // Useknuty posledni znak (nejvys 3 bajty) nevadi.
        Err(e) if e.error_len().is_none() && b.len() - e.valid_up_to() < 4 => core::str::from_utf8(&b[..e.valid_up_to()]).ok()?,
        Err(_) => return None,
    };
    let mut lines: Vec<String> = s.split('\n').take(TEXT_LINES + 1).map(|l| String::from(l.strip_suffix('\r').unwrap_or(l))).collect();
    let more = lines.len() > TEXT_LINES || b.len() >= TEXT_BYTES;
    lines.truncate(TEXT_LINES);
    Some((lines, more))
}

/// Co nahled ukaze.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Text { lines: Vec<String>, more: bool },
    Bmp(BmpInfo),
    /// Jen metadata (binarni nebo neznamy obsah).
    Binary,
    /// Obsah nejde precist (chyba).
    Unreadable(abi::Error),
}

/// Nahled ze zacatku souboru.
pub fn content(head: &[u8]) -> Content {
    if let Some(i) = bmp_info(head) {
        return Content::Bmp(i);
    }
    match text_lines(head) {
        Some((lines, more)) => Content::Text { lines, more },
        None => Content::Binary,
    }
}
