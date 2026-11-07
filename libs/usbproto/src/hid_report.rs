//! Vyznam poli reportu HID: prevod reportu na udalosti evdev.
//!
//! | Strana | Usage | Udalost |
//! |---|---|---|
//! | 0x01 Generic Desktop | X, Y relativni | `REL_X`, `REL_Y` |
//! | 0x01 | X, Y absolutni | `ABS_X`, `ABS_Y` (rozsah z logickeho min/max) |
//! | 0x01 | Wheel (0x38) | `REL_WHEEL` |
//! | 0x07 Keyboard | 0x04-0x65, 0xE0-0xE7 | klavesy ([`crate::hid::usage_to_key`]) |
//! | 0x09 Button | 1-5 | `BTN_LEFT`, `BTN_RIGHT`, `BTN_MIDDLE`, `BTN_SIDE`, `BTN_EXTRA` |
//! | 0x0C Consumer | hlasitost, prehravani, AC Pan ... | multimedialni klavesy, `REL_HWHEEL` |
//! | 0x0D Digitizer | Tip Switch, Barrel, In Range | `BTN_TOUCH`, `BTN_STYLUS`, `BTN_TOOL_PEN` |
//!
//! Promenna pole (bitmapy, osy) davaji udalost pri zmene (relativni osy pri
//! kazde nenulove hodnote); pole indexu (stisknute klavesy) se porovnavaji
//! s minulym reportem jako v boot protokolu.

use alloc::vec;
use alloc::vec::Vec;

use inputproto::code::*;
use inputproto::device::{AxisInfo, RawEvent, caps};

use crate::report::{Field, ReportDescriptor, Usage, usage};

pub const PAGE_DESKTOP: u16 = 0x01;
pub const PAGE_KEYBOARD: u16 = 0x07;
pub const PAGE_BUTTON: u16 = 0x09;
pub const PAGE_CONSUMER: u16 = 0x0C;
pub const PAGE_DIGITIZER: u16 = 0x0D;

const DESKTOP_X: Usage = usage(PAGE_DESKTOP, 0x30);
const DESKTOP_Y: Usage = usage(PAGE_DESKTOP, 0x31);
const DESKTOP_WHEEL: Usage = usage(PAGE_DESKTOP, 0x38);
const CONSUMER_AC_PAN: Usage = usage(PAGE_CONSUMER, 0x238);

/// Multimedialni klavesy (strana Consumer).
const CONSUMER_KEYS: &[(u16, u16)] = &[
    (0x6F, KEY_BRIGHTNESSUP),
    (0x70, KEY_BRIGHTNESSDOWN),
    (0xB5, KEY_NEXTSONG),
    (0xB6, KEY_PREVIOUSSONG),
    (0xB7, KEY_STOPCD),
    (0xCD, KEY_PLAYPAUSE),
    (0xE2, KEY_MUTE),
    (0xE9, KEY_VOLUMEUP),
    (0xEA, KEY_VOLUMEDOWN),
    (0x18A, KEY_MAIL),
    (0x192, KEY_CALC),
    (0x221, KEY_SEARCH),
    (0x223, KEY_HOMEPAGE),
];

/// Co usage znamena.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Meaning {
    /// Klavesa nebo tlacitko (kod evdev).
    Key(u16),
    Rel(u16),
    Abs(u16),
}

