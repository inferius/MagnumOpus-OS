//! Tabulky rozlozeni: pro kazdou klavesu ctyri urovne (zaklad, Shift,
//! AltGr, AltGr+Shift).

use inputproto::code::*;

use crate::Symbol;

const N: Symbol = Symbol::None;

const fn c(ch: char) -> Symbol {
    Symbol::Char(ch)
}

const fn d(accent: char) -> Symbol {
    Symbol::Dead(accent)
}

/// Pismeno: zaklad maly, Shift velky.
const fn l(lower: char, upper: char) -> [Symbol; 4] {
    [c(lower), c(upper), N, N]
}

/// Pismeno se znakem na urovni AltGr.
const fn la(lower: char, upper: char, altgr: char) -> [Symbol; 4] {
    [c(lower), c(upper), c(altgr), N]
}

/// Klavesa se dvema znaky (zaklad, Shift).
const fn k(base: char, shift: char) -> [Symbol; 4] {
    [c(base), c(shift), N, N]
}

/// Klavesa se dvema znaky a znakem AltGr.
const fn ka(base: char, shift: char, altgr: char) -> [Symbol; 4] {
    [c(base), c(shift), c(altgr), N]
}

pub type Table = &'static [(u16, [Symbol; 4])];

/// Americke rozlozeni (US QWERTY). Pravy Alt je Alt, ne AltGr.
pub const US: Table = &[
    (KEY_GRAVE, k('`', '~')),
    (KEY_1, k('1', '!')),
    (KEY_2, k('2', '@')),
    (KEY_3, k('3', '#')),
    (KEY_4, k('4', '$')),
    (KEY_5, k('5', '%')),
    (KEY_6, k('6', '^')),
    (KEY_7, k('7', '&')),
    (KEY_8, k('8', '*')),
    (KEY_9, k('9', '(')),
    (KEY_0, k('0', ')')),
    (KEY_MINUS, k('-', '_')),
    (KEY_EQUAL, k('=', '+')),
    (KEY_Q, l('q', 'Q')),
    (KEY_W, l('w', 'W')),
    (KEY_E, l('e', 'E')),
    (KEY_R, l('r', 'R')),
    (KEY_T, l('t', 'T')),
    (KEY_Y, l('y', 'Y')),
    (KEY_U, l('u', 'U')),
    (KEY_I, l('i', 'I')),
    (KEY_O, l('o', 'O')),
    (KEY_P, l('p', 'P')),
    (KEY_LEFTBRACE, k('[', '{')),
    (KEY_RIGHTBRACE, k(']', '}')),
    (KEY_BACKSLASH, k('\\', '|')),
    (KEY_A, l('a', 'A')),
    (KEY_S, l('s', 'S')),
    (KEY_D, l('d', 'D')),
    (KEY_F, l('f', 'F')),
    (KEY_G, l('g', 'G')),
    (KEY_H, l('h', 'H')),
    (KEY_J, l('j', 'J')),
    (KEY_K, l('k', 'K')),
    (KEY_L, l('l', 'L')),
    (KEY_SEMICOLON, k(';', ':')),
    (KEY_APOSTROPHE, k('\'', '"')),
    (KEY_Z, l('z', 'Z')),
    (KEY_X, l('x', 'X')),
    (KEY_C, l('c', 'C')),
    (KEY_V, l('v', 'V')),
    (KEY_B, l('b', 'B')),
    (KEY_N, l('n', 'N')),
    (KEY_M, l('m', 'M')),
    (KEY_COMMA, k(',', '<')),
    (KEY_DOT, k('.', '>')),
    (KEY_SLASH, k('/', '?')),
    (KEY_102ND, k('\\', '|')),
];

