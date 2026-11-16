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
    (0..m.listing.len()).map(|i| m.listing.row(i).unwrap().cells[0].clone()).collect()
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
    let names = |l: &Listing| (0..l.len()).map(|i| l.row(i).unwrap().cells[0].clone()).collect::<Vec<_>>();
    assert_eq!(names(&l), ["alfa", "skryta", "Zeta", "b 2.txt", "b 10.txt"], "slozky prvni, docasne jmeno skryte");
    // Velikost sestupne: slozky stale prvni, nezname velikosti na konci.
    l.set_sort(SortBy::Size, false, Some(T0), None);
    assert_eq!(names(&l), ["Zeta", "alfa", "skryta", "b 2.txt", "b 10.txt"]);
    // Tri stavy: skryta a nedostupna velikost jako pomlcka.
    let row = |n: &str| (0..l.len()).map(|i| l.row(i).unwrap()).find(|r| r.cells[0] == n).unwrap();
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
    // Mista: seznam s ikonami a skupinami (O20), bez velkych nadpisu.
    let items = names(&u, Role::Row);
    assert!(items.contains(&String::from("Koš")) && items.contains(&String::from("Domov")), "{items:?}");
    assert!(names(&u, Role::Label).iter().any(|l| l == "Místa"), "nadpis skupiny");
    assert!(!names(&u, Role::Row).is_empty(), "radky tabulky v pristupnosti");
    assert!(names(&u, Role::ColumnHeader).contains(&String::from("Velikost")));
    assert!(names(&u, Role::Label).iter().any(|l| l.starts_with("4 položky")), "siroke okno ma nahled");
    let narrow = ui_for(&m, 400);
    assert!(!names(&narrow, Role::Label).iter().any(|l| l.starts_with("4 položky") || l == "Místa"), "uzke okno bez panelu");
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
    // Soubor: Enter / dvojklik posle cestu sluzbe "otevri cestu".
    let plan = m.listing.items.iter().find(|i| i.e.name == "plan.md").unwrap().e.oid;
    m.update(Msg::Activate(plan));
    assert_eq!(m.b.opened, vec![String::from("/home/jana/Projekty/plan.md")]);
    assert_eq!(m.nav.cur, "/home/jana/Projekty", "soubor nemeni slozku");
    assert!(m.status.starts_with("Otevírám plan.md"));
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
    // Dvojice klic - hodnota (`.kv`, O19).
    let pair = |labels: &[String], k: &str, v: &str| labels.windows(2).any(|w| w[0] == k && w[1] == v);
    assert!(pair(&labels, "Velikost", "\u{2014} (skryto: chybí právo Spravovat)"), "{labels:?}");
    assert!(labels.windows(2).any(|w| w[0] == "Obsah" && w[1].starts_with("\u{2014}")));
    select(&mut m, "Projekty");
    let labels = names(&ui_for(&m, 1200), Role::Label);
    assert!(pair(&labels, "Obsah", "1 soubor, 0 složek"), "{labels:?}");
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
    // Kopirovat: ucinek schranky se tremi formaty.
    let fx = m.effects();
    let ui::Effect::Copy { formats } = &fx[0] else { panic!("{fx:?}") };
    let fmts: Vec<&str> = formats.iter().map(|f| f.0.as_str()).collect();
    assert_eq!(fmts, ["application/x-mo-paths", "text/uri-list", "text/plain;charset=utf-8"]);
    m.update(Msg::Place(1));
    let mut u = ui_for(&m, 1200);
    press(&mut m, &mut u, "Vložit (1)");
    // Vlozit: zadost o schranku, data prijdou v `pasted`.
    assert!(matches!(m.effects().as_slice(), [ui::Effect::Paste { formats }] if formats[0] == "application/x-mo-paths"));
    let paths = formats.iter().find(|f| f.0 == "application/x-mo-paths").unwrap().1.clone();
    assert!(m.pasted("application/x-mo-paths", &paths));
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
    let Some(ui::Effect::Copy { formats }) = m.effects().pop() else { panic!() };
    m.update(Msg::Go(String::from("/home/jana/Dokumenty")));
    m.update(Msg::Paste);
    let data = &formats[0].1;
    assert!(data.starts_with(b"move\n"), "operace ve formatu");
    m.pasted("application/x-mo-paths", data);
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
    assert!(!file.iter().find(|i| i.action == "files.copy").unwrap().disabled.is_empty(), "nic vybrano");
    select(&mut m, "zprava.txt");
    assert!(m.action("files.copy", &[]).is_ok());
    assert!(m.menu()[0].children.iter().find(|i| i.action == "files.copy").unwrap().disabled.is_empty());
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
    assert_eq!(m.listing.row(9_999).unwrap().cells[0], "soubor 9999.txt", "prirozene poradi");
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

