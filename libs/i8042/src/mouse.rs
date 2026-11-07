//! Pakety mysi PS/2.
//!
//! Bajt 0: tlacitka (bit 0 leve, 1 prave, 2 prostredni), bit 3 vzdy 1
//! (synchronizace), znamenko X (bit 4) a Y (bit 5), preteceni X a Y
//! (bity 6, 7). Bajty 1 a 2: pohyb X a Y (9bitova cisla se znamenkem
//! z bajtu 0; Y kladne nahoru). Ctvrty bajt podle druhu mysi:
//! - IntelliMouse (ID 3): kolecko jako i8 (kladne = k uzivateli, dolu),
//! - 5 tlacitek (ID 4): kolecko v dolnich 4 bitech (se znamenkem), bit 4
//!   tlacitko "zpet", bit 5 "vpred".

use alloc::vec::Vec;

use inputproto::code::*;
use inputproto::device::RawEvent;

/// Druh mysi podle odpovedi na `GET_ID`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// ID 0: 3 tlacitka, 3bajtove pakety.
    Standard,
    /// ID 3: kolecko, 4bajtove pakety.
    Wheel,
    /// ID 4: kolecko a 5 tlacitek, 4bajtove pakety.
    FiveButton,
}

impl Kind {
    pub fn from_id(id: u8) -> Self {
        match id {
            3 => Kind::Wheel,
            4 => Kind::FiveButton,
            _ => Kind::Standard,
        }
    }

    pub fn packet_len(self) -> usize {
        match self {
            Kind::Standard => 3,
            _ => 4,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Standard => "PS/2 mys",
            Kind::Wheel => "PS/2 mys s koleckem",
            Kind::FiveButton => "PS/2 mys s koleckem a 5 tlacitky",
        }
    }
}

/// Bity tlacitek v [`Packet::buttons`].
pub const LEFT: u8 = 1 << 0;
pub const RIGHT: u8 = 1 << 1;
pub const MIDDLE: u8 = 1 << 2;
pub const BACK: u8 = 1 << 3;
pub const FORWARD: u8 = 1 << 4;

/// Rozebrany paket (osy uz v konvenci evdev: Y kladne dolu, kolecko
/// kladne nahoru).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Packet {
    pub dx: i32,
    pub dy: i32,
    pub wheel: i32,
    pub buttons: u8,
}

/// Sklada bajty do paketu.
#[derive(Debug, Clone)]
pub struct Decoder {
    kind: Kind,
    buf: [u8; 4],
    len: usize,
    /// Tlacitka z minuleho paketu (udalost jen pri zmene).
    buttons: u8,
}

impl Decoder {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            buf: [0; 4],
            len: 0,
            buttons: 0,
        }
    }

    /// Zpracuje bajt; `Some` = hotovy paket. Prvni bajt bez synchronizacniho
    /// bitu 3 se zahodi (ztracena synchronizace se tak sama obnovi).
    pub fn feed(&mut self, b: u8) -> Option<Packet> {
        if self.len == 0 && b & 0x08 == 0 {
            return None;
        }
        self.buf[self.len] = b;
        self.len += 1;
        if self.len < self.kind.packet_len() {
            return None;
        }
        self.len = 0;
        Some(self.parse())
    }

    fn parse(&self) -> Packet {
        let [b0, b1, b2, b3] = self.buf;
        let overflow = b0 & 0xC0 != 0;
        let x = b1 as i32 - (((b0 as i32) << 4) & 0x100);
        let y = b2 as i32 - (((b0 as i32) << 3) & 0x100);
        let mut p = Packet {
            // Pri preteceni je pohyb nesmyslny: zahodit.
            dx: if overflow { 0 } else { x },
            dy: if overflow { 0 } else { -y },
            wheel: 0,
            buttons: b0 & 0x07,
        };
        match self.kind {
            Kind::Standard => {}
            Kind::Wheel => p.wheel = -(b3 as i8 as i32),
            Kind::FiveButton => {
                // Dolni 4 bity jako cislo se znamenkem.
                let z = ((b3 << 4) as i8 >> 4) as i32;
                p.wheel = -z;
                if b3 & 0x10 != 0 {
                    p.buttons |= BACK;
                }
                if b3 & 0x20 != 0 {
                    p.buttons |= FORWARD;
                }
            }
        }
        p
    }

    /// Udalosti evdev pro paket (vcetne `SYN_REPORT`); tlacitka jen pri
    /// zmene. Prazdny paket bez zmeny = zadne udalosti.
    pub fn events(&mut self, p: Packet, out: &mut Vec<RawEvent>) {
        let start = out.len();
        let changed = p.buttons ^ self.buttons;
        for (bit, code) in [
            (LEFT, BTN_LEFT),
            (RIGHT, BTN_RIGHT),
            (MIDDLE, BTN_MIDDLE),
            (BACK, BTN_SIDE),
            (FORWARD, BTN_EXTRA),
        ] {
            if changed & bit != 0 {
                out.push(RawEvent::key(code, p.buttons & bit != 0));
            }
        }
        self.buttons = p.buttons;
        if p.dx != 0 {
            out.push(RawEvent::rel(REL_X, p.dx));
        }
        if p.dy != 0 {
            out.push(RawEvent::rel(REL_Y, p.dy));
        }
        if p.wheel != 0 {
            out.push(RawEvent::rel(REL_WHEEL, p.wheel));
        }
        if out.len() > start {
            out.push(RawEvent::syn());
        }
    }
}
