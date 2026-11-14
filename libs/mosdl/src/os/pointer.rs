//! Kurzor, zamek ukazatele a cela obrazovka pro SDL (guiproto 5).

use guiproto::cursor::{Cursor, Custom, MAX_CUSTOM, Shape};
use libos::Error;

use super::{State, code, with_state};

/// Po obnove kompozitoru poslat posledni kurzor znovu.
pub(super) fn restore(st: &mut State) {
    if let (Some(w), Some(c)) = (st.win.as_mut(), st.cursor.clone()) {
        let _ = w.set_cursor(c);
    }
}

fn set(c: Cursor) -> i32 {
    with_state(|s| {
        let Some(w) = s.win.as_mut() else { return code(Error::BadState) };
        s.cursor = Some(c.clone());
        w.set_cursor(c).map_or_else(code, |_| 0)
    })
}

/// Systemovy tvar (`MOSDL_CURSOR_*` = `guiproto::cursor::Shape`).
#[unsafe(no_mangle)]
pub extern "C" fn mosdl_set_cursor_shape(shape: u32) -> i32 {
    match u8::try_from(shape).ok().and_then(|v| Shape::from_u8(v).ok()) {
        Some(s) => set(Cursor::Shape(s)),
        None => code(Error::InvalidArgs),
    }
}

/// Vlastni kurzor: `w * h` pixelu B G R A (u32 0xAARRGGBB) po radcich,
/// nejvys 64 x 64.
///
/// # Safety
///
/// `pixels` ukazuje na `w * h` citelnych u32.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mosdl_set_cursor_image(pixels: *const u32, w: u32, h: u32, hot_x: u32, hot_y: u32) -> i32 {
    let max = u32::from(MAX_CUSTOM);
    if pixels.is_null() || w == 0 || h == 0 || w > max || h > max || hot_x >= w || hot_y >= h {
        return code(Error::InvalidArgs);
    }
    // SAFETY: `w * h` pixelu podle smlouvy (nejvys 64 x 64).
    let px = unsafe { core::slice::from_raw_parts(pixels, (w * h) as usize) }.to_vec();
    set(Cursor::Custom(Custom { w: w as u16, h: h as u16, hot_x: hot_x as u16, hot_y: hot_y as u16, pixels: px }))
}

#[unsafe(no_mangle)]
pub extern "C" fn mosdl_window_set_fullscreen(on: i32) -> i32 {
    with_state(|s| match s.win.as_mut() {
        Some(w) => w.set_fullscreen(on != 0).map_or_else(code, |_| 0),
        None => code(Error::BadState),
    })
}

/// Zadost o zamek ukazatele; vysledek prijde jako `MOSDL_EV_POINTER_LOCK`.
#[unsafe(no_mangle)]
pub extern "C" fn mosdl_pointer_lock(on: i32) -> i32 {
    with_state(|s| match s.win.as_mut() {
        Some(w) => w.pointer_lock(on != 0).map_or_else(code, |_| 0),
        None => code(Error::BadState),
    })
}
