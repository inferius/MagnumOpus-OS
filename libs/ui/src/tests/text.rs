//! Editace textu (`ui::text`): grafemy, slova, vyber, historie.

use crate::text::*;

/// Hranice grafemu: rozlozeny hacek, emoji s barvou pleti, rodina pres
/// ZWJ, vlajky, CR LF.
#[test]
fn grapheme_boundaries() {
    // "C" + kombinacni hacek = jeden znak pro cloveka.
    let s = "C\u{30C}aj";
    assert_eq!(boundaries(s), [0, 3, 4, 5]);
    assert_eq!(grapheme_count(s), 3);
    assert_eq!(grapheme_count("Čaj"), 3);
    assert_eq!(grapheme_count("👍🏽"), 1);
    assert_eq!(grapheme_count("👨\u{200D}👩\u{200D}👧"), 1);
    // Dve vlajky = ctyri regionalni indikatory, dva grafemy.
    assert_eq!(grapheme_count("🇨🇿🇸🇰"), 2);
    assert_eq!(grapheme_count("a\r\nb"), 3);
    let s = "x🇨🇿🇸🇰";
    assert_eq!(prev_boundary(s, s.len()), 9);
    assert_eq!(prev_boundary(s, 9), 1);
    assert_eq!(prev_boundary("C\u{30C}", 3), 0);
}

/// Slova: Ctrl+sipky preskakuji slovo a mezery za nim, diakritika je
/// soucast slova, interpunkce je vlastni usek.
#[test]
fn word_motion_and_selection() {
    let s = "Žluťoučký kůň, úpěl";
    assert_eq!(next_word(s, 0), "Žluťoučký ".len());
    assert_eq!(next_word(s, "Žluťoučký ".len()), "Žluťoučký kůň".len());
    assert_eq!(prev_word(s, s.len()), "Žluťoučký kůň, ".len());
    assert_eq!(prev_word(s, "Žluťoučký kůň, ".len()), "Žluťoučký kůň".len());
    assert_eq!(prev_word(s, "Žluťoučký ".len()), 0);
    assert_eq!(word_at(s, 2), (0, "Žluťoučký".len()));
    let k = "Žluťoučký ".len();
    assert_eq!(word_at(s, k + 1), (k, k + "kůň".len()));
    assert_eq!(word_at("", 0), (0, 0));
    assert_eq!(line_at("ab\ncd\nef", 4), (3, 5));
}

#[test]
fn edit_insert_delete_selection() {
    let mut e = TextEdit::new("Čaj", false);
    assert_eq!(e.cursor(), "Čaj".len());
    e.move_to(Motion::Left, false);
    e.delete(true, false);
    assert_eq!(e.text(), "Čj");
    e.move_to(Motion::Start, false);
    e.insert("ž");
    assert_eq!(e.text(), "žČj");
    // Shift+End vybere zbytek, psani ho nahradi.
    e.move_to(Motion::End, true);
    assert_eq!(e.selected_text(), "Čj");
    e.insert("x");
    assert_eq!(e.text(), "žx");
    // Sipka vlevo s vyberem skoci na jeho zacatek.
    e.select_all();
    e.move_to(Motion::Left, false);
    assert_eq!((e.cursor(), e.selection()), (0, None));
    // Kombinacni znak se maze s pismenem.
    let mut e = TextEdit::new("C\u{30C}", false);
    e.delete(true, false);
    assert_eq!(e.text(), "");
    // Ctrl+Backspace smaze slovo.
    let mut e = TextEdit::new("ahoj světe", false);
    e.delete(true, true);
    assert_eq!(e.text(), "ahoj ");
    // Jednoradkove pole: vlozeny konec radku je mezera.
    let mut e = TextEdit::new("", false);
    e.insert("a\nb\r\nc\td");
    assert_eq!(e.text(), "a b c d");
    let mut e = TextEdit::new("", true);
    e.insert("a\nb");
    assert_eq!(e.text(), "a\nb");
    e.move_to(Motion::LineStart, false);
    assert_eq!(e.cursor(), 2);
}

/// Zpet a znovu: psani slova je jeden krok, mezera a jina akce kroky
/// oddeli, nova uprava zahodi "znovu".
#[test]
fn undo_redo_groups_typing() {
    let mut e = TextEdit::new("", false);
    for c in "ahoj světe".chars() {
        e.insert(c.encode_utf8(&mut [0; 4]));
    }
    assert!(e.undo());
    assert_eq!(e.text(), "ahoj ");
    assert!(e.undo());
    assert_eq!(e.text(), "ahoj");
    assert!(e.undo());
    assert_eq!(e.text(), "");
    assert!(!e.undo());
    assert!(e.redo());
    assert_eq!(e.text(), "ahoj");
    e.insert("!");
    assert!(!e.can_redo());
    // Backspace po sobe = jeden krok.
    e.delete(true, false);
    e.delete(true, false);
    assert_eq!(e.text(), "aho");
    e.undo();
    assert_eq!(e.text(), "ahoj!");
}

/// Hodnotu zmenila aplikace: kurzor zustane na hranici grafemu.
#[test]
fn set_text_keeps_cursor_on_boundary() {
    let mut e = TextEdit::new("abc", false);
    e.set_cursor(3, false);
    e.set_text("Č");
    assert!(e.cursor() == 0 || e.cursor() == "Č".len());
    e.set_text("C\u{30C}x");
    e.set_cursor(1, false);
    assert_eq!(e.cursor(), 0, "uprostred grafemu -> jeho zacatek");
}
