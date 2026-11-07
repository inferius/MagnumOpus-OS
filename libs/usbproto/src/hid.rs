//! HID v boot protokolu (HID 1.11, dodatek B): klavesnice a mys bez
//! parseru deskriptoru reportu. Plny parser reportu prijde pozdeji
//! (multimedialni klavesy, tablety).

use alloc::vec::Vec;

use inputproto::code::*;
use inputproto::device::RawEvent;

use crate::desc::{Setup, rt};

/// Podtrida "boot interface".
pub const SUBCLASS_BOOT: u8 = 1;
/// Protokol rozhrani: klavesnice.
pub const PROTOCOL_KEYBOARD: u8 = 1;
/// Protokol rozhrani: mys.
pub const PROTOCOL_MOUSE: u8 = 2;

/// Pozadavky tridy HID.
pub mod request {
    pub const GET_REPORT: u8 = 0x01;
    pub const SET_REPORT: u8 = 0x09;
    pub const SET_IDLE: u8 = 0x0A;
    pub const SET_PROTOCOL: u8 = 0x0B;
}

/// `SET_PROTOCOL(boot)` pro rozhrani `iface`.
pub const fn set_boot_protocol(iface: u8) -> Setup {
    Setup {
        request_type: rt::HOST_TO_DEVICE | rt::CLASS | rt::TO_INTERFACE,
        request: request::SET_PROTOCOL,
        value: 0,
        index: iface as u16,
        length: 0,
    }
}

/// `SET_IDLE(0)`: report jen pri zmene (opakovani dela sluzba vstupu).
pub const fn set_idle_zero(iface: u8) -> Setup {
    Setup {
        request_type: rt::HOST_TO_DEVICE | rt::CLASS | rt::TO_INTERFACE,
        request: request::SET_IDLE,
        value: 0,
        index: iface as u16,
        length: 0,
    }
}

/// `SET_REPORT(output)`: kontrolky klavesnice (1 B dat).
pub const fn set_leds(iface: u8) -> Setup {
    Setup {
        request_type: rt::HOST_TO_DEVICE | rt::CLASS | rt::TO_INTERFACE,
        request: request::SET_REPORT,
        value: 0x0200,
        index: iface as u16,
        length: 1,
    }
}

/// Bity kontrolek ve vystupnim reportu klavesnice.
pub fn leds(num: bool, caps: bool, scroll: bool) -> u8 {
    (num as u8) | (caps as u8) << 1 | (scroll as u8) << 2
}

/// HID usage (strana 0x07, klavesnice) -> kod evdev. Index = usage
/// 0x04..=0x65.
const USAGES: [u16; 0x62] = [
    KEY_A, KEY_B, KEY_C, KEY_D, KEY_E, KEY_F, KEY_G, KEY_H, KEY_I, KEY_J, KEY_K, KEY_L, KEY_M,
    KEY_N, KEY_O, KEY_P, KEY_Q, KEY_R, KEY_S, KEY_T, KEY_U, KEY_V, KEY_W, KEY_X, KEY_Y, KEY_Z,
    KEY_1, KEY_2, KEY_3, KEY_4, KEY_5, KEY_6, KEY_7, KEY_8, KEY_9, KEY_0, KEY_ENTER, KEY_ESC,
    KEY_BACKSPACE, KEY_TAB, KEY_SPACE, KEY_MINUS, KEY_EQUAL, KEY_LEFTBRACE, KEY_RIGHTBRACE,
    KEY_BACKSLASH, KEY_BACKSLASH, KEY_SEMICOLON, KEY_APOSTROPHE, KEY_GRAVE, KEY_COMMA, KEY_DOT,
    KEY_SLASH, KEY_CAPSLOCK, KEY_F1, KEY_F2, KEY_F3, KEY_F4, KEY_F5, KEY_F6, KEY_F7, KEY_F8,
    KEY_F9, KEY_F10, KEY_F11, KEY_F12, KEY_SYSRQ, KEY_SCROLLLOCK, KEY_PAUSE, KEY_INSERT,
    KEY_HOME, KEY_PAGEUP, KEY_DELETE, KEY_END, KEY_PAGEDOWN, KEY_RIGHT, KEY_LEFT, KEY_DOWN,
    KEY_UP, KEY_NUMLOCK, KEY_KPSLASH, KEY_KPASTERISK, KEY_KPMINUS, KEY_KPPLUS, KEY_KPENTER,
    KEY_KP1, KEY_KP2, KEY_KP3, KEY_KP4, KEY_KP5, KEY_KP6, KEY_KP7, KEY_KP8, KEY_KP9, KEY_KP0,
    KEY_KPDOT, KEY_102ND, KEY_COMPOSE,
];

