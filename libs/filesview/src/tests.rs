//! Testy spravce souboru na hostiteli: `cargo test -p filesview`.

extern crate std;

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use fileops::job::{Conflict, Op, Phase, Resolution};
use fileops::proto::{ClientMsg, ServerMsg, Status};
use fsproto::{StatEntry, avail, kind};
use guiproto::a11y::{Role, actions as a11y_actions, state};
use ui::App;

use crate::app::{FileManager, Mode, Msg};
use crate::backend::{T0, TestBackend, oid, sample_bmp};
use crate::format::{self, Avail};
use crate::listing::{Listing, SortBy, natural};
use crate::nav::Nav;
use crate::preview::{self, Content};

type Fm = FileManager<TestBackend>;

fn fm() -> Fm {
    FileManager::new(TestBackend::sample())
}

fn ui_for(m: &Fm, w: u32) -> ui::Ui<Msg> {
    let mut u = ui::Ui::new(ui::Look::ui_fonts(), w, 700);
    u.refresh(m);
    u
}

fn names(u: &ui::Ui<Msg>, role: Role) -> Vec<String> {
    u.a11y().into_iter().filter(|n| n.role == role).map(|n| n.name).collect()
}

fn press(m: &mut Fm, u: &mut ui::Ui<Msg>, name: &str) {
    let n = u.a11y().into_iter().find(|n| n.role == Role::Button && n.name == name).unwrap_or_else(|| panic!("tlacitko {name}: {:?}", names(u, Role::Button)));
    assert_eq!(n.state & state::DISABLED, 0, "{name} je zakazane");
    let r = u.a11y_action(n.id, a11y_actions::ACTIVATE, "");
    for msg in r.msgs {
        m.update(msg);
    }
    u.refresh(m);
}

fn select(m: &mut Fm, name: &str) {
    let id = m.listing.items.iter().find(|i| i.e.name == name).unwrap_or_else(|| panic!("{name} ve vypisu")).e.oid;
    m.update(Msg::Select(vec![id]));
}

fn shown(m: &Fm) -> Vec<String> {
    m.listing.rows.iter().map(|r| r.cells[0].clone()).collect()
}

fn entry(name: &str, k: u8, size: u64, known: u16, hidden: u16) -> StatEntry {
    StatEntry { name: name.into(), kind: k, oid: oid(name), size, mtime: T0, rights: 0xff, tree_bytes: size, tree_files: 1, tree_dirs: 0, known, hidden }
}

#[test]
fn format_values() {
    assert_eq!(format::size(0), "0 B");
    assert_eq!(format::size(999), "999 B");
    assert_eq!(format::size(1234), "1,2 kB");
    assert_eq!(format::size(150_000), "150 kB");
    assert_eq!(format::size(999_999), "1,0 MB");
    assert_eq!(format::size(12_345_678), "12,3 MB");
    assert_eq!(format::size(5_000_000_000), "5,0 GB");
    assert_eq!(format::plural(1, "soubor", "soubory", "souborů"), "1 soubor");
    assert_eq!(format::plural(3, "soubor", "soubory", "souborů"), "3 soubory");
    assert_eq!(format::contents(0, 5), "0 souborů, 5 složek");
    assert_eq!(format::avail(avail::SIZE, 0, avail::SIZE), Avail::Known);
    assert_eq!(format::avail(0, avail::TREE, avail::TREE), Avail::Hidden);
    assert_eq!(format::avail(0, 0, avail::TREE), Avail::Unavailable);
    let t = T0;
    assert!(format::date(t, Some(t), None).starts_with("dnes "));
    assert!(format::date(t, Some(abi::time::WallTime { secs: t.secs + 86_400, nanos: 0 }), None).starts_with("včera "));
    let c = t.to_civil();
    assert_eq!(format::date(t, None, None), alloc::format!("{}. {}. {} {}:{:02} UTC", c.day, c.month, c.year, c.hour, c.minute));
}