#[test]
fn keys_map_to_actions() {
    use crate::app::Mode;
    use crate::key_msg;
    use inputproto::code::*;
    use inputproto::mods::{ALT, CTRL, SHIFT};
    let k = |code, ch, mods| key_msg(false, Mode::Folder, code, ch, mods);
    assert_eq!(k(KEY_F2, None, 0), Some(Msg::Rename));
    assert_eq!(k(KEY_DELETE, None, 0), Some(Msg::Trash));
    assert_eq!(k(KEY_BACKSPACE, None, 0), Some(Msg::Up));
    assert_eq!(k(KEY_LEFT, None, ALT), Some(Msg::Back));
    assert_eq!(k(KEY_RIGHT, None, ALT), Some(Msg::Forward));
    assert_eq!(k(KEY_UP, None, ALT), Some(Msg::Up));
    assert_eq!(k(KEY_F5, None, 0), Some(Msg::Refresh));
    // Ctrl+C podle znaku (rozlozeni), ne podle kodu klavesy.
    assert_eq!(k(KEY_I, Some('c'), CTRL), Some(Msg::Copy));
    assert_eq!(k(KEY_X, Some('X'), CTRL | SHIFT), Some(Msg::Cut));
    assert_eq!(k(KEY_V, Some('v'), CTRL), Some(Msg::Paste));
    assert_eq!(k(KEY_N, Some('n'), CTRL | SHIFT), Some(Msg::NewFolder));
    assert_eq!(k(KEY_N, Some('n'), CTRL), None, "Ctrl+N nechame systemu (nove okno)");
    assert_eq!(k(KEY_C, Some('c'), CTRL | ALT), None, "AltGr kombinace neni zkratka");
    assert_eq!(k(KEY_LEFT, None, 0), None);
    // Kos: Delete = trvale smazat (s potvrzenim), kopirovani nic.
    assert_eq!(key_msg(false, Mode::Trash, KEY_DELETE, None, 0), Some(Msg::TrashPurge));
    assert_eq!(key_msg(false, Mode::Trash, KEY_C, Some('c'), CTRL), None);
    // Pole jmena: jen Escape.
    assert_eq!(key_msg(true, Mode::Folder, KEY_ESC, None, 0), Some(Msg::NameCancel));
    assert_eq!(key_msg(true, Mode::Folder, KEY_DELETE, None, 0), None);
}

#[test]
fn clipboard_formats_roundtrip() {
    use crate::clipfmt::{URI_LIST, decode, encode};
    let paths = vec![String::from("/home/jana/a b.txt"), String::from("/home/jana/přílohy")];
    let f = encode(Op::Move, &paths).unwrap();
    assert_eq!(decode(&f[0].0, &f[0].1), Some((Op::Move, paths.clone())));
    let uris = core::str::from_utf8(&f[1].1).unwrap();
    assert!(uris.starts_with("file:///home/jana/a%20b.txt\r\n"), "{uris}");
    assert_eq!(decode(URI_LIST, &f[1].1), Some((Op::Copy, paths.clone())), "uri-list je vzdy kopie");
    assert_eq!(decode(URI_LIST, b"# komentar\r\nfile://localhost/etc/x\r\nhttp://web/y\r\n"), Some((Op::Copy, vec![String::from("/etc/x")])));
    assert_eq!(decode(URI_LIST, b"file:///a%ZZ"), None);
    assert_eq!(decode("application/x-mo-paths", b"smazat\n/a"), None);
    assert_eq!(decode("application/x-mo-paths", b"copy\nrelativni"), None);
    // Prilis mnoho cest: do schranky se nevejdou, vklada se jen v okne.
    let many: Vec<String> = (0..5000).map(|i| alloc::format!("/home/jana/dlouhe jmeno souboru {i}.txt")).collect();
    assert!(encode(Op::Copy, &many).is_none());
}

