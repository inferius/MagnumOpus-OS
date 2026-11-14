//! Zalamovani stitku: jadro `wrap` (sirka = pocet znaku) a rozvrzeni
//! (vyska stitku podle poctu radku, zuzeni v radku, `lines`).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gfx::Canvas;
use guiproto::a11y::Role;

use crate::wrap::wrap;
use crate::*;

/// Sirka = pocet znaku (bez pisma).
fn chars(s: &str) -> u32 {
    s.chars().count() as u32
}

fn w(text: &str, width: u32, max: usize) -> Vec<String> {
    wrap(text, width, max, &chars)
}

#[test]
fn words_break_on_spaces() {
    assert_eq!(w("Smíte číst a měnit", 100, 0), ["Smíte číst a měnit"]);
    assert_eq!(w("Smíte číst a měnit", 10, 0), ["Smíte číst", "a měnit"]);
    assert_eq!(w("a bb ccc dddd", 4, 0), ["a bb", "ccc", "dddd"]);
    // Kazdy radek se vejde.
    for l in w("jeden dva tri ctyri pet sest sedm osm devet deset", 12, 0) {
        assert!(chars(&l) <= 12, "{l}");
    }
}

#[test]
fn newline_forces_break() {
    assert_eq!(w("prvni\ndruhy radek", 100, 0), ["prvni", "druhy radek"]);
    assert_eq!(w("a\n\nb", 100, 0), ["a", "", "b"]);
    assert_eq!(w("", 10, 0), [""]);
}

/// Dlouhe slovo bez mezer (cesta): za oddelovacem, jinak kde se vejde.
#[test]
fn long_word_is_split() {
    let p = "/home/uzivatel/dokumenty/projekt/zprava.txt";
    let ls = w(p, 16, 0);
    assert_eq!(ls.concat(), p, "nic se neztrati");
    assert!(ls.iter().all(|l| chars(l) <= 16), "{ls:?}");
    assert_eq!(ls[0], "/home/uzivatel/");
    // Bez oddelovacu: po znacich.
    assert_eq!(w("aaaaaaaaaa", 4, 0), ["aaaa", "aaaa", "aa"]);
    // Ani jeden znak se nevejde: aspon jeden na radek (zadne zacykleni).
    assert_eq!(w("abc", 0, 0), ["a", "b", "c"]);
    // Slovo za kratkym zacatkem radku jde na novy radek.
    assert_eq!(w("Cesta: /a/bbbbbbbb/cc", 8, 0), ["Cesta:", "/a/", "bbbbbbbb", "/cc"]);
}

/// `max` radku: posledni nese zbytek textu (kreslic ho zkrati s `...`).
#[test]
fn max_lines_keeps_rest_on_last() {
    assert_eq!(w("a bb ccc dddd", 4, 2), ["a bb", "ccc dddd"]);
    assert_eq!(w("a bb ccc dddd", 4, 3), ["a bb", "ccc", "dddd"]);
    assert_eq!(w("prvni\ndruhy\ntreti", 100, 2), ["prvni", "druhy treti"]);
    assert_eq!(w("a bb", 100, 1), ["a bb"]);
}

const TEXT: &str = "Smíte: číst, měnit a mazat soubory v této složce i ve všech podsložkách, které do ní patří";

fn ui(v: Node<()>, w: u32) -> Ui<()> {
    let mut u = Ui::new(crate::Look::ui_fonts(), w, 400);
    u.rebuild("w".into(), v);
    u
}

fn label_bounds(u: &Ui<()>, name: &str) -> Rect {
    u.a11y().into_iter().find(|n| n.role == Role::Label && n.name == name).unwrap().bounds
}

/// Ve sloupci: stitek dostane sirku sloupce, vyska = pocet radku; prvek pod
/// nim se posune (nic se neprekryva).
#[test]
fn label_wraps_in_column() {
    let u = ui(column([label(TEXT), button("OK")]), 220);
    let look = &u.look;
    let s = look.body();
    let b = label_bounds(&u, TEXT);
    assert!(look.width(TEXT, s) > b.w, "text je sirsi nez okno");
    let n = crate::layout::label_lines(look, TEXT, s, b.w, 0).len() as u32;
    assert!(n >= 3, "{n} radku");
    assert_eq!(b.h, n * s.line);
    let ok = u.a11y().into_iter().find(|n| n.role == Role::Button).unwrap().bounds;
    assert!(ok.y >= b.bottom() as i32, "tlacitko pod stitkem");
    // Kratky stitek zustane na jednom radku a v prirozene sirce.
    let u = ui(column([label("Krátký")]), 220);
    assert_eq!(label_bounds(&u, "Krátký").h, s.line);
}

/// V radku se zalamovany stitek zuzi, aby se vesly ostatni prvky.
#[test]
fn label_shrinks_in_row() {
    let u = ui(row([label(TEXT), button("Otevřít")]), 300);
    let b = label_bounds(&u, TEXT);
    let ok = u.a11y().into_iter().find(|n| n.role == Role::Button).unwrap().bounds;
    assert!(ok.right() <= 300, "tlacitko v okne: {ok:?}");
    assert!(b.right() as i32 <= ok.x, "stitek pred tlacitkem");
    assert!(b.h > u.look.body().line, "stitek je zalomeny");
}

/// `lines(2)`: nejvys dva radky; `lines(1)`: jeden radek bez zalamovani.
#[test]
fn label_max_lines() {
    let line = ui(column([label("x")]), 200).look.body().line;
    let u = ui(column([label(TEXT).lines(2)]), 200);
    assert_eq!(label_bounds(&u, TEXT).h, 2 * line);
    let u = ui(column([label(TEXT).lines(1)]), 200);
    assert_eq!(label_bounds(&u, TEXT).h, line);
}

/// Kresleni: zalomeny stitek ma inkoust na kazdem radku, nic za pravym
/// okrajem obdelniku.
#[test]
fn wrapped_label_draws_every_line() {
    let (w, h) = (220u32, 400u32);
    let mut u = ui(column([label(TEXT), label("konec")]), w);
    let mut px = vec![0u32; (w * h) as usize];
    let mut c = Canvas::new(&mut px, w, h, w).unwrap();
    u.draw(&mut c, 0);
    let bg = u.look.theme.color.surface;
    let b = label_bounds(&u, TEXT);
    let line = u.look.body().line as i32;
    let rows = b.h as i32 / line;
    for k in 0..rows {
        let ink = (b.y + k * line..b.y + (k + 1) * line).any(|y| (b.x..b.right() as i32).any(|x| px[(y as u32 * w + x as u32) as usize] != bg));
        assert!(ink, "radek {k} bez textu");
    }
    for y in b.y..b.bottom() as i32 {
        for x in b.right() as i32..w as i32 {
            assert_eq!(px[(y as u32 * w + x as u32) as usize], bg, "inkoust za okrajem na {x},{y}");
        }
    }
}
