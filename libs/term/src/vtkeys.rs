//! Vstup z terminalu VT100/xterm (klient SSH, seriova linka) -> klavesy
//! okna terminalu ([`crate::ui::Key`]).
//!
//! Bajty prichazi po kouscich (paket SSH muze sekvenci rozdelit), proto
//! se nedokoncena sekvence a nedokonceny znak UTF-8 drzi do dalsiho
//! `feed`. Samotne Esc (bez pokracovani) vrati az [`VtKeys::flush`] (volat
//! po kratke pauze bez dalsich dat), jinak by sipka rozdelena do dvou
//! paketu dala Esc a pak tri znaky. Nezname sekvence se zahodi cele.

use alloc::vec::Vec;

use crate::ui::Key;

/// Nejdelsi sekvence, kterou drzime (delsi = nesmysl, zahodi se).
const MAX_SEQ: usize = 32;

#[derive(Default)]
pub struct VtKeys {
    pending: Vec<u8>,
}

/// Konec sekvence CSI (`ESC [ ... final`): bajt 0x40..=0x7E.
fn csi_end(b: u8) -> bool {
    (0x40..=0x7E).contains(&b)
}

/// Klavesa ze sekvence CSI (parametry bez `ESC [`, posledni bajt je
/// koncovy).
fn csi(seq: &[u8]) -> Option<Key> {
    let (&fin, params) = seq.split_last()?;
    let params = core::str::from_utf8(params).ok()?;
    let mut it = params.split(';');
    let first: u32 = it.next().filter(|s| !s.is_empty()).and_then(|s| s.parse().ok()).unwrap_or(1);
    let modifier: u32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    // xterm: modifikator 1 + (shift 1, alt 2, ctrl 4).
    let (shift, alt) = (modifier.saturating_sub(1) & 1 != 0, modifier.saturating_sub(1) & 2 != 0);
    Some(match fin {
        b'A' => Key::Up,
        b'B' => Key::Down,
        b'C' if alt => Key::AltRight,
        b'D' if alt => Key::AltLeft,
        b'C' => Key::Right,
        b'D' => Key::Left,
        b'H' => Key::Home,
        b'F' => Key::End,
        b'Z' => Key::BackTab,
        b'~' => match first {
            1 | 7 => Key::Home,
            4 | 8 => Key::End,
            3 => Key::Delete,
            5 if shift => Key::ScrollPageUp,
            6 if shift => Key::ScrollPageDown,
            5 => Key::PageUp,
            6 => Key::PageDown,
            _ => return None,
        },
        _ => return None,
    })
}

/// Klavesa ze sekvence SS3 (`ESC O x`, aplikacni rezim kurzoru).
fn ss3(b: u8) -> Option<Key> {
    Some(match b {
        b'A' => Key::Up,
        b'B' => Key::Down,
        b'C' => Key::Right,
        b'D' => Key::Left,
        b'H' => Key::Home,
        b'F' => Key::End,
        b'M' => Key::Enter,
        _ => return None,
    })
}

/// Ridici znak -> klavesa.
fn control(b: u8) -> Option<Key> {
    Some(match b {
        b'\r' | b'\n' => Key::Enter,
        b'\t' => Key::Tab,
        0x7F | 0x08 => Key::Backspace,
        0x01..=0x1A => Key::Ctrl((b'a' + b - 1) as char),
        _ => return None,
    })
}

/// Delka znaku UTF-8 podle prvniho bajtu (0 = neplatny prvni bajt).
fn utf8_len(b: u8) -> usize {
    match b {
        0x00..=0x7F => 1,
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => 0,
    }
}

impl VtKeys {
    pub fn new() -> Self {
        Self::default()
    }

    /// Prida bajty, vrati hotove klavesy.
    pub fn feed(&mut self, data: &[u8]) -> Vec<Key> {
        self.pending.extend_from_slice(data);
        let mut out = Vec::new();
        let mut i = 0;
        let buf = core::mem::take(&mut self.pending);
        while i < buf.len() {
            let b = buf[i];
            if b == 0x1B {
                let rest = &buf[i + 1..];
                match rest.first() {
                    // Samotne Esc na konci: pocka na dalsi data / flush.
                    None => break,
                    Some(b'[') => {
                        match rest[1..].iter().position(|&c| csi_end(c)) {
                            Some(end) => {
                                out.extend(csi(&rest[1..=end + 1]));
                                i += 2 + end + 1;
                            }
                            None if rest.len() < MAX_SEQ => break,
                            // Nesmysl: zahodit Esc a [.
                            None => i += 2,
                        }
                    }
                    Some(b'O') => match rest.get(1) {
                        Some(&c) => {
                            out.extend(ss3(c));
                            i += 3;
                        }
                        None => break,
                    },
                    // Alt+b / Alt+f (emacs): slovo zpet / vpred = jako
                    // Alt+sipky.
                    Some(b'b') => {
                        out.push(Key::AltLeft);
                        i += 2;
                    }
                    Some(b'f') => {
                        out.push(Key::AltRight);
                        i += 2;
                    }
                    // Esc Esc nebo Esc + jiny znak: Esc, znak se zpracuje
                    // znovu.
                    Some(_) => {
                        out.push(Key::Esc);
                        i += 1;
                    }
                }
                continue;
            }
            if b < 0x20 || b == 0x7F {
                out.extend(control(b));
                // CR LF od klienta = jeden Enter.
                i += if b == b'\r' && buf.get(i + 1) == Some(&b'\n') { 2 } else { 1 };
                continue;
            }
            let n = utf8_len(b);
            if n == 0 {
                i += 1;
                continue;
            }
            if i + n > buf.len() {
                break;
            }
            match core::str::from_utf8(&buf[i..i + n]).ok().and_then(|s| s.chars().next()) {
                Some(c) => {
                    out.push(Key::Char(c));
                    i += n;
                }
                None => i += 1,
            }
        }
        self.pending = buf[i..].to_vec();
        out
    }

    /// Pauza bez dalsich dat: drzene samotne Esc je klavesa Esc,
    /// nedokoncena sekvence se zahodi.
    pub fn flush(&mut self) -> Vec<Key> {
        let esc = self.pending == [0x1B];
        self.pending.clear();
        if esc { alloc::vec![Key::Esc] } else { Vec::new() }
    }

    /// Ceka neco na `flush`?
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
}
