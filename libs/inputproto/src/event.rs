//! Udalosti `inputd` -> vlastnik fokusu.
//!
//! Zprava kanalu je posloupnost zaznamu po [`Record::SIZE`] (32 B), jeden
//! snimek zarizeni = jedna zprava. Zaznam (little-endian):
//!
//! | Offset | Typ | Pole |
//! |---|---|---|
//! | 0 | u16 | druh udalosti ([`kind`]) |
//! | 2 | u16 | priznaky ([`flags`]) |
//! | 4 | u16 | modifikatory po udalosti ([`crate::mods`]) |
//! | 6 | u16 | rezerva (0) |
//! | 8 | u64 | cas v ns od startu (monotonni hodiny), kdy `inputd` snimek prijal |
//! | 16 | u32 x 4 | `a`, `b`, `c`, `d` podle druhu |
//!
//! Neznamy druh klient preskoci ([`Event::Unknown`]), takze nove druhy
//! udalosti nejsou nekompatibilni zmena.

use alloc::vec::Vec;

/// Druhy udalosti (pole `kind`).
pub mod kind {
    /// Fyzicka klavesa: a = kod, b = scan kod, c = akce, d = znak (0 zadny).
    pub const KEY: u16 = 1;
    /// Vlozeny text: a = znak Unicode.
    pub const TEXT: u16 = 2;
    /// Relativni pohyb ukazatele: a = dx (i32), b = dy (i32, kladne dolu).
    pub const MOTION: u16 = 3;
    /// Tlacitko mysi: a = kod tlacitka (evdev), c = akce.
    pub const BUTTON: u16 = 4;
    /// Kolecko: a = dx (i32, kladne doprava), b = dy (i32, kladne nahoru).
    pub const SCROLL: u16 = 5;
    /// Bezpecna zkratka (jen na kanal `secure`).
    pub const SECURE_ATTENTION: u16 = 6;
    /// Fokus: a = 1 ziskan, 0 ztracen.
    pub const FOCUS: u16 = 7;
    /// Pocet aktivnich zdroju umeleho vstupu: a = pocet.
    pub const INJECTION: u16 = 8;
    /// Zmena rozlozeni: a = id rozlozeni (viz `keymap::LayoutId`).
    pub const LAYOUT: u16 = 9;
    /// Prijemci neprosly zpravy (plna fronta): a = pocet zprav.
    pub const DROPPED: u16 = 10;
}

/// Priznaky zaznamu.
pub mod flags {
    /// Udalost neprisla z fyzickeho zarizeni, ale z umeleho vstupu
    /// (automatizace, vzdalena plocha, agent).
    pub const SYNTHETIC: u16 = 1 << 0;
}

/// Akce klavesy nebo tlacitka.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Release,
    Press,
    /// Automaticke opakovani drzene klavesy (generuje `inputd`).
    Repeat,
}

impl Action {
    pub const fn to_u32(self) -> u32 {
        match self {
            Action::Release => 0,
            Action::Press => 1,
            Action::Repeat => 2,
        }
    }

    pub const fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Action::Release),
            1 => Some(Action::Press),
            2 => Some(Action::Repeat),
            _ => None,
        }
    }
}

/// Tlacitko mysi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
    /// Bocni "zpet".
    Back,
    /// Bocni "vpred".
    Forward,
    /// Jine tlacitko (kod evdev).
    Other(u16),
}

impl Button {
    pub const fn from_code(code: u16) -> Self {
        use crate::code::*;
        match code {
            BTN_LEFT => Button::Left,
            BTN_RIGHT => Button::Right,
            BTN_MIDDLE => Button::Middle,
            BTN_SIDE => Button::Back,
            BTN_EXTRA => Button::Forward,
            c => Button::Other(c),
        }
    }

