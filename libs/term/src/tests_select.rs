//! Vyber textu v mrizce (okno GUI): tah, dvojklik, trojklik, posun
//! s vystupem.

use crate::screen::Screen;
use crate::select::{Mode, Selector};

/// Slovo = souvisle pismena a cislice (okno dodava `ui::text::word_at`).
fn word_at(s: &str, i: usize) -> (usize, usize) {
    let w = |c: char| c.is_alphanumeric();
    let a = s[..i].char_indices().rev().find(|&(_, c)| !w(c)).map_or(0, |(k, c)| k + c.len_utf8());
    let b = s[i..].char_indices().find(|&(_, c)| !w(c)).map_or(s.len(), |(k, _)| i + k);
    (a, b)
}

fn screen(rows: &[&str]) -> Screen {
    let mut s = Screen::new(20, 5);
    s.write(&rows.join("\r\n"));
    s
}

#[test]
fn drag_selects_chars_across_rows() {
    let sc = screen(&["ahoj svete", "druhy radek"]);
    let mut s = Selector::new(Some(word_at), 400);
    s.press(&sc, (0, 5), 0, false);
    s.drag(&sc, (1, 4));
    s.release();
    assert_eq!(s.text(&sc), "svete\ndruhy");
    // Klik bez tahu nic nevybere.
    s.press(&sc, (0, 1), 10_000, false);
    s.release();
    assert!(s.sel.is_none());
}

#[test]
fn double_and_triple_click() {
    let sc = screen(&["ls docs/zprava.txt"]);
    let mut s = Selector::new(Some(word_at), 400);
    s.press(&sc, (0, 9), 0, false);
    s.release();
    s.press(&sc, (0, 9), 100, false);
    s.release();
    assert_eq!(s.sel.as_ref().unwrap().mode, Mode::Word);
    assert_eq!(s.text(&sc), "zprava");
    s.press(&sc, (0, 9), 200, false);
    s.release();
    assert_eq!(s.sel.as_ref().unwrap().mode, Mode::Line);
    assert_eq!(s.text(&sc), "ls docs/zprava.txt");
    // Pozdni klik = novy jednoduchy klik.
    s.press(&sc, (0, 9), 10_000, false);
    assert_eq!(s.sel.as_ref().unwrap().mode, Mode::Char);
}

#[test]
fn selection_follows_scrolled_output_or_clears() {
    let mut sc = screen(&["a", "b", "vyber tady", "d"]);
    let mut s = Selector::new(Some(word_at), 400);
    s.press(&sc, (2, 0), 0, false);
    s.drag(&sc, (2, 4));
    s.release();
    assert_eq!(s.text(&sc), "vyber");
    // Novy vystup posune obsah o radek nahoru: vyber jde s textem.
    sc.write("\x1b[2J\x1b[Hb\r\nvyber tady\r\nd\r\ne");
    assert!(s.follow(&sc));
    assert_eq!(s.text(&sc), "vyber");
    assert_eq!(s.sel.as_ref().unwrap().anchor.0, 1);
    // Text zmizel: vyber se zrusi.
    sc.write("\x1b[2J\x1b[Hjine\r\nnic");
    assert!(s.follow(&sc));
    assert!(s.sel.is_none());
}