fn meaning(u: Usage, relative: bool) -> Option<Meaning> {
    let (page, id) = ((u >> 16) as u16, u as u16);
    match (page, u) {
        (_, DESKTOP_X) => Some(if relative { Meaning::Rel(REL_X) } else { Meaning::Abs(ABS_X) }),
        (_, DESKTOP_Y) => Some(if relative { Meaning::Rel(REL_Y) } else { Meaning::Abs(ABS_Y) }),
        (_, DESKTOP_WHEEL) => Some(Meaning::Rel(REL_WHEEL)),
        (_, CONSUMER_AC_PAN) => Some(Meaning::Rel(REL_HWHEEL)),
        (PAGE_KEYBOARD, _) => u8::try_from(id)
            .ok()
            .and_then(crate::hid::usage_to_key)
            .map(Meaning::Key),
        (PAGE_BUTTON, _) => match id {
            1 => Some(Meaning::Key(BTN_LEFT)),
            2 => Some(Meaning::Key(BTN_RIGHT)),
            3 => Some(Meaning::Key(BTN_MIDDLE)),
            4 => Some(Meaning::Key(BTN_SIDE)),
            5 => Some(Meaning::Key(BTN_EXTRA)),
            // Dalsi tlacitka: BTN_MISC + n (0x100..0x10F).
            6..=15 => Some(Meaning::Key(0x100 + id - 6)),
            _ => None,
        },
        (PAGE_CONSUMER, _) => CONSUMER_KEYS
            .iter()
            .find(|(c, _)| *c == id)
            .map(|&(_, k)| Meaning::Key(k)),
        (PAGE_DIGITIZER, _) => match id {
            0x42 => Some(Meaning::Key(BTN_TOUCH)),
            0x44 => Some(Meaning::Key(BTN_STYLUS)),
            0x32 => Some(Meaning::Key(BTN_TOOL_PEN)),
            _ => None,
        },
        _ => None,
    }
}

/// Scan kod pro `MSC_SCAN`: HID usage (strana << 16 | id).
fn scan(u: Usage) -> i32 {
    u as i32
}

/// Dekoder reportu jednoho rozhrani.
#[derive(Debug, Clone)]
pub struct Decoder {
    desc: ReportDescriptor,
    /// Minule hodnoty prvku (promenna pole) / stisknute usage (pole indexu).
    prev: Vec<Vec<i32>>,
}

impl Decoder {
    pub fn new(desc: ReportDescriptor) -> Self {
        let prev = desc.fields.iter().map(|f| vec![0; f.count as usize]).collect();
        Self { desc, prev }
    }

    /// Schopnosti zdroje pro sluzbu vstupu (`IDEV`).
    pub fn caps(&self) -> u16 {
        let mut c = 0;
        for f in &self.desc.fields {
            for u in self.field_usages(f) {
                match meaning(u, f.is_relative()) {
                    Some(Meaning::Key(k)) if !is_button(k) => c |= caps::KEYBOARD,
                    Some(Meaning::Rel(REL_X | REL_Y)) => c |= caps::POINTER,
                    Some(Meaning::Abs(_)) => c |= caps::ABSOLUTE,
                    _ => {}
                }
            }
        }
        c
    }

    /// Rozsahy absolutnich os (pro `IAXS`).
    pub fn axes(&self) -> Vec<AxisInfo> {
        let mut v: Vec<AxisInfo> = Vec::new();
        for f in self.desc.fields.iter().filter(|f| f.is_variable() && !f.is_relative()) {
            for i in 0..f.count as usize {
                if let Some(Meaning::Abs(code)) = f.usage_of(i).and_then(|u| meaning(u, false))
                    && !v.iter().any(|a| a.code == code)
                {
                    v.push(AxisInfo {
                        code,
                        min: f.logical_min,
                        max: f.logical_max,
                    });
                }
            }
        }
        v
    }

    /// Nejdelsi vstupni report (pro velikost prenosu).
    pub fn max_report_len(&self) -> usize {
        self.desc.max_input_len()
    }

    /// Usage, ktera pole muze hlasit (pro schopnosti).
    fn field_usages(&self, f: &Field) -> Vec<Usage> {
        if let Some((a, b)) = f.usage_range {
            (a..=b.min(a.saturating_add(1024))).collect()
        } else {
            f.usages.clone()
        }
    }

