//! Rozlozeni klavesnice: prevod fyzicke klavesy (kod evdev) na znak podle
//! rozlozeni, stav modifikatoru a skladani mrtvych klaves.
//!
//! Rozlozeni v prvni verzi: americke (`us`) a ceske QWERTZ (`cz`). Pouziva
//! ho sluzba `inputd` (jedno misto pro cely system, ne kod v kazde
//! aplikaci); knihovna nezavisi na jadre a testuje se na hostiteli
//! (`cargo test -p keymap`).
//!
//! ```
//! use inputproto::{Action, code::*};
//! use keymap::{Keyboard, LayoutId, layout};
//!
//! let cz = layout(LayoutId::Cz);
//! let mut kb = Keyboard::new();
//! // Mrtva klavesa hacek (Shift + klavesa vpravo od `=`), pak `e`.
//! kb.key(cz, KEY_LEFTSHIFT, Action::Press);
//! assert_eq!(kb.key(cz, KEY_EQUAL, Action::Press).text, [None, None]);
//! kb.key(cz, KEY_LEFTSHIFT, Action::Release);
//! assert_eq!(kb.key(cz, KEY_E, Action::Press).text, [Some('ě'), None]);
//! ```

#![cfg_attr(not(test), no_std)]

mod compose;
mod layouts;

pub use compose::compose;

use inputproto::Action;
use inputproto::code::*;
use inputproto::mods;

/// Vyznam klavesy na jedne urovni rozlozeni.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Symbol {
    /// Klavesa na teto urovni nic nepise.
    None,
    Char(char),
    /// Mrtva klavesa: znamenko (samostatny znak, napr. `´`) se slozi
    /// s pristim znakem.
    Dead(char),
}

/// Id rozlozeni (v udalosti `Layout` a v prikazu `layout`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum LayoutId {
    Us = 0,
    Cz = 1,
}

impl LayoutId {
    pub const ALL: [LayoutId; 2] = [LayoutId::Us, LayoutId::Cz];

    /// Kratke jmeno (`us`, `cz`).
    pub const fn name(self) -> &'static str {
        match self {
            LayoutId::Us => "us",
            LayoutId::Cz => "cz",
        }
    }

    /// Popis pro lidi (instalator, nastaveni).
    pub const fn description(self) -> &'static str {
        match self {
            LayoutId::Us => "Anglická (US)",
            LayoutId::Cz => "Česká (QWERTZ)",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.name() == name)
    }

    pub fn from_u32(v: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|l| *l as u32 == v)
    }
}

/// Rozlozeni klavesnice.
#[derive(Debug)]
pub struct Layout {
    pub id: LayoutId,
    table: layouts::Table,
    /// Pravy Alt je AltGr (treti uroven), ne Alt.
    pub altgr: bool,
    /// Znak desetinne klavesy na numericke klavesnici.
    kp_decimal: char,
}

static US: Layout = Layout {
    id: LayoutId::Us,
    table: layouts::US,
    altgr: false,
    kp_decimal: '.',
};

static CZ: Layout = Layout {
    id: LayoutId::Cz,
    table: layouts::CZ,
    altgr: true,
    kp_decimal: ',',
};

/// Rozlozeni podle id.
pub fn layout(id: LayoutId) -> &'static Layout {
    match id {
        LayoutId::Us => &US,
        LayoutId::Cz => &CZ,
    }
}

/// Radky klaves pro nahled rozlozeni (kody evdev: cislice, horni, stredni
/// a dolni rada pismen).
pub const PREVIEW_ROWS: [&[u16]; 4] = [
    &[2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
    &[16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27],
    &[30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40],
    &[44, 45, 46, 47, 48, 49, 50, 51, 52, 53],
];

impl Layout {
    /// Nahled rady klaves `row` ([`PREVIEW_ROWS`]): znaky zakladni urovne
    /// (mrtva klavesa jako jeji znak, jina klavesa jako mezera).
    pub fn preview_row(&self, row: usize) -> impl Iterator<Item = char> + '_ {
        PREVIEW_ROWS.get(row).copied().unwrap_or(&[]).iter().map(move |&c| match self.symbol(c, 0) {
            Symbol::Char(ch) | Symbol::Dead(ch) => ch,
            _ => ' ',
        })
    }

    /// Symbol klavesy na urovni `level` (0 zaklad, 1 Shift, 2 AltGr,
    /// 3 AltGr+Shift). Numerickou klavesnici resi [`Keyboard`] (zavisi na
    /// Num Locku).
    pub fn symbol(&self, code: u16, level: usize) -> Symbol {
        self.table
            .iter()
            .chain(layouts::COMMON.iter())
            .find(|(c, _)| *c == code)
            .map_or(Symbol::None, |(_, levels)| levels[level & 3])
    }
}