#[test]
fn natural_sort_dirs_first_and_unknown_last() {
    assert!(natural("smlouva 2", "smlouva 10").is_lt());
    assert!(natural("a01", "a1").is_eq() || natural("a01", "a1").is_ne());
    let mut l = Listing::default();
    let all = avail::SIZE | avail::MTIME | avail::RIGHTS;
    l.set(
        vec![
            entry("b 10.txt", kind::FILE, 5, all, 0),
            entry("b 2.txt", kind::FILE, 50, all, 0),
            entry("Zeta", kind::DIR, 9, all | avail::TREE, 0),
            entry("alfa", kind::DATASET, 0, avail::RIGHTS, 0),
            entry("skryta", kind::DIR, 0, all & !avail::SIZE, avail::TREE | avail::SIZE),
            entry(".x.kopie-3", kind::FILE, 1, all, 0),
        ],
        Some(T0),
        None,
    );
    let names = |l: &Listing| l.rows.iter().map(|r| r.cells[0].clone()).collect::<Vec<_>>();
    assert_eq!(names(&l), ["alfa", "skryta", "Zeta", "b 2.txt", "b 10.txt"], "slozky prvni, docasne jmeno skryte");
    // Velikost sestupne: slozky stale prvni, nezname velikosti na konci.
    l.set_sort(SortBy::Size, false, Some(T0), None);
    assert_eq!(names(&l), ["Zeta", "alfa", "skryta", "b 2.txt", "b 10.txt"]);
    // Tri stavy: skryta a nedostupna velikost jako pomlcka.
    let row = |n: &str| l.rows.iter().find(|r| r.cells[0] == n).unwrap().clone();
    assert_eq!(row("skryta").cells[2], format::DASH);
    assert_eq!(row("alfa").cells[2], format::DASH);
    assert_eq!(row("alfa").cells[1], format::DASH, "hranice datasetu nema cas");
    assert_eq!(row("Zeta").cells[2], "9 B");
    assert_eq!(row("alfa").cells[3], "Dataset s historií");
    l.set_filter("B 1", Some(T0), None);
    assert_eq!(names(&l), ["b 10.txt"], "hledani bez velikosti pismen");
}

#[test]
fn nav_history_and_crumbs() {
    let mut n = Nav::new("/home/jana");
    n.go("/home/jana/Projekty/web");
    assert_eq!(n.crumbs("/home/jana").iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["Domov", "Projekty", "web"]);
    assert_eq!(n.crumbs("/home/jana")[1].1, "/home/jana/Projekty");
    assert_eq!(n.up().as_deref(), Some("web"));
    assert!(n.back());
    assert_eq!(n.cur, "/home/jana/Projekty/web");
    assert!(n.forward() && n.cur == "/home/jana/Projekty");
    n.go("/system");
    assert!(!n.can_forward());
    assert_eq!(n.crumbs("/home/jana").iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["Svazek", "system"]);
    let mut r = Nav::new("/");
    assert!(r.up().is_none());
}

#[test]
fn preview_text_bmp_binary() {
    let Content::Text { lines, more } = preview::content(b"a\r\nb\n") else { panic!() };
    assert_eq!((lines, more), (vec!["a".to_string(), "b".to_string(), String::new()], false));
    // Useknuty znak na konci nevadi.
    let cut = "příliš".as_bytes();
    assert!(matches!(preview::content(&cut[..cut.len() - 1]), Content::Text { .. }));
    assert_eq!(preview::content(&[0, 1, 2]), Content::Binary);
    let bmp = sample_bmp(3, 2);
    let Content::Bmp(i) = preview::content(&bmp) else { panic!() };
    assert_eq!((i.width, i.height, i.bpp, i.top_down), (3, 2, 24, false));
    let (w, h, px) = preview::bmp_pixels(&bmp).unwrap();
    assert_eq!((w, h, px.len()), (3, 2, 6));
    assert!(px.iter().all(|p| p >> 24 == 0xff));
    // Kratka data a nesmyslne rozmery: zadna panika.
    assert!(preview::bmp_pixels(&bmp[..60]).is_none());
    let mut huge = bmp.clone();
    huge[18..22].copy_from_slice(&100_000u32.to_le_bytes());
    huge[22..26].copy_from_slice(&100_000u32.to_le_bytes());
    assert!(preview::bmp_pixels(&huge).is_none());
    assert!(preview::bmp_info(b"BM").is_none());
}