    /// Zpracuje report (vcetne bajtu id, kdyz ho deskriptor pouziva).
    /// Udalosti jednoho reportu jsou jeden snimek (konci `SYN_REPORT`).
    pub fn report(&mut self, r: &[u8], out: &mut Vec<RawEvent>) {
        let (id, payload) = if self.desc.uses_ids {
            match r.split_first() {
                Some((&id, rest)) => (id, rest),
                None => return,
            }
        } else {
            (0, r)
        };
        let start = out.len();
        for (n, f) in self.desc.fields.iter().enumerate() {
            if f.report_id != id {
                continue;
            }
            if f.is_variable() {
                variable(f, payload, &mut self.prev[n], out);
            } else {
                array(f, payload, &mut self.prev[n], out);
            }
        }
        if out.len() > start {
            out.push(RawEvent::syn());
        }
    }

    /// Zarizeni zmizelo: uvolnit drzene klavesy a tlacitka.
    pub fn release_all(&mut self, out: &mut Vec<RawEvent>) {
        let start = out.len();
        for (n, f) in self.desc.fields.iter().enumerate() {
            for (i, v) in self.prev[n].iter_mut().enumerate() {
                let u = if f.is_variable() {
                    f.usage_of(i)
                } else {
                    f.usage_for_index(*v)
                };
                if *v != 0
                    && let Some(Meaning::Key(k)) = u.and_then(|u| meaning(u, false))
                {
                    out.push(RawEvent::key(k, false));
                }
                *v = 0;
            }
        }
        if out.len() > start {
            out.push(RawEvent::syn());
        }
    }
}

/// Promenna pole: kazdy prvek ma vlastni usage.
fn variable(f: &Field, payload: &[u8], prev: &mut [i32], out: &mut Vec<RawEvent>) {
    for i in 0..f.count as usize {
        let Some(v) = f.value(payload, i) else { return };
        let Some(u) = f.usage_of(i) else { continue };
        match meaning(u, f.is_relative()) {
            Some(Meaning::Key(k)) => {
                let pressed = v != 0;
                if pressed != (prev[i] != 0) {
                    out.push(RawEvent::new(EV_MSC, MSC_SCAN, scan(u)));
                    out.push(RawEvent::key(k, pressed));
                }
            }
            Some(Meaning::Rel(axis)) if v != 0 => out.push(RawEvent::rel(axis, v)),
            Some(Meaning::Abs(axis)) if v != prev[i] => out.push(RawEvent::new(EV_ABS, axis, v)),
            _ => {}
        }
        prev[i] = v;
    }
}

/// Pole indexu: prvky jsou usage stisknutych klaves; porovnat s minulym.
fn array(f: &Field, payload: &[u8], prev: &mut [i32], out: &mut Vec<RawEvent>) {
    let mut now = Vec::with_capacity(f.count as usize);
    for i in 0..f.count as usize {
        match f.value(payload, i) {
            Some(v) => now.push(v),
            None => return,
        }
    }
    // Rollover (klavesnice: usage 1-3 = chyba): report ignorovat.
    let rollover = now.iter().any(|&v| {
        f.usage_for_index(v)
            .is_some_and(|u| u >> 16 == PAGE_KEYBOARD as u32 && (1..=3).contains(&(u & 0xFFFF)))
    });
    if rollover {
        return;
    }
    let key = |v: i32| -> Option<(Usage, u16)> {
        let u = f.usage_for_index(v)?;
        match meaning(u, false)? {
            Meaning::Key(k) => Some((u, k)),
            _ => None,
        }
    };
    for &v in prev.iter().filter(|v| !now.contains(v)) {
        if let Some((u, k)) = key(v) {
            out.push(RawEvent::new(EV_MSC, MSC_SCAN, scan(u)));
            out.push(RawEvent::key(k, false));
        }
    }
    for &v in now.iter().filter(|v| !prev.contains(v)) {
        if let Some((u, k)) = key(v) {
            out.push(RawEvent::new(EV_MSC, MSC_SCAN, scan(u)));
            out.push(RawEvent::key(k, true));
        }
    }
    prev.copy_from_slice(&now);
}