    pub const fn code(self) -> u16 {
        use crate::code::*;
        match self {
            Button::Left => BTN_LEFT,
            Button::Right => BTN_RIGHT,
            Button::Middle => BTN_MIDDLE,
            Button::Back => BTN_SIDE,
            Button::Forward => BTN_EXTRA,
            Button::Other(c) => c,
        }
    }
}

/// Udalost pro vlastnika fokusu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Fyzicka klavesa. `code` nezavisi na rozlozeni ([`crate::code`]),
    /// `scancode` je surovy kod zarizeni (u virtio-input stejny jako
    /// `code`), `ch` je znak, ktery klavesa v aktualnim rozlozeni
    /// a s aktualnim Shift/AltGr/Caps Lock dava - i kdyz je drzeny Ctrl
    /// (pro zkratky typu Ctrl+Z nezavisle na rozlozeni). Mrtva klavesa
    /// a klavesy bez znaku (Enter, sipky) maji `None`.
    Key {
        code: u16,
        scancode: u32,
        action: Action,
        ch: Option<char>,
    },
    /// Text k vlozeni (z rozlozeni, vcetne slozeni mrtve klavesy). Jen
    /// tisknutelne znaky vcetne mezery; Enter, Tab a Backspace jsou jen
    /// `Key`. Pri drzenem Ctrl, Alt nebo Meta se text neposila.
    Text(char),
    /// Relativni pohyb ukazatele (kladne doprava a dolu).
    Motion { dx: i32, dy: i32 },
    Button { button: Button, action: Action },
    /// Kolecko v kroccich (`dy` kladne nahoru / od uzivatele).
    Scroll { dx: i32, dy: i32 },
    /// Uzivatel stiskl bezpecnou zkratku (Ctrl+Alt+Delete).
    SecureAttention,
    /// Fokus ziskan (`true`) nebo ztracen. Po ztrate fokusu ma klient
    /// povazovat vsechny klavesy za uvolnene.
    Focus(bool),
    /// Pocet aktivnich zdroju umeleho vstupu (0 = zadny). Klient ma
    /// viditelne ukazat, ze vstup ovlada nekdo jiny.
    Injection { active: u32 },
    /// Aktivni rozlozeni klavesnice se zmenilo.
    Layout(u32),
    /// Klientovi neproslo `messages` zprav (mel plnou frontu). Prijde jako
    /// prvni zaznam dalsi dorucene zpravy. Klient ma (jako u `Focus(false)`)
    /// povazovat vsechny klavesy a tlacitka za uvolnene - ztracena mohla byt
    /// i uvolneni.
    Dropped { messages: u32 },
    /// Druh, ktery tato verze knihovny nezna.
    Unknown { kind: u16 },
}

/// Zaznam: udalost s casem, modifikatory a priznaky.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record {
    pub event: Event,
    /// Modifikatory po zpracovani udalosti ([`crate::mods`]).
    pub mods: u16,
    /// [`flags`].
    pub flags: u16,
    /// ns od startu (monotonni hodiny).
    pub time: u64,
}

impl Record {
    pub const SIZE: usize = 32;

    pub const fn new(event: Event, mods: u16, flags: u16, time: u64) -> Self {
        Self {
            event,
            mods,
            flags,
            time,
        }
    }

