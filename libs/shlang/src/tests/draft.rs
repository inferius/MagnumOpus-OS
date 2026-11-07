//! Testy konceptu radku v klonech FS: datasety, konflikty, uklid klonu.

use super::host::{TestHost, interp};
use crate::error::code;
use crate::host::Fs;

#[test]
fn nested_dataset_gets_its_own_clone_and_undo() {
    let mut h = TestHost::new();
    h.fs.add_dataset("/home/docs");
    let mut it = interp();
    let o = it.run_line(&mut h, "echo x > docs/n.txt && echo y > a.txt");
    o.result.unwrap();
    assert_eq!(h.fs.clones_made(), 2, "klon / i klon /home/docs");
    it.flush(&mut h);
    assert_eq!(h.fs.asides_open(), 0);
    assert_eq!(h.fs.get("/home/docs/n.txt"), Some(&b"x"[..]));
    assert_eq!(h.fs.get("/home/a.txt"), Some(&b"y"[..]));
    assert_eq!(o.diff.len(), 2);
    it.run_line(&mut h, "undo").result.unwrap();
    assert!(!h.fs.exists("/home/docs/n.txt"));
    assert_eq!(h.fs.get("/home/a.txt"), Some(&b"alfa\n"[..]));
    assert_eq!(h.fs.views_open(), 0, "undo uvolni pohledy");
}

#[test]
fn move_between_datasets_is_copy_and_remove() {
    let mut h = TestHost::new();
    h.fs.add_dataset("/home/docs");
    let mut it = interp();
    it.run_line(&mut h, "mv a.txt docs/a.txt").result.unwrap();
    assert!(!h.fs.exists("/home/a.txt"));
    assert_eq!(h.fs.get("/home/docs/a.txt"), Some(&b"alfa\n"[..]));
    it.run_line(&mut h, "undo").result.unwrap();
    assert!(h.fs.exists("/home/a.txt"));
    assert!(!h.fs.exists("/home/docs/a.txt"));
}

#[test]
fn concurrent_change_is_conflict_not_overwrite() {
    let mut h = TestHost::new();
    let mut it = interp();
    it.lock_threshold = 0;
    // Behem dotazu zamku jina relace zmeni a.txt.
    h.write_on_confirm = Some(("/home/a.txt".into(), b"jinak".to_vec()));
    let o = it.run_line(&mut h, "echo moje > a.txt && echo nove > c.txt");
    assert_eq!(o.result.unwrap_err().code, code::CONFLICT);
    assert_eq!(h.fs.get("/home/a.txt"), Some(&b"jinak"[..]), "cizi zmena zustala");
    assert_eq!(h.fs.get("/home/c.txt"), Some(&b"nove"[..]), "ostatni zmeny prevzaty");
    it.flush(&mut h);
    assert_eq!(h.fs.asides_open(), 0);
    // Undo vrati jen to, co se prevzalo.
    it.run_line(&mut h, "undo").result.unwrap();
    assert!(!h.fs.exists("/home/c.txt"));
    assert_eq!(h.fs.get("/home/a.txt"), Some(&b"jinak"[..]));
}

#[test]
fn rejected_or_failed_lines_leave_no_clones() {
    let mut h = TestHost::new();
    let mut it = interp();
    it.lock_threshold = 0;
    h.answer = false;
    let o = it.run_line(&mut h, "echo x > a.txt");
    assert_eq!(o.result.unwrap_err().code, code::REJECTED);
    assert_eq!(h.fs.get("/home/a.txt"), Some(&b"alfa\n"[..]));
    it.flush(&mut h);
    assert_eq!(h.fs.asides_open(), 0);
    // Chyba uprostred nahledu: vrstva nahledu pryc, radek se prevezme.
    h.answer = true;
    it.lock_threshold = 20;
    let o = it.run_line(&mut h, "echo a > x.txt\npreview { echo b > y.txt; error \"stop\" }");
    assert_eq!(o.result.unwrap_err().code, code::USER);
    assert!(h.fs.exists("/home/x.txt"));
    assert!(!h.fs.exists("/home/y.txt"));
    it.flush(&mut h);
    assert_eq!(h.fs.asides_open(), 0);
}

