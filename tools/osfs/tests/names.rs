//! Testy prekladu jmen mezi nasim FS a Windows.
//!
//! Akceptacni test z navrhu (`docs/cs/navrh-fs-v3.md`, "Ovladac pro
//! Windows"): pro nahodna jmena kazde jmeno ve vypisu otevre prave objekt,
//! ze ktereho vzniklo, a zadne dve polozky nemaji stejne zobrazene jmeno.

use std::collections::{HashMap, HashSet};

use fs::{FormatOptions, Fs, Kind, MemDevice, fold_name};
use osfs::image::{HiddenWhy, hidden_entries, looks_like_device, parse_size};
use osfs::names::*;
use proptest::prelude::*;

#[test]
fn plain_names_unchanged() {
    for n in [
        "a.txt",
        "Příliš žluťoučký kůň.doc",
        "x y",
        ".gitignore",
        "a..b",
        "COM10",
        "CONx",
        "LPT",
    ] {
        assert_eq!(to_windows(n).as_deref(), Some(n), "{n}");
        assert_eq!(from_windows(n).as_deref(), Ok(n), "{n}");
    }
}

#[test]
fn forbidden_chars_map_to_private_use() {
    assert_eq!(to_windows("a:b").as_deref(), Some("a\u{F03A}b"));
    assert_eq!(
        to_windows("<*?>").as_deref(),
        Some("\u{F03C}\u{F02A}\u{F03F}\u{F03E}")
    );
    assert_eq!(
        to_windows("a\\b|c\"").as_deref(),
        Some("a\u{F05C}b\u{F07C}c\u{F022}")
    );
    assert_eq!(from_windows("a\u{F03A}b").as_deref(), Ok("a:b"));
}

#[test]
fn hidden_names() {
    assert_eq!(to_windows_checked("a\u{F03A}b"), Err(Hidden::PrivateUse));
    assert_eq!(to_windows_checked("x\u{F041}"), Err(Hidden::PrivateUse));
    for n in [
        "CON",
        "con",
        "Con.txt",
        "NUL.tar.gz",
        "nul .txt",
        "AUX",
        "PRN.",
        "COM1",
        "com9.log",
        "LPT1",
        "LPT9.x",
        "COM\u{B9}",
        "LPT\u{B2}.a",
        "CONIN$",
        "conout$.txt",
    ] {
        assert!(to_windows(n).is_none(), "{n} ma byt skryte");
    }
    assert_eq!(to_windows_checked("a."), Err(Hidden::TrailingDotOrSpace));
    assert_eq!(to_windows_checked("a "), Err(Hidden::TrailingDotOrSpace));
    assert_eq!(to_windows_checked(&"x".repeat(255)), Ok("x".repeat(255)));
    assert_eq!(to_windows_checked(&"x".repeat(256)), Err(Hidden::TooLong));
    // 128 znaku mimo BMP = 256 jednotek UTF-16.
    assert_eq!(
        to_windows_checked(&"\u{1F600}".repeat(128)),
        Err(Hidden::TooLong)
    );
    assert!(to_windows("COM0").is_some());
}

#[test]
fn windows_input_rejected() {
    // U+F0xx, ktere neodpovida zakazanemu znaku.
    assert_eq!(from_windows("a\u{F041}"), Err(BadName::StrayPrivateUse));
    assert_eq!(from_windows("a:b"), Err(BadName::Forbidden));
    assert!(matches!(
        from_windows("CON"),
        Err(BadName::WouldBeHidden(Hidden::Reserved))
    ));
    assert!(matches!(from_windows("a."), Err(BadName::WouldBeHidden(_))));
    assert_eq!(
        from_windows("faktura\u{202E}fdp.exe"),
        Err(BadName::RejectedByFs)
    );
    assert_eq!(from_windows(".."), Err(BadName::RejectedByFs));
    assert_eq!(from_windows(""), Err(BadName::RejectedByFs));
}

#[test]
fn paths() {
    assert_eq!(path_from_windows("\\").as_deref(), Ok("/"));
    assert_eq!(path_from_windows("").as_deref(), Ok("/"));
    assert_eq!(
        path_from_windows("\\a\\b\u{F03A}c").as_deref(),
        Ok("/a/b:c")
    );
    assert!(path_from_windows("\\a\\CON\\b").is_err());
}

#[test]
fn devices_rejected() {
    for p in [
        "E:",
        "e:\\",
        "\\\\.\\PhysicalDrive0",
        "\\.\\PhysicalDrive1",
        "//./PhysicalDrive2",
        "\\\\?\\Volume{x}",
        "/dev/sdb",
    ] {
        assert!(looks_like_device(p), "{p}");
    }
    for p in [
        "obraz.img",
        "C:\\x\\obraz.img",
        "./a.img",
        "E:\\obrazy\\x.img",
    ] {
        assert!(!looks_like_device(p), "{p}");
    }
}

#[test]
fn sizes() {
    assert_eq!(parse_size("64M"), Ok(64 << 20));
    assert_eq!(parse_size("1g"), Ok(1 << 30));
    assert_eq!(parse_size("4096"), Ok(4096));
    assert!(parse_size("x").is_err());
    assert!(parse_size("99999999999T").is_err());
}

