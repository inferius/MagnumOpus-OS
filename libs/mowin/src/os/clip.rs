//! Schranka (`mowin_clipboard_*`): text pres `clipproto` v obalce
//! `CLIPBOARD`.
//!
//! Kopirovani i vlozeni posila cislo posledniho fyzickeho stisku
//! doruceneho oknum procesu (`State::gesture`): Win32 aplikace pracuji se
//! schrankou az po zpracovani stisku (Ctrl+C, polozka nabidky), pravidlo
//! kompozitoru (jedno gesto = jedna akce, nejvys 10 s) tak plati beze
//! zmeny. Odmitnuti zapise kompozitor do logu s identitou aplikace.
//!
//! Proces drzi kopirovany text sam a posle ho, az o nej kompozitor pozada
//! (liny prenos). Vlozeni = zadost a cekani na data (nejdel 1 s); udalosti,
//! ktere mezitim prijdou, zustanou ve fronte.

use std::ffi::c_char;

use clipproto::{ClientMsg as CMsg, MAX_INLINE, ServerMsg as SMsg, TEXT};
use guiproto::window::ClientMsg;
use libos::{Error, Result};

use super::{State, code, with_state};

const PASTE_TIMEOUT_NS: u64 = 1_000_000_000;

#[derive(Default)]
pub(super) struct Clip {
    /// Text, ktery proces nabidl (posle se na zadost).
    owned: Vec<u8>,
    /// Formaty ve schrance (posledni `Changed`).
    formats: Vec<String>,
    serial: u32,
    /// Data posledniho vlozeni (`None` = ceka se, `Some(Err)` = odmitnuto).
    got: Option<core::result::Result<Vec<u8>, Error>>,
    /// Text posledniho `mowin_clipboard_fetch`.
    text: Vec<u8>,
    /// Posledni zmena schranky je nase nabidka (udalost se neposila).
    offered: bool,
}

impl State {
    fn clip_send(&mut self, m: CMsg) -> Result<()> {
        self.send(ClientMsg::Clipboard { data: m.encode() })
    }

    pub(super) fn clip_event(&mut self, m: SMsg) -> Result<()> {
        match m {
            SMsg::Send { serial, format } => {
                let data = if format == TEXT { self.clip.owned.clone() } else { Vec::new() };
                // Jen delka, obsah schranky nikdy do logu.
                println!("mowin: schranka: data na zadost kompozitoru ({} B)", data.len());
                self.clip_send(CMsg::Data { serial, data })?;
            }
            SMsg::Deliver { serial, format, data } => {
                if serial == self.clip.serial && format == TEXT {
                    self.clip.got = Some(Ok(data));
                }
            }
            SMsg::Changed { formats } => {
                let text = formats.iter().any(|f| f == TEXT);
                self.clip.formats = formats;
                // Zmena po nasi nabidce je nase vlastni (Wine ji uz ma).
                if core::mem::take(&mut self.clip.offered) {
                    return Ok(());
                }
                self.push(crate::event::kind::CLIPBOARD, 0, [text as i32, 0, 0, 0]);
            }
            SMsg::Denied { serial, .. } => {
                if serial == self.clip.serial {
                    self.clip.got = Some(Err(Error::AccessDenied));
                }
            }
        }
        Ok(())
    }
}

/// # Safety
///
/// `text` je retezec C nebo NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mowin_clipboard_set_text(text: *const c_char) -> i32 {
    let mut t = super::c_str(text).into_bytes();
    // Na hranici znaku UTF-8.
    let mut n = t.len().min(MAX_INLINE);
    while n > 0 && n < t.len() && (t[n] & 0xC0) == 0x80 {
        n -= 1;
    }
    t.truncate(n);
    with_state(|s| {
        s.clip.owned = t;
        s.clip.offered = true;
        let gesture = s.gesture;
        println!("mowin: schranka: nabidka textu ({} B, gesto {gesture})", s.clip.owned.len());
        s.clip_send(CMsg::Offer { gesture, formats: vec![String::from(TEXT)] }).map_or_else(code, |_| 0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn mowin_clipboard_has_text() -> i32 {
    with_state(|s| s.clip.formats.iter().any(|f| f == TEXT) as i32)
}

#[unsafe(no_mangle)]
pub extern "C" fn mowin_clipboard_fetch() -> i64 {
    with_state(|s| {
        s.clip.text.clear();
        s.clip.serial = s.clip.serial.wrapping_add(1).max(1);
        s.clip.got = None;
        let (gesture, serial) = (s.gesture, s.clip.serial);
        if let Err(e) = s.clip_send(CMsg::Request { gesture, serial, format: String::from(TEXT) }) {
            return code(e) as i64;
        }
        let deadline = libos::time::now().saturating_add(PASTE_TIMEOUT_NS);
        loop {
            match s.clip.got.take() {
                Some(Ok(data)) => {
                    s.clip.text = data;
                    return s.clip.text.len() as i64;
                }
                Some(Err(e)) => return code(e) as i64,
                None => {}
            }
            match s.pump(deadline) {
                Ok(false) if libos::time::now() >= deadline => return code(Error::TimedOut) as i64,
                Ok(_) => {}
                Err(e) => return code(e) as i64,
            }
        }
    })
}

/// # Safety
///
/// `buf` ukazuje na `cap` zapisovatelnych bajtu (nebo NULL pri `cap` 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mowin_clipboard_copy(buf: *mut u8, cap: usize) -> i64 {
    with_state(|s| {
        let t = &s.clip.text;
        if cap > 0 && !buf.is_null() {
            let n = t.len().min(cap - 1);
            // SAFETY: `buf` ma `cap` bajtu podle smlouvy.
            unsafe {
                core::ptr::copy_nonoverlapping(t.as_ptr(), buf, n);
                *buf.add(n) = 0;
            }
        }
        t.len() as i64
    })
}