#[test]
fn big_selection_pastes_locally() {
    let mut b = TestBackend::sample();
    for i in 0..3000 {
        b.fs.file(&alloc::format!("/home/jana/hodne/soubor s dlouhym jmenem {i}.txt"), b"x");
    }
    let mut m = FileManager::new(b);
    m.update(Msg::Go(String::from("/home/jana/hodne")));
    let all: Vec<u64> = m.listing.items.iter().map(|i| i.e.oid).collect();
    m.update(Msg::Select(all));
    m.update(Msg::Copy);
    assert!(m.effects().is_empty() && m.clip_local);
    assert!(m.status.contains("příliš velký"));
    m.update(Msg::Go(String::from("/home/jana/Dokumenty")));
    let before = m.b.sent.len();
    m.update(Msg::Paste);
    let starts = m.b.sent[before..].iter().filter(|s| matches!(s, ClientMsg::Start { .. })).count();
    assert_eq!(starts, 3000usize.div_ceil(fileops::proto::MAX_SOURCES), "po davkach");
}

#[test]
fn drop_on_folder_moves() {
    let mut m = fm();
    let id = |m: &Fm, n: &str| m.listing.items.iter().find(|i| i.e.name == n).unwrap().e.oid;
    let (txt, docs, bmp) = (id(&m, "zprava.txt"), id(&m, "Dokumenty"), id(&m, "obrazek.bmp"));
    m.update(Msg::Drop(Some(docs), vec![txt]));
    assert!(matches!(m.b.sent.last(), Some(ClientMsg::Start { op: Op::Move, sources, dest, .. }) if sources == &vec![String::from("/home/jana/zprava.txt")] && dest == "/home/jana/Dokumenty"));
    let n = m.b.sent.len();
    m.update(Msg::Drop(Some(bmp), vec![txt]));
    m.update(Msg::Drop(None, vec![txt]));
    m.update(Msg::Drop(Some(docs), vec![docs]));
    assert_eq!(m.b.sent.len(), n, "na soubor, prazdne misto ani sebe nic");
}

#[test]
fn context_menu_and_open_reply() {
    let mut m = fm();
    let id = m.listing.items.iter().find(|i| i.e.name == "zprava.txt").unwrap().e.oid;
    m.update(Msg::Context(id));
    assert_eq!(m.sel, vec![id]);
    m.update(Msg::OpenSelected);
    assert_eq!(m.b.opened, vec![String::from("/home/jana/zprava.txt")]);
    assert!(m.external_from(1, &openwith::msg::Reply::Revealed(String::from("org.magnumopus.files")).encode()));
    assert_eq!(m.status, "Pro tento typ souboru zatím není aplikace.");
    m.external_from(1, &openwith::msg::Reply::NoApp(String::from("xyz")).encode());
    assert_eq!(m.status, "Pro soubory .xyz zatím není aplikace.");
    m.external_from(1, &openwith::msg::Reply::NoApp(String::new()).encode());
    assert_eq!(m.status, "Pro soubory bez přípony zatím není aplikace.");
    m.external_from(1, &openwith::msg::Reply::Failed(String::from("x: NOT_FOUND")).encode());
    assert_eq!(m.status, "Nejde otevřít: x: NOT_FOUND");
    assert!(!m.external_from(1, b"nesmysl"));
    assert!(!m.external_from(7, b"ok x"), "neznamy kanal");
}

#[test]
fn bmp_preview_shows_image() {
    let mut m = fm();
    select(&mut m, "obrazek.bmp");
    let p = m.preview.as_ref().unwrap();
    let (w, h, px) = p.image.as_ref().expect("obrazek");
    assert_eq!((*w, *h, px.len()), (2, 2, 4));
    let u = ui_for(&m, 1200);
    assert!(u.a11y().iter().any(|n| n.role == Role::Image && n.name == "Obrázek BMP 2 x 2, 24 bitů"));
}

/// `/home` jako u uzivatele: vlastni domov a cizi domov bez prava vypisu.
fn home_with_foreign() -> Fm {
    let mut b = TestBackend::sample();
    b.fs.file("/home/petr/tajne.txt", b"x");
    b.locked.push(String::from("/home/petr"));
    let mut m = FileManager::new(b);
    m.update(Msg::Go(String::from("/home")));
    m
}

