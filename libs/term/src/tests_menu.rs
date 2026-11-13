//! Nabidka doplnovani kreslena oknem (`ui::menu`): typy polozek, zahlavi,
//! vyber / prijeti / zavreni zvenku, kotva a snimek bez radku nabidky.

use alloc::string::String;
use alloc::vec;

use crate::proto::{ClientMsg, ServerMsg};
use crate::theme::Theme;
use crate::ui::{App, Key, menu};
use shlang::complete::CandKind;

fn app() -> App {
    let mut a = App::new(Theme::dark(), 80, 30);
    a.on_msg(ServerMsg::Attached { session: 1, cwd: "/".into(), blocks: 0 });
    a
}

/// "open d" + Tab a odpoved se dvema polozkami (soubor, slozka).
fn with_menu(a: &mut App) {
    for c in "open d".chars() {
        a.key(Key::Char(c));
    }
    a.key(Key::Tab);
    let seq = a.take_out().iter().rev().find_map(|m| match m {
        ClientMsg::Complete { seq, .. } => Some(*seq),
        _ => None,
    });
    a.on_msg(ServerMsg::Completions {
        seq: seq.unwrap(),
        start: 5,
        items: vec![("data.json".into(), CandKind::File as u8, String::new()), ("docs/".into(), CandKind::Dir as u8, String::new())],
    });
    assert!(a.menu.is_some());
}

#[test]
fn kinds_and_header() {
    assert_eq!(menu::kind_info(CandKind::Dir as u8), ("folder", "složka"));
    assert_eq!(menu::kind_info(CandKind::Command as u8), ("terminal", "příkaz"));
    assert_eq!(menu::header([CandKind::File as u8, CandKind::Dir as u8]), "Cesty");
    assert_eq!(menu::header([CandKind::Command as u8; 3]), "Příkazy");
    assert_eq!(menu::header([CandKind::Command as u8, CandKind::Var as u8]), "Návrhy");
    assert_eq!(menu::header([]), "Návrhy");
}

#[test]
fn select_accept_close_from_window() {
    let mut a = app();
    with_menu(&mut a);
    a.menu_select(1);
    assert_eq!(a.menu.as_ref().unwrap().sel, 1);
    // Mimo rozsah se vyber nezmeni.
    a.menu_select(9);
    assert_eq!(a.menu.as_ref().unwrap().sel, 1);
    a.menu_accept(1);
    assert_eq!(a.editor.text, "open docs/");
    assert!(a.menu.is_none());
    let mut a = app();
    with_menu(&mut a);
    a.menu_close();
    assert!(a.menu.is_none());
    assert_eq!(a.editor.text, "open d");
}

#[test]
fn anchor_column_is_word_start() {
    let mut a = app();
    with_menu(&mut a);
    // Vyzva "/ > " (bez ikony): slovo zacina za "open ".
    let base = a.input_col(0);
    assert_eq!(a.menu_col(), Some(base + 5));
    a.icon_room = true;
    assert_eq!(a.input_col(0), base + 2);
    a.menu_close();
    assert_eq!(a.menu_col(), None);
}

#[test]
fn float_menu_has_no_lines() {
    let plain = |a: &App| a.draw().iter().map(|l| l.plain()).collect::<alloc::vec::Vec<_>>().join("\n");
    let mut a = app();
    with_menu(&mut a);
    assert!(plain(&a).contains("docs/"));
    a.float_menu = true;
    assert!(!plain(&a).contains("docs/"));
    assert!(a.menu.is_some());
}
