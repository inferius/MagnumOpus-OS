//! Vicebajtove kodovani znaku (`mbrtowc`, `wcrtomb`): stavovy automat
//! UTF-8 a jednobajtove kodovani locale C.
//!
//! Locale C/POSIX je jednobajtove (`MB_CUR_MAX` = 1) a reverzibilni jako
//! v musl: bajty 0x00-0x7F jsou ASCII, bajty 0x80-0xFF se mapuji na
//! U+DF80-U+DFFF (nahradni polovicky, ktere zadny platny text neobsahuje)
//! a zpet. Program v locale C tak muze libovolna data projit pres siroke
//! znaky bez ztraty. Locale s UTF-8 (`C.UTF-8` a ostatni jmena) prijima jen
//! platne UTF-8 (bez prilis dlouhych zapisu a nahradnich polovicek).
//!
//! Ktere kodovani plati, rozhoduje locale vlakna (`LC_CTYPE`); to zna az
//! `cstd`, ktery si pri prvni zmene locale zaregistruje dotaz
//! ([`set_ctype_hook`]). Bez nej (sluzby no_std, program bez `setlocale`)
//! plati locale C.

use core::sync::atomic::{AtomicUsize, Ordering};

/// Kodovani vicebajtovych znaku.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enc {
    /// Locale C/POSIX: jeden bajt = jeden znak.
    C,
    /// UTF-8, 1-4 bajty na znak.
    Utf8,
}

/// Stav prevodu (`mbstate_t`, 8 B; nuly = pocatecni stav).
///
/// UTF-8: `count` dolnich 8 bitu = kolik bajtu jeste chybi, bity 8-15 =
/// povoleny rozsah dalsiho bajtu (jen pro druhy bajt znaku), `value` =
/// dosud slozene bity znaku.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MbState {
    pub count: u32,
    pub value: u32,
}

impl MbState {
    pub const INITIAL: MbState = MbState { count: 0, value: 0 };

    /// Je stav pocatecni (`mbsinit`)?
    pub fn is_initial(&self) -> bool {
        self.count & 0xFF == 0
    }
}

/// Navratove hodnoty `mbrtowc` jako `size_t`.
pub const MB_ERROR: usize = usize::MAX; // (size_t)-1, EILSEQ
pub const MB_INCOMPLETE: usize = usize::MAX - 1; // (size_t)-2

/// Prvni znak nahradniho rozsahu pro bajty 0x80-0xFF v locale C.
const C_HIGH: u32 = 0xDF00;

/// Rozsah povoleneho druheho bajtu UTF-8 podle prvniho bajtu (tabulka 3-7
/// standardu Unicode): 0 = 80..BF, 1 = A0..BF (E0), 2 = 80..9F (ED),
/// 3 = 90..BF (F0), 4 = 80..8F (F4).
fn second_ok(kind: u32, b: u8) -> bool {
    match kind {
        1 => (0xA0..=0xBF).contains(&b),
        2 => (0x80..=0x9F).contains(&b),
        3 => (0x90..=0xBF).contains(&b),
        4 => (0x80..=0x8F).contains(&b),
        _ => (0x80..=0xBF).contains(&b),
    }
}

/// Vysledek jednoho bajtu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Hotovy znak.
    Char(u32),
    /// Znak pokracuje dalsim bajtem.
    More,
    /// Neplatna posloupnost (stav je nedefinovany, volajici ho vynuluje).
    Bad,
}

impl Enc {
    /// Nejvic bajtu na znak (`MB_CUR_MAX`).
    pub fn max_len(self) -> usize {
        match self {
            Enc::C => 1,
            Enc::Utf8 => 4,
        }
    }