#[test]
fn foreign_folder_has_lock() {
    let m = home_with_foreign();
    let petr = m.listing.items.iter().find(|i| i.e.name == "petr").expect("cizi domov ve vypisu");
    assert!(petr.locked());
    assert_eq!((petr.icon(), petr.kind_text()), ("lock", "Složka bez přístupu"));
    let jana = m.listing.items.iter().find(|i| i.e.name == "jana").unwrap();
    assert!(!jana.locked());
    assert_eq!(jana.icon(), "folder");
}

#[test]
fn denied_listing_is_not_empty_folder() {
    let mut m = home_with_foreign();
    let id = m.listing.items.iter().find(|i| i.e.name == "petr").unwrap().e.oid;
    m.update(Msg::Activate(id));
    assert_eq!(m.nav.cur, "/home/petr");
    assert_eq!(m.list_error, Some(abi::Error::AccessDenied));
    let labels = names(&ui_for(&m, 1200), Role::Label);
    assert!(labels.iter().any(|l| l == "Ke složce petr nemáte přístup."), "{labels:?}");
    assert!(!labels.iter().any(|l| l.contains("prázdná")), "odmitnuty vypis neni prazdna slozka: {labels:?}");
    assert!(!names(&ui_for(&m, 1200), Role::Button).iter().any(|b| b == "Nová složka"), "bez vypisu ani akce");
    // Navrat zpet: chyba zmizi.
    m.update(Msg::Back);
    assert_eq!(m.list_error, None);
    // Neexistujici slozka: duvod, ne prazdna slozka.
    m.update(Msg::Go(String::from("/home/nikdo")));
    assert!(names(&ui_for(&m, 1200), Role::Label).iter().any(|l| l == "Složku nikdo nejde otevřít: neexistuje."));
}

/// Prepnuti rezimu: okno se prekresli barvami motivu (svetly, tmavy,
/// kontrast). Snimky do `target/shots/soubory-*.png`.
#[test]
fn theme_modes_redraw_window() {
    // Siroke okno s vybranou slozkou: nahled s dlouhou hodnotou (O19).
    let mut b = TestBackend::sample();
    b.hidden_tree.push(String::from("/home/jana/Dokumenty"));
    let mut m = FileManager::new(b);
    select(&mut m, "Dokumenty");
    let (w, h) = (1200u32, 700u32);
    let mut last = None;
    for (name, th) in [
        ("svetly", theme::Theme::builtin(theme::Mode::Light)),
        ("tmavy", theme::Theme::builtin(theme::Mode::Dark)),
        ("kontrast", theme::Theme::builtin(theme::Mode::Contrast)),
    ] {
        let mut u = ui::Ui::new(ui::Look::ui_fonts(), w, h);
        u.look.theme = th.clone();
        u.refresh(&m);
        let mut px = vec![0u32; (w * h) as usize];
        let mut c = gfx::Canvas::new(&mut px, w, h, w).unwrap();
        u.draw(&mut c, 0);
        let corner = px[((h - 2) * w + w - 2) as usize];
        assert_eq!(corner, th.color.surface, "{name}: pozadi okna");
        assert_ne!(Some(corner), last, "{name}: jiny nez predchozi rezim");
        last = Some(corner);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(alloc::format!("soubory-{name}.png")), gfx::png::encode(&px, w, h, w));
    }
}

#[test]
fn download_from_address() {
    let mut m = fm();
    m.update(Msg::Download);
    assert!(m.edit.as_ref().is_some_and(|e| e.url && e.value == "https://"));
    let mut u = ui_for(&m, 1200);
    // Spatna adresa: chyba u pole, nic se neposle.
    m.update(Msg::NameValue("ftp://server/soubor".into()));
    m.update(Msg::NameOk);
    assert!(m.edit.as_ref().unwrap().error.contains("https://"));
    m.update(Msg::NameValue("https://".into()));
    m.update(Msg::NameOk);
    assert!(m.edit.as_ref().unwrap().error.contains("server"));
    assert!(m.b.downloads.is_empty());
    // Dobra adresa: stahovani predano, pole zavrene, zprava v liste.
    m.update(Msg::NameValue("https://example.org/obraz.iso".into()));
    u = ui_for(&m, 1200);
    press(&mut m, &mut u, "Stáhnout");
    assert_eq!(m.b.downloads, vec![String::from("https://example.org/obraz.iso")]);
    assert!(m.edit.is_none());
    assert!(m.status.contains("Úlohách"));
    // Prikaz je v menu (paleta Ctrl+K).
    assert!(crate::app_menu(false)[0].children.iter().any(|i| i.action == "files.download"));
}