#[test]
fn writes_into_clone_boundary_fail_cleanly() {
    // Primo nad MemFs: hranice vnoreneho datasetu v klonu.
    let mut fs = crate::vfs::mem::MemFs::new();
    fs.put("/d/x.txt", b"1");
    fs.add_dataset("/d");
    let a = fs.aside_begin("/").unwrap();
    let inside = crate::vfs::path::join(&a.root, "d/x.txt");
    assert_eq!(fs.write(&inside, b"2"), Err(abi::Error::NotSupported));
    assert!(fs.aside_changes(&a).unwrap().is_empty());
    fs.aside_discard(&a).unwrap();
    assert_eq!(fs.get("/d/x.txt"), Some(&b"1"[..]));
}

// ---- review: undo, konflikty, nahled, cteni bez dotazu, odlozeny uklid ----

#[test]
fn view_sync_never_touches_nested_datasets() {
    // Srovnani celeho "/" (zalozni cesta undo programu) nesmi sahnout do
    // vnoreneho datasetu - ten pohled "/" nevidi.
    let mut fs = crate::vfs::mem::MemFs::new();
    fs.put("/home/a.txt", b"puvodni");
    fs.put("/home/docs/x.txt", b"x1");
    fs.add_dataset("/home/docs");
    let v = fs.view("/").unwrap();
    fs.write("/home/a.txt", b"zmena").unwrap();
    fs.write("/home/docs/x.txt", b"x2").unwrap();
    fs.write("/home/docs/novy.txt", b"n").unwrap();
    crate::vfs::undo::sync_from_view(&mut fs, &v, "/").unwrap();
    assert_eq!(fs.get("/home/a.txt"), Some(&b"puvodni"[..]));
    assert_eq!(fs.get("/home/docs/x.txt"), Some(&b"x2"[..]), "vnoreny dataset beze zmeny");
    assert!(fs.exists("/home/docs/novy.txt"), "nic ve vnorenem datasetu se nesmaze");
}

#[test]
fn truncated_conflict_list_means_no_undo() {
    let mut h = TestHost::new();
    h.fs.conflict_limit = 0;
    let mut it = interp();
    it.lock_threshold = 0;
    h.write_on_confirm = Some(("/home/a.txt".into(), b"jinak".to_vec()));
    let before = it.undo.entries.len();
    let o = it.run_line(&mut h, "echo moje > a.txt && echo nove > c.txt");
    let e = o.result.unwrap_err();
    assert_eq!(e.code, code::CONFLICT);
    assert_eq!(it.undo.entries.len(), before, "nejisty vysledek prevzeti nejde vratit");
    assert_eq!(h.fs.get("/home/a.txt"), Some(&b"jinak"[..]));
}

#[test]
fn preview_program_gets_clone_of_nested_home() {
    let mut h = TestHost::new();
    h.fs.add_dataset("/home");
    let mut it = interp();
    it.run_line(&mut h, "preview -y { tool }").result.unwrap();
    let aside = &h.execs[0].2.aside;
    assert!(aside.iter().any(|(ds, _)| ds == "/home"), "domov jako vnoreny dataset ma vlastni klon: {aside:?}");
    assert_eq!(h.fs.get("/home/tool.out"), Some(&b"z programu"[..]));
}

#[test]
fn reads_in_a_writing_line_do_not_ask_for_datasets() {
    let mut h = TestHost::new();
    h.fs.add_dataset("/home/docs");
    let mut it = interp();
    let o = it.run_line(&mut h, "echo x > a.txt\nls -r / | count\nopen docs/small.pdf");
    o.result.unwrap();
    // Zapis: par dotazu; cteni stromu (desitky cest) zadny navic.
    assert!(h.fs.dataset_queries < 6, "dotazu na dataset: {}", h.fs.dataset_queries);
}

#[test]
fn discarding_clones_waits_for_idle() {
    let mut h = TestHost::new();
    let mut it = interp();
    it.run_line(&mut h, "echo x > a.txt").result.unwrap();
    assert_eq!(h.fs.get("/home/a.txt"), Some(&b"x"[..]), "zmena je prevzata hned");
    assert_eq!(h.fs.asides_open(), 1, "klon se zahodi az ve volne chvili");
    assert!(it.has_flush());
    it.flush(&mut h);
    assert_eq!(h.fs.asides_open(), 0);
    assert!(!it.has_flush());
}
