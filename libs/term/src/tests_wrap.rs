//! Styl vystupu pres konce radku a zalomeni: barva viceradkoveho hlaseni
//! (chyba) plati na vsech jeho radcich i zalomenych pokracovanich.

use crate::proto::{Item, ServerMsg};
use crate::ui::App;

#[test]
fn multiline_error_keeps_color_on_every_line_and_wrap() {
    let mut a = App::new(crate::theme::Theme::dark(), 40, 24);
    a.on_msg(ServerMsg::Attached { session: 7, cwd: "/".into(), blocks: 0 });
    a.on_msg(ServerMsg::BlockStart { session: 7, block: 1, line: "open x".into() });
    // Zalamuje se az druhy radek (barvu nastavuje jen zacatek hlaseni).
    let report = "chyba NOT_FOUND: x\n  na radku 1: soubor nejde precist protoze neexistuje vubec nikde\n    open x";
    a.on_msg(ServerMsg::Item { session: 7, block: 1, item: Item::Error { code: "NOT_FOUND".into(), message: "x".into(), report: report.into() } });
    let mut s = crate::screen::Screen::new(40, 24);
    s.write(&a.frame());
    let err = a.theme.ui.err;
    let err = (err.0, err.1, err.2);
    // Kazdy radek s textem hlaseni ma barvu chyby u prvniho znaku textu.
    let mut rows = 0;
    for y in 0..24 {
        let line = s.line(y);
        if ["chyba", "nejde", "vubec", "nikde", "na radku", "open x"].iter().any(|w| line.contains(w)) && !line.contains("| open") {
            let x = line.find(|c: char| !c.is_whitespace()).unwrap();
            let x = line[..x].chars().count();
            assert_eq!(s.cell(x, y).fg, err, "radek {y} '{line}' bez barvy chyby");
            rows += 1;
        }
    }
    assert!(rows >= 4, "hlaseni ma byt zalomene na vic radku: {rows}");
}

/// Alt+sipky pri prazdnem radku: zpet / vpred v historii slozek.
#[test]
fn alt_arrows_navigate_folder_history() {
    use crate::proto::ClientMsg;
    use crate::ui::Key;
    let mut a = App::new(crate::theme::Theme::dark(), 80, 24);
    a.on_msg(ServerMsg::Attached { session: 7, cwd: "/".into(), blocks: 0 });
    a.take_out();
    a.key(Key::AltLeft);
    assert!(a.take_out().iter().any(|m| matches!(m, ClientMsg::Run { line, .. } if line == "cd -")));
    a.key(Key::AltRight);
    assert!(a.take_out().iter().any(|m| matches!(m, ClientMsg::Run { line, .. } if line == "cd +")));
    // S rozepsanym radkem nic nespousti.
    a.key(Key::Char('x'));
    a.take_out();
    a.key(Key::AltLeft);
    assert!(!a.take_out().iter().any(|m| matches!(m, ClientMsg::Run { .. })));
}