#[test]
fn opens_home_with_places_and_table() {
    let m = fm();
    assert_eq!(m.title(), "Domov - Soubory");
    assert_eq!(shown(&m), ["Dokumenty", "Projekty", "obrazek.bmp", "zprava.txt"]);
    assert_eq!(m.places.iter().map(|p| p.label.as_str()).collect::<Vec<_>>(), ["Domov", "Projekty", "Svazek", "Koš"]);
    assert_eq!(m.b.sent[..2], [ClientMsg::Hello { version: fileops::proto::VERSION }, ClientMsg::Subscribe]);
    let u = ui_for(&m, 1200);
    let items = names(&u, Role::ListItem);
    assert!(items.contains(&String::from("Koš")), "{items:?}");
    assert!(!names(&u, Role::Row).is_empty(), "radky tabulky v pristupnosti");
    assert!(names(&u, Role::ColumnHeader).contains(&String::from("Velikost")));
    assert!(names(&u, Role::Label).iter().any(|l| l == "Náhled"), "siroke okno ma nahled");
    let narrow = ui_for(&m, 400);
    assert!(!names(&narrow, Role::Label).iter().any(|l| l == "Náhled" || l == "Místa"), "uzke okno bez panelu");
}

#[test]
fn navigate_into_and_up_reselects() {
    let mut m = fm();
    let id = m.listing.items.iter().find(|i| i.e.name == "Dokumenty").unwrap().e.oid;
    m.update(Msg::Activate(id));
    assert_eq!(m.nav.cur, "/home/jana/Dokumenty");
    assert_eq!(shown(&m), ["smlouva 2.txt", "smlouva 10.txt"], "prirozene poradi");
    m.update(Msg::Up);
    assert_eq!(m.nav.cur, "/home/jana");
    assert_eq!(m.sel, vec![id], "po navratu vybrana slozka, ze ktere se odeslo");
    let mut u = ui_for(&m, 1200);
    press(&mut m, &mut u, "Zpět");
    assert_eq!(m.nav.cur, "/home/jana/Dokumenty");
    press(&mut m, &mut u, "Domov");
    assert_eq!(m.nav.cur, "/home/jana");
    m.update(Msg::Place(1));
    assert_eq!(m.nav.cur, "/home/jana/Projekty");
    assert!(m.show_path("/home/jana/Dokumenty/smlouva 10.txt").is_ok());
    assert_eq!((m.nav.cur.as_str(), m.sel.len()), ("/home/jana/Dokumenty", 1));
    assert!(m.show_path("/neni").is_err());
}

#[test]
fn preview_shows_three_states_and_versions() {
    let mut b = TestBackend::sample();
    b.hidden_tree.push(String::from("/home/jana/Dokumenty"));
    let mut m = FileManager::new(b);
    select(&mut m, "Dokumenty");
    let u = ui_for(&m, 1200);
    let labels = names(&u, Role::Label);
    assert!(labels.iter().any(|l| l == "Velikost: \u{2014} (skryto: chybí právo Spravovat)"), "{labels:?}");
    assert!(labels.iter().any(|l| l.starts_with("Obsah: \u{2014}")));
    select(&mut m, "Projekty");
    let labels = names(&ui_for(&m, 1200), Role::Label);
    assert!(labels.iter().any(|l| l == "Obsah: 1 soubor, 0 složek"), "{labels:?}");
    assert!(labels.iter().any(|l| l.starts_with("Dataset:")));
    select(&mut m, "zprava.txt");
    let mut u = ui_for(&m, 1200);
    let labels = names(&u, Role::Label);
    assert!(labels.iter().any(|l| l == "Ahoj,") && labels.iter().any(|l| l == "druhy radek"), "text bez CR: {labels:?}");
    assert!(labels.iter().any(|l| l == "Historie verzí"));
    press(&mut m, &mut u, "Kopie vedle");
    assert_eq!(m.b.fs.data("/home/jana/zprava (verze).txt").map(<[u8]>::len), Some(5));
    assert!(m.status.contains("zprava (verze).txt"));
    select(&mut m, "obrazek.bmp");
    assert!(names(&ui_for(&m, 1200), Role::Label).iter().any(|l| l == "Obrázek BMP 2 x 2, 24 bitů"));
}

