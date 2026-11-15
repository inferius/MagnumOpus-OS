//! Kurzor nad obsahem okna (`SET_CURSOR`, guiproto 5).

use guiproto::cursor::{Cursor, Custom, MAX_CUSTOM, Shape};
use guiproto::window::ClientMsg;
use libos::Error;

use super::{code, with_state};

fn set(win: u32, cursor: Cursor) -> i32 {
    with_state(|s| {
        if !s.windows.contains_key(&win) {
            return code(Error::NotFound);
        }
        s.send(ClientMsg::SetCursor { window: win, cursor }).map_or_else(code, |_| 0)
    })
}

/// Systemovy tvar (`MOSDL_CURSOR_*` = `guiproto::cursor::Shape`).
#[unsafe(no_mangle)]
pub extern "C" fn mowin_set_cursor_shape(win: u32, shape: u32) -> i32 {
    match u8::try_from(shape).ok().and_then(|v| Shape::from_u8(v).ok()) {
        Some(s) => set(win, Cursor::Shape(s)),
        None => code(Error::InvalidArgs),
    }
}

/// Vlastni kurzor: `w * h` pixelu B G R A po radcich, nejvys 64 x 64.
///
/// # Safety
///
/// `pixels` ukazuje na `w * h` citelnych u32.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mowin_set_cursor_image(win: u32, pixels: *const u32, w: u32, h: u32, hot_x: u32, hot_y: u32) -> i32 {
    let max = u32::from(MAX_CUSTOM);
    if pixels.is_null() || w == 0 || h == 0 || w > max || h > max || hot_x >= w || hot_y >= h {
        return code(Error::InvalidArgs);
    }
    // SAFETY: `w * h` pixelu podle smlouvy (nejvys 64 x 64).
    let px = unsafe { core::slice::from_raw_parts(pixels, (w * h) as usize) }.to_vec();
    set(win, Cursor::Custom(Custom { w: w as u16, h: h as u16, hot_x: hot_x as u16, hot_y: hot_y as u16, pixels: px }))
}
