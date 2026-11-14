//! Preklad udalosti okna (`guiproto::window::InputEvent`) na `mosdl_event`
//! pro C a kopie snimku do bufferu okna. Ciste funkce (testy na hostiteli).

use guiproto::window::{InputEvent, InputRecord};

/// Typy udalosti (`MOSDL_EV_*` v `include/mosdl.h`).
pub mod kind {
    pub const NONE: u32 = 0;
    pub const KEY: u32 = 1;
    pub const TEXT: u32 = 2;
    pub const MOTION: u32 = 3;
    pub const LEAVE: u32 = 4;
    pub const BUTTON: u32 = 5;
    pub const WHEEL: u32 = 6;
    pub const FOCUS: u32 = 7;
    pub const RESIZE: u32 = 8;
    pub const CLOSE: u32 = 9;
    pub const EXPOSE: u32 = 10;
}

/// `mosdl_event` (rozlozeni musi sedet s `include/mosdl.h`, 40 B).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MosdlEvent {
    pub kind: u32,
    pub mods: u32,
    pub time_ns: u64,
    pub a: i32,
    pub b: i32,
    pub c: i32,
    pub d: i32,
    pub synthetic: u32,
    pub reserved: u32,
}

impl MosdlEvent {
    pub fn simple(kind: u32, a: i32, b: i32) -> Self {
        Self { kind, a, b, ..Default::default() }
    }
}

/// Zaznam vstupu -> udalost pro C.
pub fn from_input(r: &InputRecord) -> MosdlEvent {
    let (kind, a, b, c, d) = match r.event {
        InputEvent::Key { code, action, ch } => (kind::KEY, code as i32, action as i32, ch.map_or(0, |c| c as i32), 0),
        InputEvent::Text(ch) => (kind::TEXT, ch as i32, 0, 0, 0),
        InputEvent::PointerMove { x, y } => (kind::MOTION, x, y, 0, 0),
        InputEvent::PointerLeave => (kind::LEAVE, 0, 0, 0, 0),
        InputEvent::Button { code, pressed, x, y } => (kind::BUTTON, code as i32, pressed as i32, x, y),
        InputEvent::Scroll { dx, dy } => (kind::WHEEL, dx, dy, 0, 0),
        InputEvent::Focus(f) => (kind::FOCUS, f as i32, 0, 0, 0),
    };
    MosdlEvent {
        kind,
        mods: r.mods as u32,
        time_ns: r.time,
        a,
        b,
        c,
        d,
        synthetic: r.synthetic as u32,
        reserved: 0,
    }
}

/// Zkopiruje snimek `src` (`sw` x `sh`, krok `pitch` bajtu, pixely po 4 B)
/// do bufferu okna `dst` (`dw` x `dh`, krok `dw` pixelu). Co snimek
/// nepokryje, je cerne (0xFF000000). Kratky `src` (krok mensi nez radek,
/// nebo malo dat) = chyba, nic se nezapise.
pub fn copy_frame(dst: &mut [u32], dw: u32, dh: u32, src: &[u8], pitch: u32, sw: u32, sh: u32) -> bool {
    let (dw, dh, sw, sh, pitch) = (dw as usize, dh as usize, sw as usize, sh as usize, pitch as usize);
    let Some(row) = sw.checked_mul(4) else { return false };
    if dst.len() < dw * dh || pitch < row {
        return false;
    }
    if sh > 0 && pitch.checked_mul(sh - 1).and_then(|v| v.checked_add(row)).is_none_or(|need| src.len() < need) {
        return false;
    }
    let w = dw.min(sw);
    let h = dh.min(sh);
    for y in 0..dh {
        let out = &mut dst[y * dw..(y + 1) * dw];
        if y < h {
            let line = &src[y * pitch..y * pitch + w * 4];
            for (o, px) in out[..w].iter_mut().zip(line.chunks_exact(4)) {
                // B G R A v pameti = u32 0xAARRGGBB na little endian;
                // alfu okna nechat nepruhlednou (XRGB).
                *o = u32::from_le_bytes([px[0], px[1], px[2], 0xFF]);
            }
            out[w..].fill(0xFF00_0000);
        } else {
            out.fill(0xFF00_0000);
        }
    }
    true
}
