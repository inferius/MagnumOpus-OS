//! Testy motivu, barevneho vykresleni a zvyrazneni v protokolu.

use shlang::highlight::{Class, highlight};
use shlang::value::Value;
use shlang::vfs::mem::MemFs;

use crate::proto::{ClientMsg, ServerMsg};
use crate::render::{self, age_level, file_class, sgr, size_level};
use crate::screen::Screen;
use crate::session::Service;
use crate::theme::{FileClass, Rgb, Style, Theme};

fn rec(name: &str, kind: &str, ext: &str, access: &str, size: i64) -> Value {
    Value::record(&[
        ("name", Value::str(name)),
        ("kind", Value::str(kind)),
        ("size", Value::Size(size)),
        ("ext", Value::str(ext)),
        ("access", Value::str(access)),
    ])
}

#[test]
fn theme_file_overrides_builtin() {
    let t = Theme::parse_toml(
        "base = \"light\"   # svetly\n[syntax]\ncmd = \"#112233 bold\"\n[files]\ndir = \"#445566\"\n[ui]\nscale3 = \"#ff0000\"\n[ext]\nimage = \".raw jxl\"\n",
    )
    .unwrap();
    assert_eq!(t.name, "light");
    assert_eq!(t.syntax(Class::Command), Style { bold: true, ..Style::fg(Rgb(0x11, 0x22, 0x33)) });
    assert_eq!(t.file(FileClass::Dir).fg, Rgb(0x44, 0x55, 0x66));
    assert_eq!(t.scale[3], Rgb(0xff, 0, 0));
    assert_eq!(t.ext_class("RAW"), Some(FileClass::Image));
    assert_eq!(t.ext_class("jpg"), Some(FileClass::Image), "vychozi pripony zustanou");
    // Chyby s cislem radku.
    assert_eq!(Theme::parse_toml("[syntax]\ncmd = \"zelena\"\n").unwrap_err().0, 2);
    assert_eq!(Theme::parse_toml("[barvy]\n").unwrap_err().0, 1);
    assert_eq!(Theme::parse_toml("[syntax]\nneni = \"#000000\"\n").unwrap_err().0, 2);
}

#[test]
fn file_records_get_kind_colors() {
    let t = Theme::dark();
    assert_eq!(file_class(&rec("foto", "dir", "", "rwx", 0), &t), Some(FileClass::Dir));
    assert_eq!(file_class(&rec("repo", "dataset", "", "rw-", 0), &t), Some(FileClass::Dataset));
    assert_eq!(file_class(&rec("a.jpg", "file", "jpg", "rw-", 0), &t), Some(FileClass::Image));
    assert_eq!(file_class(&rec("b.zst", "file", "zst", "rw-", 0), &t), Some(FileClass::Archive));
    assert_eq!(file_class(&rec("run", "file", "", "r-x", 0), &t), Some(FileClass::Exec));
    assert_eq!(file_class(&rec(".env", "file", "", "rw-", 0), &t), Some(FileClass::Hidden));
    assert_eq!(file_class(&rec("tajne", "file", "", "---", 0), &t), Some(FileClass::NoAccess));
    assert_eq!(file_class(&rec("x.txt", "file", "txt", "", 0), &t), Some(FileClass::Document), "prava neznama");
    assert_eq!(file_class(&Value::record(&[("a", Value::Int(1))]), &t), None);
}

#[test]
fn scales_for_size_and_age() {
    assert_eq!(size_level(10), 0);
    assert_eq!(size_level(5 << 20), 1);
    assert_eq!(size_level(500 << 20), 2);
    assert_eq!(size_level(2 << 30), 3);
    let day = 86_400 * 1_000_000_000i64;
    let now = 1000 * day;
    assert_eq!(age_level(now - 10, now), 0);
    assert_eq!(age_level(now - 5 * day, now), 1);
    assert_eq!(age_level(now - 100 * day, now), 2);
    assert_eq!(age_level(now - 800 * day, now), 3);
}

#[test]
fn colored_table_and_line() {
    let t = Theme::dark();
    let rows = Value::List(alloc::vec![rec("foto", "dir", "", "rwx", 0), rec("big.zst", "file", "zst", "rw-", 3 << 30)]);
    let out = render::value(&rows, &t, None);
    assert!(out.contains(&alloc::format!("{}foto", sgr(t.file(FileClass::Dir)))), "{out:?}");
    assert!(out.contains(&sgr(Style::fg(t.scale[3]))), "obri soubor v nejvyssim stupni skaly");
    // Bez barev zarovnani stejne jako text.
    assert_eq!(crate::vt::strip(&out).lines().count(), 3);
    let src = "ls -r | where size > 5MB";
    let l = render::line(src, &highlight(src, None), &t);
    assert!(l.starts_with(&alloc::format!("{}ls", sgr(t.syntax(Class::Command)))));
    assert_eq!(crate::vt::strip(&l), src);
}

#[test]
fn screen_understands_truecolor_and_underline() {
    let mut s = Screen::new(10, 2);
    s.write("\x1b[0;38;2;1;2;3;4mab\x1b[0mc");
    assert_eq!(s.cell(0, 0).fg, (1, 2, 3));
    assert!(s.cell(1, 0).underline);
    assert!(!s.cell(2, 0).underline);
}

#[test]
fn docs_css_comes_from_theme() {
    let t = Theme::dark();
    let vars = t.css_vars(":root");
    assert!(vars.contains(&alloc::format!("--sh-cmd: {};", t.syntax(Class::Command).fg.hex())));
    assert!(vars.contains("--fk-dir:"));
    let rules = t.css_rules();
    assert!(rules.contains(".sh-unknown { color: var(--sh-unknown); text-decoration: underline wavy;"));
}

#[test]
fn service_highlights_with_session_knowledge() {
    let mut svc = Service::single("/home", 1);
    let mut link = super::tests::TestLink::default();
    let mut f = MemFs::new();
    f.put("/home/a.txt", b"a");
    let mut env = super::tests::TestEnv::default();
    svc.handle(ClientMsg::Attach { session: 1 }, &mut link, &mut f, &mut env);
    svc.handle(ClientMsg::Run { session: 1, line: "let x = 5".into() }, &mut link, &mut f, &mut env);
    svc.handle(ClientMsg::Highlight { session: 1, line: "lss $x".into(), seq: 7 }, &mut link, &mut f, &mut env);
    let Some(ServerMsg::Highlighted { seq, spans, error }) = link.sent.last().cloned() else {
        panic!("cekal jsem Highlighted: {:?}", link.sent.last());
    };
    assert_eq!(seq, 7);
    assert_eq!(spans[0], (0, 3, Class::Unknown as u8), "neznamy prikaz");
    assert_eq!(spans[1], (4, 2, Class::Var as u8));
    assert!(error.is_none());
    // Zprava projde kodovanim.
    let m = ServerMsg::Highlighted { seq: 1, spans: alloc::vec![(0, 2, 1)], error: Some((3, 1, "x".into())) };
    assert_eq!(ServerMsg::decode(&m.encode()).unwrap(), m);
    let c = ClientMsg::Highlight { session: 2, line: "ls".into(), seq: 3 };
    assert_eq!(ClientMsg::decode(&c.encode()).unwrap(), c);
}