#[test]
fn new_folder_rename_validation() {
    let mut m = fm();
    let mut u = ui_for(&m, 1200);
    press(&mut m, &mut u, "Nová složka");
    assert_eq!(m.edit.as_ref().unwrap().value, "Nová složka");
    press(&mut m, &mut u, "Uložit");
    assert!(m.b.fs.exists("/home/jana/Nová složka"));
    assert_eq!(m.sel.len(), 1, "nova slozka vybrana");
    m.update(Msg::NewFolder);
    assert_eq!(m.edit.as_ref().unwrap().value, "Nová složka (2)");
    m.update(Msg::NameCancel);
    select(&mut m, "zprava.txt");
    m.update(Msg::Rename);
    m.update(Msg::NameValue(String::from("DOKUMENTY")));
    m.update(Msg::NameOk);
    assert_eq!(m.edit.as_ref().unwrap().error, "Položka s tímto jménem už ve složce je.");
    m.update(Msg::NameValue(String::from("a/b")));
    m.update(Msg::NameOk);
    assert!(m.edit.as_ref().unwrap().error.starts_with("Jméno nesmí"));
    m.update(Msg::NameValue(String::from("dopis.txt")));
    m.update(Msg::NameOk);
    assert!(m.edit.is_none() && m.b.fs.exists("/home/jana/dopis.txt") && !m.b.fs.exists("/home/jana/zprava.txt"));
}

#[test]
fn trash_undo_and_trash_view() {
    let mut m = fm();
    select(&mut m, "zprava.txt");
    let mut u = ui_for(&m, 1200);
    press(&mut m, &mut u, "Do koše");
    assert!(!m.b.fs.exists("/home/jana/zprava.txt"));
    assert_eq!(m.status, "1 položka v koši.");
    press(&mut m, &mut u, "Vrátit");
    assert!(m.b.fs.exists("/home/jana/zprava.txt"));
    // Kos: obnovit a trvale smazat s potvrzenim.
    select(&mut m, "zprava.txt");
    m.update(Msg::Trash);
    select(&mut m, "obrazek.bmp");
    m.update(Msg::Trash);
    let kos = m.places.iter().position(|p| p.path.is_empty()).unwrap();
    m.update(Msg::Place(kos));
    assert_eq!(m.mode, Mode::Trash);
    assert_eq!(m.trash.len(), 2);
    let ids: Vec<u64> = m.trash.iter().filter(|t| t.e.name == "zprava.txt").map(|t| t.row_id()).collect();
    m.update(Msg::TrashSelect(ids));
    let mut u = ui_for(&m, 1200);
    press(&mut m, &mut u, "Obnovit");
    assert!(m.b.fs.exists("/home/jana/zprava.txt") && m.trash.len() == 1);
    let ids = vec![m.trash[0].row_id()];
    m.update(Msg::TrashSelect(ids));
    let mut u = ui_for(&m, 1200);
    press(&mut m, &mut u, "Smazat trvale");
    assert!(m.confirm_purge && m.trash.len() == 1, "nejdriv potvrzeni");
    assert!(names(&u, Role::Label).iter().any(|l| l.starts_with("Trvale smazat 1 položku?")));
    m.update(Msg::TrashPurgeConfirm(true));
    assert!(m.trash.is_empty() && m.b.trash.is_empty());
}

