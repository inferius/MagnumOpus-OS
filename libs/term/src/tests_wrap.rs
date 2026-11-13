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

/// Role radku pro okno GUI: hlavicka bloku s casem, telo, chyba, zahlavi
/// tabulky, radek vstupu s pilulkou, napoveda jako stitky.
#[test]
fn lines_carry_roles_for_gui() {
    use crate::ui::draw::{Mark, Role};
    use shlang::value::Value;
    let mut a = App::new(crate::theme::Theme::dark(), 80, 30);
    a.on_msg(ServerMsg::Attached { session: 7, cwd: "/home/eva".into(), blocks: 0 });
    a.on_msg(ServerMsg::BlockStart { session: 7, block: 1, line: "ls".into() });
    let row = |n: &str| Value::record(&[("name", Value::str(n)), ("size", Value::Size(5))]);
    a.on_msg(ServerMsg::Item { session: 7, block: 1, item: Item::Value(Value::List(vec![row("a"), row("b")])) });
    a.on_msg(ServerMsg::BlockEnd { session: 7, block: 1, ok: true, duration: 3_000_000 });
    a.on_msg(ServerMsg::BlockStart { session: 7, block: 2, line: "open x".into() });
    a.on_msg(ServerMsg::Item { session: 7, block: 2, item: Item::Error { code: "NOT_FOUND".into(), message: "x".into(), report: "chyba\n  dal".into() } });
    a.on_msg(ServerMsg::BlockEnd { session: 7, block: 2, ok: false, duration: 1_000_000 });
    let (_, lines) = a.frame_lines();
    let roles: Vec<&Role> = lines.iter().map(|l| &l.role).collect();
    assert!(roles.iter().any(|r| matches!(r, Role::Head { mark: Mark::Ok, .. })));
    assert!(roles.iter().any(|r| matches!(r, Role::Head { mark: Mark::Err, .. })));
    assert!(roles.iter().any(|r| matches!(r, Role::Body { table_head: true, .. })));
    assert_eq!(roles.iter().filter(|r| matches!(r, Role::Body { error: true, .. })).count(), 2, "dva radky hlaseni");
    assert!(roles.iter().any(|r| matches!(r, Role::Input { prefix: 2, cwd } if cwd == "~")));
    let hints = roles.iter().find_map(|r| match r {
        Role::Hints(h) => Some(h.clone()),
        _ => None,
    });
    let hints = hints.expect("napoveda klaves");
    assert_eq!(hints[0], ("Tab".into(), "doplnit".into()));
    assert!(hints.iter().any(|(k, l)| k == "\u{2191}" && l == "historie"));
    // Cas bloku je samostatny usek (okno ho kresli vpravo).
    let head = lines.iter().find(|l| matches!(l.role, Role::Head { mark: Mark::Ok, .. })).unwrap();
    let Role::Head { meta, .. } = head.role else { unreachable!() };
    assert!(head.part_text(meta).contains("ms"), "{:?}", head.part_text(meta));
}

#[test]
fn icon_room_reserves_cells_before_folder() {
    use crate::ui::draw::Role;
    let mut a = App::new(crate::theme::Theme::dark(), 80, 30);
    a.on_msg(ServerMsg::Attached { session: 7, cwd: "/home/eva".into(), blocks: 0 });
    a.icon_room = true;
    let (_, lines) = a.frame_lines();
    let input = lines.iter().find(|l| matches!(l.role, Role::Input { .. })).expect("radek vstupu");
    let Role::Input { prefix, .. } = input.role else { unreachable!() };
    assert_eq!(prefix, 3);
    // Dve prazdne bunky pro ikonu, pak slozka a oddelovac.
    assert_eq!(input.part_text(0), "  ");
    assert_eq!(input.part_cols()[prefix - 1].1, 2 + "~".len() + 3);
}

#[test]
fn notation_switch_formats_line_once() {
    use crate::proto::ClientMsg;
    use crate::ui::Key;
    let mut a = App::new(crate::theme::Theme::dark(), 80, 30);
    a.on_msg(ServerMsg::Attached { session: 7, cwd: "/".into(), blocks: 0 });
    a.take_out();
    // Prazdny radek: jen zmena zapisu, nic se neposila.
    a.set_notation(true);
    assert!(a.notation_c && a.take_out().is_empty());
    a.key(Key::Ctrl('f'));
    assert!(!a.notation_c);
    for c in "ls -r".chars() {
        a.key(Key::Char(c));
    }
    a.take_out();
    a.set_notation(true);
    let out = a.take_out();
    assert!(matches!(&out[..], [ClientMsg::Format { line, style: 1, .. }] if line == "ls -r"), "{out:?}");
    // Stejny zapis znovu = nic.
    a.set_notation(true);
    assert!(a.take_out().iter().all(|m| !matches!(m, ClientMsg::Format { .. })));
}

#[test]
fn table_explains_missing_values() {
    use shlang::value::{Value, Why};
    let mut a = App::new(crate::theme::Theme::dark(), 100, 30);
    a.on_msg(ServerMsg::Attached { session: 7, cwd: "/".into(), blocks: 0 });
    a.on_msg(ServerMsg::BlockStart { session: 7, block: 1, line: "ls".into() });
    let row = |n: &str, size: Value| Value::record(&[("name", Value::str(n)), ("size", size)]);
    let rows = vec![row("prazdna", Value::Size(0)), row("cizi", Value::Missing(Why::Hidden))];
    a.on_msg(ServerMsg::Item { session: 7, block: 1, item: Item::Value(Value::List(rows)) });
    a.on_msg(ServerMsg::BlockEnd { session: 7, block: 1, ok: true, duration: 1 });
    let (_, lines) = a.frame_lines();
    let text: Vec<String> = lines.iter().map(|l| (0..l.parts_len()).map(|i| l.part_text(i)).collect()).collect();
    assert!(text.iter().any(|t| t.contains("cizi") && t.contains('\u{2014}')), "{text:?}");
    assert!(text.iter().any(|t| t.contains("prazdna") && t.contains("0 B")), "{text:?}");
    // Vysvetlivka jen k duvodu, ktery v tabulce je.
    assert!(text.iter().any(|t| t.contains("skryté (chybí právo)")), "{text:?}");
    assert!(!text.iter().any(|t| t.contains("nedostupné")), "{text:?}");
}