    pub const fn is_synthetic(&self) -> bool {
        self.flags & flags::SYNTHETIC != 0
    }

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let (k, a, b, c, d): (u16, u32, u32, u32, u32) = match self.event {
            Event::Key {
                code,
                scancode,
                action,
                ch,
            } => (
                kind::KEY,
                code as u32,
                scancode,
                action.to_u32(),
                ch.map_or(0, |c| c as u32),
            ),
            Event::Text(ch) => (kind::TEXT, ch as u32, 0, 0, 0),
            Event::Motion { dx, dy } => (kind::MOTION, dx as u32, dy as u32, 0, 0),
            Event::Button { button, action } => {
                (kind::BUTTON, button.code() as u32, 0, action.to_u32(), 0)
            }
            Event::Scroll { dx, dy } => (kind::SCROLL, dx as u32, dy as u32, 0, 0),
            Event::SecureAttention => (kind::SECURE_ATTENTION, 0, 0, 0, 0),
            Event::Focus(f) => (kind::FOCUS, f as u32, 0, 0, 0),
            Event::Injection { active } => (kind::INJECTION, active, 0, 0, 0),
            Event::Layout(id) => (kind::LAYOUT, id, 0, 0, 0),
            Event::Dropped { messages } => (kind::DROPPED, messages, 0, 0, 0),
            Event::Unknown { kind } => (kind, 0, 0, 0, 0),
        };
        let mut out = [0u8; Self::SIZE];
        out[0..2].copy_from_slice(&k.to_le_bytes());
        out[2..4].copy_from_slice(&self.flags.to_le_bytes());
        out[4..6].copy_from_slice(&self.mods.to_le_bytes());
        out[8..16].copy_from_slice(&self.time.to_le_bytes());
        out[16..20].copy_from_slice(&a.to_le_bytes());
        out[20..24].copy_from_slice(&b.to_le_bytes());
        out[24..28].copy_from_slice(&c.to_le_bytes());
        out[28..32].copy_from_slice(&d.to_le_bytes());
        out
    }

    /// Rozebere zaznam; `None` = kratky zaznam nebo neplatne hodnoty
    /// (akce, znak) u znameho druhu.
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        let b: &[u8; Self::SIZE] = b.get(..Self::SIZE)?.try_into().ok()?;
        let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
        let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let (a, bb, c, d) = (u32_at(16), u32_at(20), u32_at(24), u32_at(28));
        let k = u16_at(0);
        let event = match k {
            kind::KEY => Event::Key {
                code: u16::try_from(a).ok()?,
                scancode: bb,
                action: Action::from_u32(c)?,
                ch: if d == 0 {
                    None
                } else {
                    Some(char::from_u32(d)?)
                },
            },
            kind::TEXT => Event::Text(char::from_u32(a)?),
            kind::MOTION => Event::Motion {
                dx: a as i32,
                dy: bb as i32,
            },
            kind::BUTTON => Event::Button {
                button: Button::from_code(u16::try_from(a).ok()?),
                action: match Action::from_u32(c)? {
                    Action::Repeat => return None,
                    x => x,
                },
            },
            kind::SCROLL => Event::Scroll {
                dx: a as i32,
                dy: bb as i32,
            },
            kind::SECURE_ATTENTION => Event::SecureAttention,
            kind::FOCUS => Event::Focus(a != 0),
            kind::INJECTION => Event::Injection { active: a },
            kind::LAYOUT => Event::Layout(a),
            kind::DROPPED => Event::Dropped { messages: a },
            other => Event::Unknown { kind: other },
        };
        Some(Self {
            event,
            flags: u16_at(2),
            mods: u16_at(4),
            time: u64::from_le_bytes(b[8..16].try_into().ok()?),
        })
    }
}

/// Nejvic zaznamu v jedne zprave (vejde se do zpravy kanalu s rezervou).
pub const MAX_RECORDS: usize = 1024;

/// Zprava ze zaznamu (nejvys [`MAX_RECORDS`]).
pub fn encode(records: &[Record]) -> Vec<u8> {
    let records = &records[..records.len().min(MAX_RECORDS)];
    let mut v = Vec::with_capacity(records.len() * Record::SIZE);
    for r in records {
        v.extend_from_slice(&r.to_bytes());
    }
    v
}

/// Zaznamy ze zpravy; `None` = delka neni nasobek 32 B nebo neplatny zaznam.
pub fn decode(data: &[u8]) -> Option<Vec<Record>> {
    if data.len() % Record::SIZE != 0 {
        return None;
    }
    data.chunks_exact(Record::SIZE)
        .map(Record::from_bytes)
        .collect()
}