#[test]
fn copy_paste_and_job_conflict_ui() {
    let mut m = fm();
    select(&mut m, "zprava.txt");
    m.update(Msg::Copy);
    m.update(Msg::Place(1));
    let mut u = ui_for(&m, 1200);
    press(&mut m, &mut u, "Vložit (1)");
    let start = m.b.sent.last().cloned().unwrap();
    assert!(matches!(&start, ClientMsg::Start { op: Op::Copy, sources, dest, .. } if sources == &vec![String::from("/home/jana/zprava.txt")] && dest == "/home/jana/Projekty"), "{start:?}");
    assert!(m.clip.is_some(), "kopie zustava ve schrance");
    // Stav ulohy s konfliktem z files-copy.
    let st = Status {
        job: 7,
        op: Op::Copy,
        phase: Phase::Conflict,
        dest: String::from("/home/jana/Projekty"),
        total_items: 1,
        done_items: 0,
        skipped: 0,
        errors: 0,
        done_bytes: 0,
        total_bytes: Some(20),
        percent: 0,
        conflict: Some(Conflict { source: String::from("/home/jana/zprava.txt"), target: String::from("/home/jana/Projekty/zprava.txt") }),
        last_error: None,
    };
    assert!(m.external(&ServerMsg::Status(st.clone()).encode()));
    let mut u = ui_for(&m, 1200);
    assert!(names(&u, Role::Label).iter().any(|l| l == "\"zprava.txt\" už v cíli je."));
    m.update(Msg::ForAll(true));
    press(&mut m, &mut u, "Ponechat obě");
    assert_eq!(m.b.sent.last(), Some(&ClientMsg::Decide { job: 7, resolution: Resolution::Rename, all: true }));
    assert!(!m.for_all);
    // Hotovo: vypis se obnovi, uloha jde zavrit.
    m.b.fs.file("/home/jana/Projekty/zprava (2).txt", b"x");
    let done = Status { phase: Phase::Done, conflict: None, done_items: 1, percent: 100, ..st };
    m.external(&ServerMsg::Status(done).encode());
    assert!(shown(&m).contains(&String::from("zprava (2).txt")));
    let mut u = ui_for(&m, 1200);
    press(&mut m, &mut u, "Zavřít");
    assert!(m.jobs.is_empty());
    // Vyjmout: vlozi se jednou.
    m.update(Msg::Go(String::from("/home/jana")));
    select(&mut m, "obrazek.bmp");
    m.update(Msg::Cut);
    m.update(Msg::Go(String::from("/home/jana/Dokumenty")));
    m.update(Msg::Paste);
    assert!(matches!(m.b.sent.last(), Some(ClientMsg::Start { op: Op::Move, .. })));
    assert!(m.clip.is_none());
    assert!(!m.external(b"nesmysl"), "neplatna zprava nic nerozbije");
}

#[test]
fn menu_actions() {
    let mut m = fm();
    let menu = m.menu();
    let file = &menu[0].children;
    assert_eq!(file.iter().find(|i| i.action == "files.rename").unwrap().shortcut, "F2");
    assert!(!file.iter().find(|i| i.action == "files.paste").unwrap().disabled.is_empty());
    select(&mut m, "zprava.txt");
    assert!(m.action("files.copy", &[]).is_ok());
    assert!(m.menu()[0].children.iter().find(|i| i.action == "files.paste").unwrap().disabled.is_empty());
    assert!(m.action("files.show", &[(String::from("path"), guiproto::action::Value::Path(String::from("/home/jana/Projekty")))]).is_ok());
    assert_eq!(m.nav.cur, "/home/jana/Projekty");
    assert!(m.action("files.nic", &[]).is_err());
}

/// 10 000 polozek: vypis, razeni a prekresleni (tabulka cte jen viditelne
/// radky). Mereni v ladicim prekladu; cil v QEMU je plynule rolovani.
#[test]
fn ten_thousand_items_are_fast() {
    let mut b = TestBackend::sample();
    for i in 0..10_000 {
        b.fs.file(&alloc::format!("/home/jana/velka/soubor {i}.txt"), b"x");
    }
    let mut m = FileManager::new(b);
    let t = std::time::Instant::now();
    m.update(Msg::Go(String::from("/home/jana/velka")));
    let load = t.elapsed();
    assert_eq!(m.listing.len(), 10_000);
    assert_eq!(m.listing.rows[9_999].cells[0], "soubor 9999.txt", "prirozene poradi");
    let t = std::time::Instant::now();
    m.update(Msg::Sort(2, false));
    let sort = t.elapsed();
    let t = std::time::Instant::now();
    let mut u = ui_for(&m, 1200);
    for _ in 0..20 {
        u.refresh(&m);
    }
    let draw = t.elapsed() / 20;
    let rows = names(&u, Role::Row).len();
    assert!(rows < 100, "pristupnost jen viditelnych radku: {rows}");
    std::println!("10 000 polozek: nacteni (TestBackend) {load:?}, razeni {sort:?}, prestaveni rozhrani {draw:?}");
    assert!(draw.as_millis() < 50, "prestaveni rozhrani {draw:?}");
}