// Vnitrni bity drzenych modifikatoru (levy a pravy zvlast).
const H_LSHIFT: u16 = 1 << 0;
const H_RSHIFT: u16 = 1 << 1;
const H_LCTRL: u16 = 1 << 2;
const H_RCTRL: u16 = 1 << 3;
const H_LALT: u16 = 1 << 4;
const H_RALT: u16 = 1 << 5;
const H_LMETA: u16 = 1 << 6;
const H_RMETA: u16 = 1 << 7;

/// Bit drzeneho modifikatoru pro kod klavesy.
fn modifier_bit(code: u16) -> Option<u16> {
    Some(match code {
        KEY_LEFTSHIFT => H_LSHIFT,
        KEY_RIGHTSHIFT => H_RSHIFT,
        KEY_LEFTCTRL => H_LCTRL,
        KEY_RIGHTCTRL => H_RCTRL,
        KEY_LEFTALT => H_LALT,
        KEY_RIGHTALT => H_RALT,
        KEY_LEFTMETA => H_LMETA,
        KEY_RIGHTMETA => H_RMETA,
        _ => return None,
    })
}

/// Je kod modifikator nebo prepinac (Shift, Ctrl, Alt, Meta, Caps Lock,
/// Num Lock)? Takova klavesa nepise a neopakuje se.
pub fn is_modifier(code: u16) -> bool {
    modifier_bit(code).is_some() || matches!(code, KEY_CAPSLOCK | KEY_NUMLOCK | KEY_SCROLLLOCK)
}

/// Vysledek jedne klavesove udalosti.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyOutput {
    /// Modifikatory po udalosti ([`inputproto::mods`]).
    pub mods: u16,
    /// Znak klavesy v rozlozeni (pole `ch` udalosti `Key`); pocita se i pri
    /// drzenem Ctrl. Mrtva klavesa a klavesy bez znaku: `None`.
    pub ch: Option<char>,
    /// Text k vlozeni: nejvys dva znaky (znamenko mrtve klavesy, ktere
    /// nejde slozit, a pak znak).
    pub text: [Option<char>; 2],
}

/// Stav jedne klavesnice: drzene modifikatory, prepinace a cekajici mrtva
/// klavesa.
#[derive(Debug, Clone, Default)]
pub struct Keyboard {
    held: u16,
    caps: bool,
    num: bool,
    dead: Option<char>,
}

impl Keyboard {
    pub const fn new() -> Self {
        Self {
            held: 0,
            caps: false,
            num: false,
            dead: None,
        }
    }

    /// Modifikatory ve tvaru protokolu (prave Alt podle rozlozeni: AltGr
    /// nebo Alt).
    pub fn mods(&self, layout: &Layout) -> u16 {
        let mut m = 0;
        if self.held & (H_LSHIFT | H_RSHIFT) != 0 {
            m |= mods::SHIFT;
        }
        if self.held & (H_LCTRL | H_RCTRL) != 0 {
            m |= mods::CTRL;
        }
        if self.held & H_LALT != 0 {
            m |= mods::ALT;
        }
        if self.held & H_RALT != 0 {
            m |= if layout.altgr { mods::ALTGR } else { mods::ALT };
        }
        if self.held & (H_LMETA | H_RMETA) != 0 {
            m |= mods::META;
        }
        if self.caps {
            m |= mods::CAPS_LOCK;
        }
        if self.num {
            m |= mods::NUM_LOCK;
        }
        m
    }

    /// Drzi uzivatel Ctrl a Alt (kterykoli, i AltGr)? Pro bezpecnou zkratku.
    pub fn ctrl_alt_held(&self) -> bool {
        self.held & (H_LCTRL | H_RCTRL) != 0 && self.held & (H_LALT | H_RALT) != 0
    }

    /// Ceka mrtva klavesa na dalsi znak?
    pub fn dead_pending(&self) -> Option<char> {
        self.dead
    }

    /// Uvolni drzene modifikatory a zrusi mrtvou klavesu (napr. pri ztrate
    /// fokusu). Caps Lock a Num Lock zustavaji.
    pub fn reset(&mut self) {
        self.held = 0;
        self.dead = None;
    }