/// Generator jmen se zaludnostmi: zakazane znaky, U+F0xx, rezervovana
/// jmena, koncove tecky a mezery, velikost pismen, diakritika, dlouha jmena.
fn name_strategy() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        3 => "[a-cA-C]{1,3}",
        1 => Just(":".to_string()),
        1 => Just("\\".to_string()),
        1 => Just("*".to_string()),
        1 => Just("?".to_string()),
        1 => Just("\"".to_string()),
        1 => Just("<".to_string()),
        1 => Just(">".to_string()),
        1 => Just("|".to_string()),
        1 => Just(".".to_string()),
        1 => Just(" ".to_string()),
        1 => Just("\u{F03A}".to_string()),
        1 => Just("\u{F041}".to_string()),
        1 => Just("\u{F0FF}".to_string()),
        1 => Just("con".to_string()),
        1 => Just("NUL".to_string()),
        1 => Just("Com1".to_string()),
        1 => Just("lpt9".to_string()),
        1 => Just("é".to_string()),
        1 => Just("E\u{301}".to_string()),
        1 => Just("ů".to_string()),
        1 => Just("\u{1F600}".to_string()),
        1 => Just("x".repeat(250)),
    ];
    prop::collection::vec(piece, 1..5).prop_map(|v| v.concat())
}

/// Vytvori v adresari `dir` objekty s danymi jmeny (platnymi pro nas FS),
/// vrati jmeno -> oid.
fn populate(fs: &mut Fs<MemDevice>, dir: &str, names: &[String]) -> HashMap<String, u64> {
    let mut out = HashMap::new();
    for (i, n) in names.iter().enumerate() {
        if fs::validate_name(n).is_err() {
            continue;
        }
        let path = if dir == "/" {
            format!("/{n}")
        } else {
            format!("{dir}/{n}")
        };
        let kind = if i % 3 == 0 { Kind::Dir } else { Kind::File };
        if fs.create(&path, kind).is_ok() {
            let oid = fs.stat(&path).unwrap().oid;
            out.insert(fold_name(n), oid);
        }
    }
    out
}

fn new_fs() -> Fs<MemDevice> {
    let dev = Fs::format(MemDevice::new(4096), &FormatOptions::default()).unwrap();
    Fs::mount(dev).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Akceptacni test: kazde zobrazene jmeno otevre prave svuj objekt,
    /// zadna dve se nezobrazi stejne (ani bez ohledu na velikost pismen),
    /// skryte jmeno nejde z Windows zadat.
    #[test]
    fn listing_roundtrip(names in prop::collection::vec(name_strategy(), 1..25)) {
        let mut fs = new_fs();
        fs.create("/d", Kind::Dir).unwrap();
        let created = populate(&mut fs, "/d", &names);

        let mut shown = HashSet::new();
        let mut shown_folded = HashSet::new();
        for e in fs.list("/d").unwrap() {
            let oid = created[&fold_name(&e.name)];
            match to_windows(&e.name) {
                Some(win) => {
                    prop_assert!(shown.insert(win.clone()), "dvakrat zobrazeno: {:?}", win);
                    prop_assert!(shown_folded.insert(fold_name(&win)), "stejne az na velikost pismen: {:?}", win);
                    prop_assert!(win.encode_utf16().count() <= WIN_NAME_MAX);
                    prop_assert!(!win.chars().any(|c| FORBIDDEN.contains(&c)));
                    // Cesta tak, jak ji posle Windows.
                    let back = path_from_windows(&format!("\\d\\{win}")).unwrap();
                    prop_assert_eq!(&back, &format!("/d/{}", e.name));
                    prop_assert_eq!(fs.stat(&back).unwrap().oid, oid);
                    // Jine velikosti pismen vedou na stejny objekt.
                    let upper = win.to_uppercase();
                    if let Ok(p) = path_from_windows(&format!("\\d\\{upper}"))
                        && let Ok(st) = fs.stat(&p) {
                            prop_assert_eq!(st.oid, oid);
                        }
                }
                None => {
                    // Skryte jmeno: zadani ve Windows (tak jak je, nebo
                    // s privatnimi znaky misto zakazanych) nevede na nej.
                    let attempts = [e.name.clone(), e.name.chars().map(|c| {
                        if FORBIDDEN.contains(&c) { char::from_u32(PUA_BASE + c as u32).unwrap() } else { c }
                    }).collect::<String>()];
                    for a in attempts {
                        if let Ok(n) = from_windows(&a) {
                            prop_assert_ne!(fold_name(&n), fold_name(&e.name));
                        }
                    }
                }
            }
        }
    }

    /// Obecna vlastnost prekladu: from_windows(w) == Ok(n) <=> to_windows(n) == Some(w).
    #[test]
    fn translation_is_bijective(n in name_strategy()) {
        if let Some(w) = to_windows(&n)
            && fs::validate_name(&n).is_ok() {
                prop_assert_eq!(from_windows(&w), Ok(n.clone()));
            }
        if let Ok(back) = from_windows(&n) {
            prop_assert_eq!(to_windows(&back), Some(n.clone()));
        }
    }
}

#[test]
fn check_lists_hidden() {
    let mut fs = new_fs();
    fs.create("/ok", Kind::Dir).unwrap();
    fs.create("/ok/CON.txt", Kind::File).unwrap();
    fs.create("/ok/a.", Kind::File).unwrap();
    fs.create("/ok/x\u{F041}", Kind::File).unwrap();
    fs.create("/aux", Kind::Dir).unwrap();
    // Do skryteho adresare se nevstupuje.
    fs.create("/aux/CON", Kind::File).unwrap();
    fs.symlink("/ok/odkaz", "/ok").unwrap();
    let h = hidden_entries(&mut fs).unwrap();
    let paths: Vec<&str> = h.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "/aux",
            "/ok/CON.txt",
            "/ok/a.",
            "/ok/odkaz",
            "/ok/x\u{F041}"
        ]
    );
    assert_eq!(h[0].why, HiddenWhy::Name(Hidden::Reserved));
    assert_eq!(h[3].why, HiddenWhy::Symlink);
}