/// Ceske rozlozeni QWERTZ (jako "Czech" ve Windows). Mrtve klavesy: carka
/// a hacek na klavese vpravo od `=`, prehlaska na klavese nad Enterem.
/// Uroven AltGr pise znaky primo (`~`, `^`, `` ` `` nejsou mrtve), aby
/// se v terminalu daly psat bez mezery navic.
pub const CZ: Table = &[
    (KEY_GRAVE, k(';', '°')),
    (KEY_1, ka('+', '1', '~')),
    (KEY_2, k('ě', '2')),
    (KEY_3, ka('š', '3', '^')),
    (KEY_4, k('č', '4')),
    (KEY_5, k('ř', '5')),
    (KEY_6, k('ž', '6')),
    (KEY_7, ka('ý', '7', '`')),
    (KEY_8, k('á', '8')),
    (KEY_9, k('í', '9')),
    (KEY_0, k('é', '0')),
    (KEY_MINUS, k('=', '%')),
    (KEY_EQUAL, [d('´'), d('ˇ'), N, N]),
    (KEY_Q, la('q', 'Q', '\\')),
    (KEY_W, la('w', 'W', '|')),
    (KEY_E, la('e', 'E', '€')),
    (KEY_R, l('r', 'R')),
    (KEY_T, l('t', 'T')),
    (KEY_Y, l('z', 'Z')),
    (KEY_U, l('u', 'U')),
    (KEY_I, l('i', 'I')),
    (KEY_O, l('o', 'O')),
    (KEY_P, l('p', 'P')),
    (KEY_LEFTBRACE, ka('ú', '/', '÷')),
    (KEY_RIGHTBRACE, ka(')', '(', '×')),
    (KEY_BACKSLASH, [d('¨'), c('\''), c('¤'), N]),
    (KEY_A, l('a', 'A')),
    (KEY_S, la('s', 'S', 'đ')),
    (KEY_D, la('d', 'D', 'Đ')),
    (KEY_F, la('f', 'F', '[')),
    (KEY_G, la('g', 'G', ']')),
    (KEY_H, l('h', 'H')),
    (KEY_J, l('j', 'J')),
    (KEY_K, la('k', 'K', 'ł')),
    (KEY_L, la('l', 'L', 'Ł')),
    (KEY_SEMICOLON, ka('ů', '"', '$')),
    (KEY_APOSTROPHE, ka('§', '!', 'ß')),
    (KEY_Z, l('y', 'Y')),
    (KEY_X, la('x', 'X', '#')),
    (KEY_C, la('c', 'C', '&')),
    (KEY_V, la('v', 'V', '@')),
    (KEY_B, la('b', 'B', '{')),
    (KEY_N, la('n', 'N', '}')),
    (KEY_M, l('m', 'M')),
    (KEY_COMMA, ka(',', '?', '<')),
    (KEY_DOT, ka('.', ':', '>')),
    (KEY_SLASH, ka('-', '_', '*')),
    (KEY_102ND, k('\\', '|')),
];

/// Klavesy spolecne vsem rozlozenim (mezernik).
pub const COMMON: Table = &[(KEY_SPACE, [c(' '), c(' '), c(' '), c(' ')])];

/// Numericka klavesnice: znaky operaci vzdy, cislice a tecka jen se
/// zapnutym Num Lockem (jinak jsou to sipky, Home, End ...).
pub const KEYPAD_OPS: &[(u16, char)] = &[
    (KEY_KPSLASH, '/'),
    (KEY_KPASTERISK, '*'),
    (KEY_KPMINUS, '-'),
    (KEY_KPPLUS, '+'),
];

pub const KEYPAD_DIGITS: &[(u16, char)] = &[
    (KEY_KP0, '0'),
    (KEY_KP1, '1'),
    (KEY_KP2, '2'),
    (KEY_KP3, '3'),
    (KEY_KP4, '4'),
    (KEY_KP5, '5'),
    (KEY_KP6, '6'),
    (KEY_KP7, '7'),
    (KEY_KP8, '8'),
    (KEY_KP9, '9'),
];
