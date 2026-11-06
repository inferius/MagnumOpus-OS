//! Formatovani jednoho radku logu do bufferu na zasobniku (bez alokace).

use core::fmt;

const LINE_MAX: usize = 512;
const TRUNCATED: &str = "...";

pub struct LineBuf {
    buf: [u8; LINE_MAX],
    len: usize,
    truncated: bool,
}

impl LineBuf {
    pub fn new() -> Self {
        Self {
            buf: [0; LINE_MAX],
            len: 0,
            truncated: false,
        }
    }

    /// Ukonci radek; prilis dlouhy oznaci `...`. Misto pro konec radku
    /// a znacku je vyhrazene v `write_str`.
    pub fn finish_line(&mut self) {
        if self.truncated {
            self.push(TRUNCATED);
        }
        self.push("\n");
    }

    fn push(&mut self, s: &str) {
        let n = s.len().min(LINE_MAX - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    pub fn as_str(&self) -> &str {
        // SAFETY: do bufferu se zapisuji jen cele UTF-8 znaky (`write_str`
        // zkracuje na hranici znaku), konstanty jsou ASCII.
        unsafe { core::str::from_utf8_unchecked(self.as_bytes()) }
    }
}

impl fmt::Write for LineBuf {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        // Rezerva na "...\n".
        let room = LINE_MAX - TRUNCATED.len() - 1 - self.len;
        if s.len() <= room {
            self.push(s);
            return Ok(());
        }
        let mut cut = room;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        self.push(&s[..cut]);
        self.truncated = true;
        Ok(())
    }
}
