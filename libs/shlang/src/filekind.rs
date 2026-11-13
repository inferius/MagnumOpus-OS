//! Druh dat podle obsahu (hlavicky): obrazek s rozmery, zvuk, dokument,
//! archiv, program, text. Pro souhrn souboru v terminalu (`open`) i pro
//! hodnotu "soubor" (cte se jen zacatek).

use alloc::string::String;

fn u16le(b: &[u8], i: usize) -> Option<u32> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?) as u32)
}

fn u32le(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

fn u32be(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// Druh dat podle obsahu (hlavicky), lidsky: "obrázek BMP 1920×1080".
pub fn kind(b: &[u8], whole: bool) -> String {
    let dims = |what: &str, w: Option<u32>, h: Option<u32>| match (w, h) {
        (Some(w), Some(h)) => alloc::format!("{what} {w}×{h}"),
        _ => String::from(what),
    };
    if b.starts_with(b"BM") {
        // Vyska muze byt zaporna (radky shora dolu).
        let h = u32le(b, 22).map(|h| (h as i32).unsigned_abs());
        return dims("obrázek BMP", u32le(b, 18), h);
    }
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return dims("obrázek PNG", u32be(b, 16), u32be(b, 20));
    }
    if b.starts_with(b"GIF8") {
        return dims("obrázek GIF", u16le(b, 6), u16le(b, 8));
    }
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return "obrázek JPEG".into();
    }
    if b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WAVE") {
        return "zvuk WAV".into();
    }
    if b.starts_with(b"%PDF") {
        return "dokument PDF".into();
    }
    if b.starts_with(b"PK\x03\x04") {
        return "archiv ZIP".into();
    }
    if b.starts_with(b"\x7fELF") {
        return "program ELF".into();
    }
    match core::str::from_utf8(b) {
        Ok(s) if whole => alloc::format!("text UTF-8, {} řádků (převod: from text)", s.lines().count()),
        Ok(_) => "text UTF-8 (převod: from text)".into(),
        // Jen zacatek souboru: posledni znak muze byt useknuty.
        Err(e) if !whole && e.error_len().is_none() => "text UTF-8 (převod: from text)".into(),
        Err(_) => "binární data".into(),
    }
}

