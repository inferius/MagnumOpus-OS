//! Zalamovani textu stitku po slovech podle sirky z rozvrzeni.
//!
//! - Radky se lame na mezerach; `\n` je vynuceny konec radku.
//! - Slovo sirsi nez radek (cesta, adresa) se rozdeli: prednostne za
//!   oddelovacem (`/`, `\`, `-`, `_`, `.`, `:`), jinak kde se vejde.
//! - `max` radku (0 = bez omezeni): posledni radek dostane zbytek textu
//!   a pri kresleni se zkrati s `...` (`Text::draw_ellipsized`).
//!
//! Jadro je bez pisma: sirku meri predana funkce (testovatelne bez fontu).

use alloc::string::String;
use alloc::vec::Vec;

/// Znaky, za kterymi se dlouhe slovo rozdeli nejradeji.
fn is_break(c: char) -> bool {
    matches!(c, '/' | '\\' | '-' | '_' | '.' | ':' | ',' | ';')
}

/// Rozdeli text na radky sirky nejvys `width` (meri `w`). Vraci aspon
/// jeden radek. S `max > 0` je radku nejvys `max` a posledni nese zbytek
/// textu (muze byt sirsi, kreslic ho zkrati).
pub fn wrap(text: &str, width: u32, max: usize, w: &dyn Fn(&str) -> u32) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // Zacatek (bajt) zbytku textu od kazdeho radku: pri `max` se posledni
    // radek nahradi celym zbytkem.
    let mut starts: Vec<usize> = Vec::new();
    let mut base = 0;
    for para in text.split('\n') {
        wrap_para(para, base, width, w, &mut out, &mut starts);
        base += para.len() + 1;
    }
    if max > 0 && out.len() > max {
        out.truncate(max);
        let rest = text[starts[max - 1]..].replace('\n', " ");
        out[max - 1] = String::from(rest.trim_end());
    }
    out
}

/// Jeden odstavec (bez `\n`); `base` = jeho pocatek v celem textu.
fn wrap_para(para: &str, base: usize, width: u32, w: &dyn Fn(&str) -> u32, out: &mut Vec<String>, starts: &mut Vec<usize>) {
    if w(para) <= width {
        starts.push(base);
        out.push(String::from(para));
        return;
    }
    let mut line = String::new();
    let mut line_start = base;
    // Slova i s pozici (mezery mezi nimi se na zlomu zahodi).
    let mut pos = 0;
    for word in para.split(' ') {
        let at = base + pos;
        pos += word.len() + 1;
        if word.is_empty() {
            // Vic mezer za sebou: zachovat uvnitr radku.
            if !line.is_empty() {
                line.push(' ');
            }
            continue;
        }
        let cand = if line.is_empty() { String::from(word) } else { alloc::format!("{line} {word}") };
        if w(&cand) <= width {
            if line.is_empty() {
                line_start = at;
            }
            line = cand;
            continue;
        }
        if !line.is_empty() {
            starts.push(line_start);
            out.push(core::mem::take(&mut line));
        }
        // Slovo se nevejde ani samo: rozdelit.
        let mut rest = word;
        let mut rest_at = at;
        while w(rest) > width {
            let cut = split_point(rest, width, w);
            starts.push(rest_at);
            out.push(String::from(&rest[..cut]));
            rest = &rest[cut..];
            rest_at += cut;
        }
        line = String::from(rest);
        line_start = rest_at;
    }
    let line = String::from(line.trim_end());
    if !line.is_empty() || out.is_empty() {
        starts.push(line_start);
        out.push(line);
    }
}

/// Kde rozdelit slovo sirsi nez `width`: za posledni oddelovac, ktery se
/// vejde, jinak za posledni znak, ktery se vejde (aspon jeden znak).
fn split_point(word: &str, width: u32, w: &dyn Fn(&str) -> u32) -> usize {
    let mut fit = 0;
    let mut brk = 0;
    for (i, c) in word.char_indices() {
        let end = i + c.len_utf8();
        if w(&word[..end]) > width {
            break;
        }
        fit = end;
        if is_break(c) && end < word.len() {
            brk = end;
        }
    }
    if fit == 0 {
        // Ani jeden znak se nevejde: aspon jeden, jinak by se necyklilo.
        return word.chars().next().map_or(word.len(), |c| c.len_utf8());
    }
    if brk > 0 { brk } else { fit }
}
