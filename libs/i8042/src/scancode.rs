//! Scan kody sady 1 (radic s prekladem: klavesnice posila sadu 2, radic ji
//! prevadi na sadu 1).
//!
//! - Bajt `k` = stisk klavesy `k & 0x7F`, s bitem 0x80 = uvolneni.
//! - Prefix `E0`: rozsirene klavesy (sipky, pravy Ctrl a Alt, Windows ...).
//! - `E1 1D 45` / `E1 9D C5`: Pause (stisk a uvolneni; klavesa nema
//!   samostatne uvolneni, oboji prijde pri stisku).
//! - `E0 2A` / `E0 AA` / `E0 36` / `E0 B6`: falesne Shifty, ktere klavesnice
//!   posila kolem rozsirenych klaves (Print Screen, sipky s NumLockem) -
//!   ignoruji se.
//!
//! Kody sady 1 jsou pro zakladni klavesy totozne s kody evdev (evdev je
//! z nich odvozeny): `KEY_ESC` = 1 ... `KEY_F12` = 0x58.

use inputproto::code::*;

/// Jedna klavesa z posloupnosti bajtu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    /// Kod evdev.
    pub code: u16,
    /// Surovy scan kod pro `MSC_SCAN`: `k`, `0xE000 | k`, Pause `0xE11D45`.
    pub scancode: u32,
    pub pressed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum State {
    #[default]
    Normal,
    /// Prisel `E0`.
    Extended,
    /// Prisel `E1`, ceka se na dva bajty (prvni ulozeny).
    Pause(Option<u8>),
}

/// Dekoder posloupnosti scan kodu.
#[derive(Debug, Clone, Default)]
pub struct Decoder {
    state: State,
}

/// Rozsirene kody (`E0 k`) -> evdev.
const EXTENDED: &[(u8, u16)] = &[
    (0x1C, KEY_KPENTER),
    (0x1D, KEY_RIGHTCTRL),
    (0x35, KEY_KPSLASH),
    (0x37, KEY_SYSRQ),
    (0x38, KEY_RIGHTALT),
    (0x47, KEY_HOME),
    (0x48, KEY_UP),
    (0x49, KEY_PAGEUP),
    (0x4B, KEY_LEFT),
    (0x4D, KEY_RIGHT),
    (0x4F, KEY_END),
    (0x50, KEY_DOWN),
    (0x51, KEY_PAGEDOWN),
    (0x52, KEY_INSERT),
    (0x53, KEY_DELETE),
    (0x5B, KEY_LEFTMETA),
    (0x5C, KEY_RIGHTMETA),
    (0x5D, KEY_COMPOSE),
];

/// Bajty, ktere nejsou scan kody (odpovedi zarizeni, chyby) - mimo
/// inicializaci se zahazuji.
fn is_control(b: u8) -> bool {
    matches!(b, 0x00 | 0xFF | 0xEE | 0xFA | 0xFE)
}

impl Decoder {
    pub const fn new() -> Self {
        Self {
            state: State::Normal,
        }
    }

    /// Zpracuje bajt od klavesnice; `Some` = hotova klavesa.
    pub fn feed(&mut self, b: u8) -> Option<Key> {
        match self.state {
            State::Normal => match b {
                0xE0 => {
                    self.state = State::Extended;
                    None
                }
                0xE1 => {
                    self.state = State::Pause(None);
                    None
                }
                b if is_control(b) => None,
                b => {
                    let k = b & 0x7F;
                    // Zakladni kody sady 1 = evdev; ostatni (japonske a dalsi
                    // klavesy) zatim nezname.
                    (1..=0x58).contains(&k).then_some(Key {
                        code: k as u16,
                        scancode: k as u32,
                        pressed: b & 0x80 == 0,
                    })
                }
            },
            State::Extended => {
                self.state = State::Normal;
                let k = b & 0x7F;
                if matches!(k, 0x2A | 0x36) {
                    return None;
                }
                EXTENDED.iter().find(|(s, _)| *s == k).map(|&(_, code)| Key {
                    code,
                    scancode: 0xE000 | k as u32,
                    pressed: b & 0x80 == 0,
                })
            }
            State::Pause(None) => {
                self.state = State::Pause(Some(b));
                None
            }
            State::Pause(Some(first)) => {
                self.state = State::Normal;
                (first & 0x7F == 0x1D && b & 0x7F == 0x45).then_some(Key {
                    code: KEY_PAUSE,
                    scancode: 0xE11D45,
                    pressed: first & 0x80 == 0,
                })
            }
        }
    }
}