#[test]
fn media_places_and_eject() {
    let mut m = fm();
    m.b.fs.dir("/media").dir("/media/MOTEST").file("/media/MOTEST/foto.jpg", b"jpg");
    let medium = |name: &str, ro: bool| fsproto::v2::MediaEntry { name: name.into(), label: name.into(), fs: "fat".into(), read_only: ro, total_bytes: 64 << 20, free_bytes: 32 << 20 };
    m.b.media = vec![medium("MOTEST", false), medium("mylabel", true)];
    m.update(Msg::Refresh);
    let labels: Vec<&str> = m.places.iter().map(|p| p.label.as_str()).collect();
    assert!(labels.contains(&"MOTEST") && labels.contains(&"mylabel (jen čtení)"), "{labels:?}");
    let i = m.places.iter().position(|p| p.label == "MOTEST").unwrap();
    m.update(Msg::Place(i));
    assert_eq!(m.nav.cur, "/media/MOTEST");
    // Odpojit: tlacitko jen ve slozce media.
    let mut u = ui_for(&m, 1200);
    press(&mut m, &mut u, "Odpojit");
    assert!(m.status.contains("můžete ho vytáhnout"), "{}", m.status);
    assert_eq!(m.nav.cur, "/home/jana");
    assert!(!m.places.iter().any(|p| p.label == "MOTEST"));
    let u = ui_for(&m, 1200);
    assert!(!names(&u, Role::Button).iter().any(|n| n == "Odpojit"), "doma tlacitko neni");
}

/// Stazene mezi misty (ikona stahovani) a puvod stazeneho souboru
/// v nahledu (`sys.origin`, zapsal ho fetch); media ve vlastni skupine.
#[test]
fn downloads_place_origin_and_media_group() {
    let mut m = fm();
    m.b.fs.dir("/home/jana/Stazene").file("/home/jana/Stazene/data.bin", b"abc");
    m.b.origins.insert(String::from("/home/jana/Stazene/data.bin"), b"https://example.org/data.bin\n1794571200000000000".to_vec());
    m.b.fs.dir("/media").dir("/media/USB");
    m.b.media = vec![fsproto::v2::MediaEntry { name: "USB".into(), label: "USB".into(), fs: "fat".into(), read_only: false, total_bytes: 1 << 20, free_bytes: 1 << 19 }];
    m.update(Msg::Refresh);
    let p = m.places.iter().find(|p| p.label == "Stažené").expect("misto Stazene");
    assert_eq!((p.icon, p.path.as_str()), ("download", "/home/jana/Stazene"));
    let u = ui_for(&m, 1200);
    let labels = names(&u, Role::Label);
    assert!(labels.iter().any(|l| l == "Média"), "skupina medii: {labels:?}");
    let i = m.places.iter().position(|p| p.label == "Stažené").unwrap();
    m.update(Msg::Place(i));
    select(&mut m, "data.bin");
    let labels = names(&ui_for(&m, 1200), Role::Label);
    assert!(labels.windows(2).any(|w| w[0] == "Staženo z" && w[1] == "https://example.org/data.bin"), "{labels:?}");
    assert!(crate::backend::parse_origin(b"bez casu").is_none());
}

/// Misto vybrane pres pristupnost (akce `activate` na radku, jako agent
/// nebo test v QEMU) otevre slozku.
#[test]
fn place_row_activates_by_a11y() {
    let mut m = fm();
    let mut u = ui_for(&m, 1200);
    let n = u.a11y().into_iter().find(|n| n.role == Role::Row && n.name == "Projekty").unwrap();
    let r = u.a11y_action(n.id, a11y_actions::ACTIVATE, "");
    for msg in r.msgs {
        m.update(msg);
    }
    u.refresh(&m);
    assert_eq!(m.nav.cur, "/home/jana/Projekty");
}
