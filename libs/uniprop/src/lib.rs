//! Vlastnosti znaku Unicode bez stavu a bez alokace (`no_std`): tridy
//! znaku pro `wctype.h`, jednoduche prevody velikosti pismen a sirka znaku
//! v terminalu (`wcwidth`).
//!
//! Tabulky (`tables.rs`) generuje `cargo xtask unicode-tables` z Unicode
//! Character Database ve verzi `core::char::UNICODE_VERSION` - C a Rust tak
//! odpovidaji stejne (`iswalpha` = `char::is_alphabetic`). Vyhledavani je
//! binarni v serazenych rozsazich, ASCII ma rychlou cestu.

#![cfg_attr(not(test), no_std)]

mod tables;

pub use tables::UNICODE_VERSION;

fn in_ranges(t: &[(u32, u32)], c: u32) -> bool {
    t.binary_search_by(|&(a, b)| {
        if b < c {
            core::cmp::Ordering::Less
        } else if a > c {
            core::cmp::Ordering::Greater
        } else {
            core::cmp::Ordering::Equal
        }
    })
    .is_ok()
}

fn map_case(t: &[(u32, u32, u32, i32)], c: u32) -> u32 {
    let i = t.partition_point(|&(a, _, _, _)| a <= c);
    if i == 0 {
        return c;
    }
    let (a, b, step, delta) = t[i - 1];
    if c <= b && (c - a).is_multiple_of(step) { (c as i64 + delta as i64) as u32 } else { c }
}

/// Pismeno (vlastnost Alphabetic).
pub fn is_alpha(c: u32) -> bool {
    if c < 0x80 {
        return (c as u8).is_ascii_alphabetic();
    }
    in_ranges(tables::ALPHABETIC, c)
}

/// Male pismeno (vlastnost Lowercase).
pub fn is_lower(c: u32) -> bool {
    if c < 0x80 {
        return (c as u8).is_ascii_lowercase();
    }
    in_ranges(tables::LOWERCASE, c)
}

/// Velke pismeno (vlastnost Uppercase).
pub fn is_upper(c: u32) -> bool {
    if c < 0x80 {
        return (c as u8).is_ascii_uppercase();
    }
    in_ranges(tables::UPPERCASE, c)
}

/// Desitkova cislice: v C jen `0`-`9` (standard), ne cislice jinych pisem.
pub fn is_digit(c: u32) -> bool {
    (0x30..=0x39).contains(&c)
}

/// Sestnactkova cislice (jen ASCII).
pub fn is_xdigit(c: u32) -> bool {
    c < 0x80 && (c as u8).is_ascii_hexdigit()
}

/// Bily znak (White_Space bez mezer bez zalomeni, jako glibc a musl).
pub fn is_space(c: u32) -> bool {
    if c < 0x80 {
        return matches!(c, 0x09..=0x0D | 0x20);
    }
    in_ranges(tables::SPACE, c)
}

/// Mezera v radku (tabulator a Zs bez mezer bez zalomeni).
pub fn is_blank(c: u32) -> bool {
    if c < 0x80 {
        return c == 0x09 || c == 0x20;
    }
    in_ranges(tables::BLANK, c)
}

/// Ridici znak (Cc, Zl, Zp).
pub fn is_cntrl(c: u32) -> bool {
    in_ranges(tables::CNTRL, c)
}

/// Tisknutelny znak: prirazeny a ne Cc, Cs, Zl, Zp (vcetne mezer
/// a soukromych znaku).
pub fn is_print(c: u32) -> bool {
    if c < 0x80 {
        return (0x20..0x7F).contains(&c);
    }
    c <= 0x10FFFF && !in_ranges(tables::NONPRINT, c)
}

/// Viditelny znak: tisknutelny a ne bily.
pub fn is_graph(c: u32) -> bool {
    is_print(c) && !is_space(c) && !is_blank(c)
}

/// Pismeno nebo cislice.
pub fn is_alnum(c: u32) -> bool {
    is_alpha(c) || is_digit(c)
}

/// Interpunkce a symboly: viditelny znak, ktery neni pismeno ani cislice.
pub fn is_punct(c: u32) -> bool {
    is_graph(c) && !is_alnum(c)
}

/// Jednoduchy prevod na velke pismeno (UnicodeData, pole 12).
pub fn to_upper(c: u32) -> u32 {
    if c < 0x80 {
        return (c as u8).to_ascii_uppercase() as u32;
    }
    map_case(tables::TO_UPPER, c)
}

/// Jednoduchy prevod na male pismeno (UnicodeData, pole 13).
pub fn to_lower(c: u32) -> u32 {
    if c < 0x80 {
        return (c as u8).to_ascii_lowercase() as u32;
    }
    map_case(tables::TO_LOWER, c)
}

/// Sirka znaku v bunkach terminalu (`wcwidth`): 0 pro NUL a znaky nulove
/// sirky (kombinujici znamenka, format), 2 pro siroke (East_Asian_Width
/// W/F), -1 pro netisknutelne, jinak 1.
pub fn width(c: u32) -> i32 {
    if c == 0 {
        return 0;
    }
    if c < 0x80 {
        return if (0x20..0x7F).contains(&c) { 1 } else { -1 };
    }
    if !is_print(c) {
        return -1;
    }
    if in_ranges(tables::ZERO_WIDTH, c) {
        0
    } else if in_ranges(tables::WIDE, c) {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests;