/// Modifikatory v bajtu 0 reportu klavesnice (bit 0 = usage 0xE0).
const MODIFIERS: [u16; 8] = [
    KEY_LEFTCTRL,
    KEY_LEFTSHIFT,
    KEY_LEFTALT,
    KEY_LEFTMETA,
    KEY_RIGHTCTRL,
    KEY_RIGHTSHIFT,
    KEY_RIGHTALT,
    KEY_RIGHTMETA,
];

/// Kod evdev pro usage klavesnice (`None` = nezname).
pub fn usage_to_key(usage: u8) -> Option<u16> {
    match usage {
        0x04..=0x65 => Some(USAGES[(usage - 0x04) as usize]),
        0xE0..=0xE7 => Some(MODIFIERS[(usage - 0xE0) as usize]),
        _ => None,
    }
}

/// Udalosti jedne klavesy: `MSC_SCAN` (HID usage 0x7xxxx), klavesa, konec
/// snimku.
fn key_events(usage: u8, code: u16, pressed: bool, out: &mut Vec<RawEvent>) {
    out.push(RawEvent::new(EV_MSC, MSC_SCAN, 0x70000 | usage as i32));
    out.push(RawEvent::key(code, pressed));
    out.push(RawEvent::syn());
}

/// Klavesnice v boot protokolu: report 8 B (modifikatory, rezerva, az 6
/// stisknutych klaves). Udalosti vznikaji z rozdilu proti minulemu reportu.
#[derive(Debug, Clone, Default)]
pub struct BootKeyboard {
    mods: u8,
    keys: [u8; 6],
}

impl BootKeyboard {
    pub const fn new() -> Self {
        Self {
            mods: 0,
            keys: [0; 6],
        }
    }

    /// Zpracuje report. Report "rollover" (prilis mnoho klaves, usage 1)
    /// a kratky report se ignoruji.
    pub fn report(&mut self, r: &[u8], out: &mut Vec<RawEvent>) {
        let Some(r) = r.get(..8) else {
            return;
        };
        let keys: [u8; 6] = r[2..8].try_into().unwrap_or([0; 6]);
        if keys.iter().any(|&k| (1..=3).contains(&k)) {
            return;
        }
        // Uvolneni driv nez stisky (napr. rychle prepsani A -> B).
        for (bit, &code) in MODIFIERS.iter().enumerate() {
            let m = 1 << bit;
            if self.mods & m != 0 && r[0] & m == 0 {
                key_events(0xE0 + bit as u8, code, false, out);
            }
        }
        for &k in self.keys.iter().filter(|&&k| k != 0 && !keys.contains(&k)) {
            if let Some(code) = usage_to_key(k) {
                key_events(k, code, false, out);
            }
        }
        for (bit, &code) in MODIFIERS.iter().enumerate() {
            let m = 1 << bit;
            if self.mods & m == 0 && r[0] & m != 0 {
                key_events(0xE0 + bit as u8, code, true, out);
            }
        }
        for &k in keys.iter().filter(|&&k| k != 0 && !self.keys.contains(&k)) {
            if let Some(code) = usage_to_key(k) {
                key_events(k, code, true, out);
            }
        }
        self.mods = r[0];
        self.keys = keys;
    }

    /// Zarizeni zmizelo: uvolnit vse drzene.
    pub fn release_all(&mut self, out: &mut Vec<RawEvent>) {
        self.report(&[0; 8], out);
    }
}

/// Mys v boot protokolu: tlacitka (bit 0-4), dx, dy (i8; Y kladne dolu
/// jako evdev), volitelne kolecko (i8, kladne od uzivatele jako `REL_WHEEL`).
#[derive(Debug, Clone, Default)]
pub struct BootMouse {
    buttons: u8,
}

impl BootMouse {
    pub const fn new() -> Self {
        Self { buttons: 0 }
    }

    pub fn report(&mut self, r: &[u8], out: &mut Vec<RawEvent>) {
        if r.len() < 3 {
            return;
        }
        let start = out.len();
        let b = r[0] & 0x1F;
        let changed = b ^ self.buttons;
        for (bit, code) in [BTN_LEFT, BTN_RIGHT, BTN_MIDDLE, BTN_SIDE, BTN_EXTRA]
            .into_iter()
            .enumerate()
        {
            if changed & (1 << bit) != 0 {
                out.push(RawEvent::key(code, b & (1 << bit) != 0));
            }
        }
        self.buttons = b;
        let (dx, dy) = (r[1] as i8 as i32, r[2] as i8 as i32);
        if dx != 0 {
            out.push(RawEvent::rel(REL_X, dx));
        }
        if dy != 0 {
            out.push(RawEvent::rel(REL_Y, dy));
        }
        if let Some(&w) = r.get(3)
            && w != 0
        {
            out.push(RawEvent::rel(REL_WHEEL, w as i8 as i32));
        }
        if out.len() > start {
            out.push(RawEvent::syn());
        }
    }

    pub fn release_all(&mut self, out: &mut Vec<RawEvent>) {
        self.report(&[0, 0, 0], out);
    }
}
