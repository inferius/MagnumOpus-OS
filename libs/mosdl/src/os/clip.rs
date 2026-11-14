//! Schranka pro SDL (`mosdl_clipboard_*`): text pres `clipproto` v okne.
//!
//! Kopirovani plati jen v reakci na fyzicky stisk v okne s fokusem (cislo
//! gesta posledni udalosti, pravidlo kompozitoru). Vlozeni je liny prenos:
//! zadost a cekani na data (nejdel `PASTE_TIMEOUT`); udalosti, ktere mezitim
//! prijdou, se ulozi do fronty a SDL je dostane dalsim `mosdl_next_event`.

use std::ffi::c_char;

use clipproto::TEXT;
use libos::Error;

use super::{Item, code, next_item, with_state};

/// Jak dlouho cekat na data schranky od jine aplikace.
const PASTE_TIMEOUT_NS: u64 = 1_000_000_000;

/// # Safety
///
/// `text` je retezec C nebo NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mosdl_clipboard_set_text(text: *const c_char) -> i32 {
    let t = super::c_str(text);
    with_state(|s| match s.win.as_mut() {
        Some(w) => w.copy_text(t).map_or_else(code, |_| 0),
        None => code(Error::BadState),
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn mosdl_clipboard_has_text() -> i32 {
    with_state(|s| s.win.as_ref().is_some_and(|w| w.clipboard_has_text()) as i32)
}

/// Pozada o text schranky a pocka na nej. Vrati delku v bajtech (bez
/// nuly), text si drzi pro `mosdl_clipboard_copy`; zaporne = chyba
/// (`TIMED_OUT`, schranka prazdna = 0).
#[unsafe(no_mangle)]
pub extern "C" fn mosdl_clipboard_fetch() -> i64 {
    with_state(|s| {
        s.clip_text.clear();
        let Some(w) = s.win.as_mut() else { return code(Error::BadState) as i64 };
        if !w.clipboard_has_text() {
            return 0;
        }
        let serial = match w.request_paste() {
            Ok(n) => n,
            Err(e) => return code(e) as i64,
        };
        let deadline = libos::time::now().saturating_add(PASTE_TIMEOUT_NS);
        loop {
            match next_item(s, deadline) {
                Ok(Some(Item::Pasted(p))) if p.serial == serial && p.format == TEXT => {
                    s.clip_text = p.data;
                    return s.clip_text.len() as i64;
                }
                Ok(Some(Item::Pasted(_))) => {}
                Ok(Some(Item::Ev(e))) => s.pending.push_back(e),
                Ok(None) if libos::time::now() >= deadline => return code(Error::TimedOut) as i64,
                Ok(None) => {}
                Err(e) => return code(e) as i64,
            }
        }
    })
}

/// Zkopiruje text posledniho `mosdl_clipboard_fetch` do `buf` (nejvys
/// `cap - 1` bajtu a nula). Vrati delku textu.
///
/// # Safety
///
/// `buf` ukazuje na `cap` zapisovatelnych bajtu (nebo NULL pri `cap` 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mosdl_clipboard_copy(buf: *mut u8, cap: usize) -> i64 {
    with_state(|s| {
        let t = &s.clip_text;
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