    /// Jeden bajt vstupu.
    pub fn feed(self, st: &mut MbState, b: u8) -> Step {
        if self == Enc::C {
            return Step::Char(if b < 0x80 { b as u32 } else { C_HIGH | b as u32 });
        }
        let need = st.count & 0xFF;
        if need == 0 {
            let (n, kind, bits) = match b {
                0x00..=0x7F => return Step::Char(b as u32),
                0xC2..=0xDF => (1, 0, b as u32 & 0x1F),
                0xE0 => (2, 1, 0),
                0xED => (2, 2, 0x0D),
                0xE1..=0xEF => (2, 0, b as u32 & 0x0F),
                0xF0 => (3, 3, 0),
                0xF1..=0xF3 => (3, 0, b as u32 & 0x07),
                0xF4 => (3, 4, 4),
                _ => return Step::Bad,
            };
            *st = MbState { count: n | (kind << 8), value: bits };
            return Step::More;
        }
        let kind = (st.count >> 8) & 0xFF;
        if !second_ok(kind, b) {
            return Step::Bad;
        }
        let value = (st.value << 6) | (b as u32 & 0x3F);
        if need == 1 {
            *st = MbState::INITIAL;
            Step::Char(value)
        } else {
            *st = MbState { count: need - 1, value };
            Step::More
        }
    }

    /// `mbrtowc` nad bajty `s` (uz bez ukazatele): vraci znak (pokud je)
    /// a pocet spotrebovanych bajtu, 0 pro NUL, `MB_INCOMPLETE` (vsechny
    /// bajty spotrebovany do stavu) nebo `MB_ERROR`.
    pub fn mbrtowc(self, s: &[u8], st: &mut MbState) -> (Option<u32>, usize) {
        for (i, &b) in s.iter().enumerate() {
            match self.feed(st, b) {
                Step::Char(c) => return (Some(c), if c == 0 { 0 } else { i + 1 }),
                Step::More => {}
                Step::Bad => {
                    *st = MbState::INITIAL;
                    return (None, MB_ERROR);
                }
            }
        }
        (None, MB_INCOMPLETE)
    }

    /// Prvni znak celeho retezce `s` (bez stavu): znak a jeho delka, `None`
    /// = neplatny nebo neuplny.
    pub fn decode(self, s: &[u8]) -> Option<(u32, usize)> {
        let mut st = MbState::INITIAL;
        for (i, &b) in s.iter().enumerate() {
            match self.feed(&mut st, b) {
                Step::Char(c) => return Some((c, i + 1)),
                Step::More => {}
                Step::Bad => return None,
            }
        }
        None
    }

    /// `wcrtomb`: zakoduje znak, vraci pocet bajtu nebo `None` (EILSEQ).
    pub fn encode(self, wc: u32, buf: &mut [u8; 4]) -> Option<usize> {
        match self {
            Enc::C => {
                // ASCII nebo bajt 0x80-0xFF ulozeny jako U+DF80-U+DFFF.
                if wc < 0x80 || (C_HIGH | 0x80..=C_HIGH | 0xFF).contains(&wc) {
                    buf[0] = wc as u8;
                    Some(1)
                } else {
                    None
                }
            }
            Enc::Utf8 => {
                let ch = char::from_u32(wc)?;
                Some(ch.encode_utf8(buf).len())
            }
        }
    }
}

/// Dotaz na kodovani aktualniho locale (0 = zadny, plati C).
static CTYPE_HOOK: AtomicUsize = AtomicUsize::new(0);

/// Zaregistruje dotaz na kodovani aktualniho locale (vola `cstd` pri prvni
/// zmene locale).
pub fn set_ctype_hook(f: fn() -> Enc) {
    CTYPE_HOOK.store(f as usize, Ordering::Release);
}

/// Kodovani aktualniho locale vlakna.
pub fn current() -> Enc {
    let f = CTYPE_HOOK.load(Ordering::Acquire);
    if f == 0 {
        return Enc::C;
    }
    // SAFETY: hodnota je ukazatel na `fn() -> Enc` ze `set_ctype_hook`.
    let f: fn() -> Enc = unsafe { core::mem::transmute::<usize, fn() -> Enc>(f) };
    f()
}