    /// Zpracuje klavesu a vrati modifikatory, znak a text.
    pub fn key(&mut self, layout: &Layout, code: u16, action: Action) -> KeyOutput {
        if let Some(bit) = modifier_bit(code) {
            match action {
                Action::Press | Action::Repeat => self.held |= bit,
                Action::Release => self.held &= !bit,
            }
            return self.output(layout, None, [None, None]);
        }
        if action == Action::Press {
            match code {
                KEY_CAPSLOCK => self.caps = !self.caps,
                KEY_NUMLOCK => self.num = !self.num,
                _ => {}
            }
        }
        if is_modifier(code) {
            return self.output(layout, None, [None, None]);
        }
        let symbol = self.resolve(layout, code);
        let ch = match symbol {
            Symbol::Char(c) => Some(c),
            _ => None,
        };
        if action == Action::Release {
            return self.output(layout, ch, [None, None]);
        }
        let m = self.mods(layout);
        if m & (mods::CTRL | mods::ALT | mods::META) != 0 {
            // Zkratka: text se nevklada a rozepsana mrtva klavesa propada.
            self.dead = None;
            return self.output(layout, ch, [None, None]);
        }
        let text = match symbol {
            Symbol::Dead(_) if action == Action::Repeat => [None, None],
            Symbol::Dead(accent) => match self.dead.take() {
                // Dvakrat stejne znamenko = znamenko samotne.
                Some(prev) if prev == accent => [Some(accent), None],
                Some(prev) => {
                    self.dead = Some(accent);
                    [Some(prev), None]
                }
                None => {
                    self.dead = Some(accent);
                    [None, None]
                }
            },
            Symbol::Char(c) => match self.dead.take() {
                None => [Some(c), None],
                Some(accent) => match compose(accent, c) {
                    Some(x) => [Some(x), None],
                    None if c == ' ' => [Some(accent), None],
                    None => [Some(accent), Some(c)],
                },
            },
            Symbol::None => {
                // Klavesa bez znaku (Enter, sipka) mrtvou klavesu zrusi.
                self.dead = None;
                [None, None]
            }
        };
        self.output(layout, ch, text)
    }

    fn output(&self, layout: &Layout, ch: Option<char>, text: [Option<char>; 2]) -> KeyOutput {
        KeyOutput {
            mods: self.mods(layout),
            ch,
            text,
        }
    }

    /// Symbol klavesy podle drzenych modifikatoru a prepinacu.
    fn resolve(&self, layout: &Layout, code: u16) -> Symbol {
        if let Some(&(_, ch)) = layouts::KEYPAD_OPS.iter().find(|(c, _)| *c == code) {
            return Symbol::Char(ch);
        }
        if let Some(&(_, ch)) = layouts::KEYPAD_DIGITS.iter().find(|(c, _)| *c == code) {
            return if self.num { Symbol::Char(ch) } else { Symbol::None };
        }
        if code == KEY_KPDOT {
            return if self.num {
                Symbol::Char(layout.kp_decimal)
            } else {
                Symbol::None
            };
        }
        let shift = self.held & (H_LSHIFT | H_RSHIFT) != 0;
        let altgr = layout.altgr && self.held & H_RALT != 0;
        let level = (shift as usize) | ((altgr as usize) << 1);
        if altgr || !self.caps {
            return layout.symbol(code, level);
        }
        // Caps Lock: u pismen prohodi velikost (Shift pak pise male), u
        // klaves s pismenem jen v zakladu (ceska horni rada) pise velke
        // pismeno a se Shiftem cislici.
        let base = layout.symbol(code, 0);
        let upper = layout.symbol(code, 1);
        match (base, upper) {
            (Symbol::Char(b), Symbol::Char(u)) if b.is_alphabetic() && to_upper(b) == Some(u) => {
                if shift { base } else { upper }
            }
            (Symbol::Char(b), _) if b.is_alphabetic() && !shift => {
                to_upper(b).map_or(base, Symbol::Char)
            }
            _ => layout.symbol(code, level),
        }
    }
}

/// Velke pismeno, pokud je to jeden znak.
fn to_upper(c: char) -> Option<char> {
    let mut it = c.to_uppercase();
    let u = it.next()?;
    if it.next().is_some() { None } else { Some(u) }
}

#[cfg(test)]
mod tests;
