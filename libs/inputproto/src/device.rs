//! Zpravy zdroje vstupu -> `inputd` (ovladac zarizeni nebo umely vstup).
//!
//! Kazda zprava zacina znackou (4 B):
//!
//! | Znacka | Obsah |
//! |---|---|
//! | `IDEV` | predstaveni: verze u16, schopnosti u16 ([`caps`]), jmeno v UTF-8 |
//! | `IEVT` | surove udalosti, kazda 8 B: typ u16, kod u16, hodnota i32 |
//! | `IAXS` | rozsahy absolutnich os, kazdy 12 B: kod u16, rezerva u16, min i32, max i32 |
//!
//! Vse little-endian. Udalosti jsou ve tvaru evdev (stejne jako
//! `virtio_input_event`), skupinu zakoncuje `EV_SYN`/`SYN_REPORT`. Jedna
//! zprava muze nest vic snimku i jen cast snimku; `inputd` sklada snimky
//! napric zpravami.

use alloc::vec::Vec;

/// Verze protokolu zarizeni.
pub const VERSION: u16 = 1;

pub const TAG_HELLO: &[u8; 4] = b"IDEV";
pub const TAG_EVENTS: &[u8; 4] = b"IEVT";
pub const TAG_AXES: &[u8; 4] = b"IAXS";

/// Nejvic absolutnich os v `IAXS`.
pub const MAX_AXES: usize = 16;

/// Rozsah absolutni osy (`ABS_X`, `ABS_Y`): hodnoty `EV_ABS` jsou v
/// `min..=max`, sluzba je prevede na 0..=65535.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxisInfo {
    pub code: u16,
    pub min: i32,
    pub max: i32,
}

/// Zprava `IAXS` (posila zdroj s `caps::ABSOLUTE` po predstaveni).
pub fn encode_axes(axes: &[AxisInfo]) -> Vec<u8> {
    let axes = &axes[..axes.len().min(MAX_AXES)];
    let mut v = Vec::with_capacity(4 + axes.len() * 12);
    v.extend_from_slice(TAG_AXES);
    for a in axes {
        v.extend_from_slice(&a.code.to_le_bytes());
        v.extend_from_slice(&[0, 0]);
        v.extend_from_slice(&a.min.to_le_bytes());
        v.extend_from_slice(&a.max.to_le_bytes());
    }
    v
}

/// Nejvic udalosti v jedne zprave (`IEVT`).
pub const MAX_EVENTS: usize = 1024;
/// Nejdelsi jmeno zarizeni v bajtech.
pub const MAX_NAME: usize = 128;

/// Schopnosti zarizeni (bity v `IDEV`).
pub mod caps {
    /// Ma klavesy (klavesnice).
    pub const KEYBOARD: u16 = 1 << 0;
    /// Ma relativni osy a tlacitka (mys).
    pub const POINTER: u16 = 1 << 1;
    /// Ma absolutni osy (tablet, dotykova plocha); rozsahy posle `IAXS`.
    pub const ABSOLUTE: u16 = 1 << 2;
}

/// Jedna surova udalost (evdev).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawEvent {
    pub kind: u16,
    pub code: u16,
    pub value: i32,
}

impl RawEvent {
    pub const SIZE: usize = 8;

    pub const fn new(kind: u16, code: u16, value: i32) -> Self {
        Self { kind, code, value }
    }

    /// Konec snimku.
    pub const fn syn() -> Self {
        Self::new(crate::code::EV_SYN, crate::code::SYN_REPORT, 0)
    }

    /// Klavesa nebo tlacitko: `true` stisk, `false` uvolneni.
    pub const fn key(code: u16, pressed: bool) -> Self {
        Self::new(crate::code::EV_KEY, code, pressed as i32)
    }

    pub const fn rel(axis: u16, value: i32) -> Self {
        Self::new(crate::code::EV_REL, axis, value)
    }

    pub fn to_bytes(self) -> [u8; 8] {
        let mut b = [0u8; 8];
        b[0..2].copy_from_slice(&self.kind.to_le_bytes());
        b[2..4].copy_from_slice(&self.code.to_le_bytes());
        b[4..8].copy_from_slice(&self.value.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        let b: &[u8; 8] = b.get(..8)?.try_into().ok()?;
        Some(Self {
            kind: u16::from_le_bytes([b[0], b[1]]),
            code: u16::from_le_bytes([b[2], b[3]]),
            value: i32::from_le_bytes([b[4], b[5], b[6], b[7]]),
        })
    }
}

/// Rozebrana zprava zdroje vstupu.
#[derive(Debug, PartialEq, Eq)]
pub enum DeviceMessage<'a> {
    Hello {
        version: u16,
        caps: u16,
        name: &'a str,
    },
    Events(Vec<RawEvent>),
    Axes(Vec<AxisInfo>),
}

/// Zprava `IDEV`. Jmeno delsi nez [`MAX_NAME`] se zkrati (na hranici znaku).
pub fn encode_hello(caps: u16, name: &str) -> Vec<u8> {
    let mut end = name.len().min(MAX_NAME);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    let mut v = Vec::with_capacity(8 + end);
    v.extend_from_slice(TAG_HELLO);
    v.extend_from_slice(&VERSION.to_le_bytes());
    v.extend_from_slice(&caps.to_le_bytes());
    v.extend_from_slice(&name.as_bytes()[..end]);
    v
}

/// Zprava `IEVT` (nejvys [`MAX_EVENTS`] udalosti, zbytek je treba poslat
/// dalsi zpravou).
pub fn encode_events(events: &[RawEvent]) -> Vec<u8> {
    let events = &events[..events.len().min(MAX_EVENTS)];
    let mut v = Vec::with_capacity(4 + events.len() * RawEvent::SIZE);
    v.extend_from_slice(TAG_EVENTS);
    for e in events {
        v.extend_from_slice(&e.to_bytes());
    }
    v
}

/// Rozebere zpravu; `None` = neplatna (neznama znacka, spatna delka, jmeno
/// neni UTF-8, prilis mnoho udalosti).
pub fn decode(data: &[u8]) -> Option<DeviceMessage<'_>> {
    let (tag, rest) = data.split_at_checked(4)?;
    if tag == TAG_HELLO {
        let version = u16::from_le_bytes(rest.get(0..2)?.try_into().ok()?);
        let caps = u16::from_le_bytes(rest.get(2..4)?.try_into().ok()?);
        let name = rest.get(4..)?;
        if name.len() > MAX_NAME {
            return None;
        }
        let name = core::str::from_utf8(name).ok()?;
        Some(DeviceMessage::Hello {
            version,
            caps,
            name,
        })
    } else if tag == TAG_AXES {
        if rest.len() % 12 != 0 || rest.len() / 12 > MAX_AXES {
            return None;
        }
        let axes = rest
            .chunks_exact(12)
            .map(|c| AxisInfo {
                code: u16::from_le_bytes([c[0], c[1]]),
                min: i32::from_le_bytes(c[4..8].try_into().unwrap()),
                max: i32::from_le_bytes(c[8..12].try_into().unwrap()),
            })
            .collect();
        Some(DeviceMessage::Axes(axes))
    } else if tag == TAG_EVENTS {
        if rest.len() % RawEvent::SIZE != 0 || rest.len() / RawEvent::SIZE > MAX_EVENTS {
            return None;
        }
        let events = rest
            .chunks_exact(RawEvent::SIZE)
            .filter_map(RawEvent::from_bytes)
            .collect();
        Some(DeviceMessage::Events(events))
    } else {
        None
    }
}
