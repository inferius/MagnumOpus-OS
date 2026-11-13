//! Mys a schranka v okne: vlozeni do radku vstupu, klik do radku vstupu.

use crate::proto::ServerMsg;
use crate::theme::Theme;
use crate::ui::{App, Key};

fn app() -> App {
    let mut a = App::new(Theme::dark(), 80, 30);
    a.on_msg(ServerMsg::Attached { session: 1, cwd: "/".into(), blocks: 0 });
    a
}

#[test]
fn paste_inserts_at_cursor_without_running() {
    let mut a = app();
    for c in "ls  x".chars() {
        a.key(Key::Char(c));
    }
    a.key(Key::Left);
    a.paste("-a\r\n");
    assert_eq!(a.editor.text, "ls  -ax");
    // Vic radku: zustanou (Enter je spusti jako jeden skript), nic se samo
    // nespusti; tabulator = mezera, ridici znaky pryc.
    let mut a = app();
    a.paste("echo 1\necho\t2\u{7}\n\n");
    assert_eq!(a.editor.text, "echo 1\necho 2");
    assert!(a.take_out().iter().all(|m| !matches!(m, crate::proto::ClientMsg::Run { .. })));
    // Radek vstupu ukaze konec radku znakem, ne zalomenim obrazovky.
    let (_, lines) = a.frame_lines();
    let input: String = lines
        .iter()
        .map(|l| (0..l.parts_len()).map(|i| l.part_text(i)).collect::<String>())
        .find(|t| t.contains("echo 1"))
        .unwrap();
    assert!(input.contains("echo 1\u{21B5}echo 2"), "{input:?}");
}

#[test]
fn click_input_moves_cursor() {
    let mut a = app();
    for c in "echo ahoj".chars() {
        a.key(Key::Char(c));
    }
    // Vyzva "/ > " = 4 sloupce (bez mista pro ikonu).
    a.click_input(4 + 5);
    assert_eq!(a.editor.cursor, 5);
    a.click_input(0);
    assert_eq!(a.editor.cursor, 0);
    a.click_input(200);
    assert_eq!(a.editor.cursor, "echo ahoj".len());
}
