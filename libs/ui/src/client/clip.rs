//! Schranka v klientovi (`clipproto` v obalce `CLIPBOARD`).
//!
//! Okno, ktere kopirovalo, drzi text samo a posle ho, az o nej kompozitor
//! pozada (liny prenos). Pouziva ho [`super::run_with`] pro textova pole
//! a primo i aplikace, ktere kresli samy (vyber v terminalu).

use alloc::string::String;
use alloc::vec;

use clipproto::{ClientMsg as CMsg, ServerMsg as SMsg, MAX_INLINE, TEXT};
use guiproto::window::ClientMsg;
use libos::Result;

use super::Window;

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
        self.clip_owned = Some(t);
        self.clip(CMsg::Offer { formats: vec![String::from(TEXT)] })
    }

    /// Pozadat o vlozeni textu; odpoved prijde jako `Event::Clipboard`
    /// (zpracuje ji [`Window::clipboard_event`]).
    pub fn request_paste(&mut self) -> Result<u32> {
        self.clip_serial = self.clip_serial.wrapping_add(1).max(1);
        let serial = self.clip_serial;
        self.clip(CMsg::Request { serial, format: String::from(TEXT) })?;
        Ok(serial)
    }

    /// Zprava schranky: zadost o nas text obslouzi sama, dorucena data
    /// vrati jako text k vlozeni (jen odpoved na posledni `request_paste`).
    pub fn clipboard_event(&mut self, m: SMsg) -> Result<Option<String>> {
        match m {
            SMsg::Send { serial, format } => {
                let data = match (&self.clip_owned, format.as_str()) {
                    (Some(t), TEXT) => clamp(t).as_bytes().to_vec(),
                    _ => alloc::vec::Vec::new(),
                };
                self.clip(CMsg::Data { serial, data })?;
                Ok(None)
            }
            SMsg::Deliver { serial, format, data } if serial == self.clip_serial && format == TEXT => Ok(Some(String::from_utf8_lossy(&data).into_owned())),
            SMsg::Changed { formats } => {
                self.clip_formats = formats;
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    /// Co je prave ve schrance (polozka menu "Vlozit" aktivni / ne).
    pub fn clipboard_has_text(&self) -> bool {
        self.clip_formats.iter().any(|f| f == TEXT)
    }
}
