//! Schranka v klientovi (`clipproto` v obalce `CLIPBOARD`).
//!
//! Okno, ktere kopirovalo, drzi data samo (ve vsech nabidnutych
//! formatech) a posle je, az o ne kompozitor pozada (liny prenos).
//! Pouziva ho [`super::run_with`] pro textova pole a ucinky aplikace
//! (`Effect::Copy`, `Effect::Paste`) a primo i aplikace, ktere kresli samy
//! (vyber v terminalu).

use alloc::string::String;
use alloc::vec::Vec;

use clipproto::{ClientMsg as CMsg, MAX_INLINE, ServerMsg as SMsg, TEXT};
use guiproto::window::ClientMsg;
use libos::Result;

use super::Window;

/// Vlozena data: cislo pozadavku (`request_paste*`), format, bajty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pasted {
    pub serial: u32,
    pub format: String,
    pub data: Vec<u8>,
}

/// Text oriznuty na nejvys `MAX_INLINE` bajtu na hranici znaku.
fn clamp(t: &str) -> &str {
    let mut n = t.len().min(MAX_INLINE);
    while !t.is_char_boundary(n) {
        n -= 1;
    }
    &t[..n]
}

impl Window {
    fn clip(&mut self, m: CMsg) -> Result<()> {
        self.send(ClientMsg::Clipboard { data: m.encode() })
    }

    /// Zkopirovat text: okno ho drzi, kompozitor ho dostane az na zadost.
    /// Plati jen hned po fyzickem vstupu do okna s fokusem.
    pub fn copy_text(&mut self, t: String) -> Result<()> {
        let data = clamp(&t).as_bytes().to_vec();
        self.copy(alloc::vec![(String::from(TEXT), data)])
    }

    /// Zkopirovat data ve vic formatech (napr. `text/uri-list` a text);
    /// kazdy nejvys `clipproto::MAX_INLINE` bajtu (vic se oriznou).
    pub fn copy(&mut self, mut formats: Vec<(String, Vec<u8>)>) -> Result<()> {
        formats.truncate(clipproto::MAX_FORMATS);
        for (_, d) in formats.iter_mut() {
            d.truncate(MAX_INLINE);
        }
        let names = formats.iter().map(|(f, _)| f.clone()).collect();
        self.clip_owned = formats;
        self.clip(CMsg::Offer { formats: names })
    }

    /// Pozadat o vlozeni textu; odpoved prijde jako `Event::Clipboard`
    /// (zpracuje ji [`Window::clipboard_event`]).
    pub fn request_paste(&mut self) -> Result<u32> {
        self.request_paste_formats(&[String::from(TEXT)])
    }

    /// Pozadat o vlozeni prvniho z `formats`, ktery schranka ma (podle
    /// posledni zpravy `Changed`); jinak prvniho z nich.
    pub fn request_paste_formats(&mut self, formats: &[String]) -> Result<u32> {
        let format = formats.iter().find(|f| self.clip_formats.contains(f)).or(formats.first()).cloned().unwrap_or_else(|| String::from(TEXT));
        self.clip_serial = self.clip_serial.wrapping_add(1).max(1);
        let serial = self.clip_serial;
        self.clip(CMsg::Request { serial, format })?;
        Ok(serial)
    }

    /// Zprava schranky: zadost o nase data obslouzi sama, `Changed` si
    /// zapamatuje, dorucena data vrati.
    pub fn clipboard_event_any(&mut self, m: SMsg) -> Result<Option<Pasted>> {
        match m {
            SMsg::Send { serial, format } => {
                let data = self.clip_owned.iter().find(|(f, _)| *f == format).map(|(_, d)| d.clone()).unwrap_or_default();
                self.clip(CMsg::Data { serial, data })?;
                Ok(None)
            }
            SMsg::Deliver { serial, format, data } => Ok(Some(Pasted { serial, format, data })),
            SMsg::Changed { formats } => {
                self.clip_formats = formats;
                Ok(None)
            }
            SMsg::Denied { .. } => Ok(None),
        }
    }

    /// Jako [`Window::clipboard_event_any`], vrati jen text k vlozeni
    /// (odpoved na posledni `request_paste`).
    pub fn clipboard_event(&mut self, m: SMsg) -> Result<Option<String>> {
        Ok(self.clipboard_event_any(m)?.filter(|p| p.serial == self.clip_serial && p.format == TEXT).map(|p| String::from_utf8_lossy(&p.data).into_owned()))
    }

    /// Co je prave ve schrance (polozka menu "Vlozit" aktivni / ne).
    pub fn clipboard_has_text(&self) -> bool {
        self.clip_formats.iter().any(|f| f == TEXT)
    }

    /// Formaty ve schrance (posledni `Changed`).
    pub fn clipboard_formats(&self) -> &[String] {
        &self.clip_formats
    }
}
