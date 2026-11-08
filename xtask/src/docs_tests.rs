//! Testy generatoru dokumentace: prepis a kontrola odkazu do `design/`.

use super::*;
use crate::docs_books::parse_index;

/// Vyvojarska kniha (dokumentace v `docs/`).
fn dev() -> &'static BookDef {
    BOOKS.iter().find(|b| b.out_sub.is_empty()).unwrap()
}

/// Uzivatelska prirucka (`docs/prirucka/`).
fn guide() -> &'static BookDef {
    BOOKS.iter().find(|b| b.out_sub == "prirucka").unwrap()
}

#[test]
fn odkaz_do_design_se_prepise_o_uroven_vys() {
    assert_eq!(rewrite_link("../../design/prirucka.html", dev()), "../design/prirucka.html");
    assert_eq!(
        rewrite_link("../../design/okna.html#stavy", dev()),
        "../design/okna.html#stavy"
    );
    // Ostatni odkazy se chovaji jako drive.
    assert_eq!(rewrite_link("vzhled.md#a", dev()), "vzhled.html#a");
    assert_eq!(rewrite_link("https://x/../../design/a", dev()), "https://x/../../design/a");
}

#[test]
fn render_sbira_prepsany_odkaz() {
    let r = render("x.md", "# X\n\n[p](../../design/prirucka.html)\n", dev());
    assert_eq!(r.links, vec!["../design/prirucka.html".to_string()]);
    assert!(r.html.contains("href=\"../design/prirucka.html\""));
}

/// Docasna slozka s malou kopii `design/`.
fn sample_design(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xtask-docs-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("design/assets/logo")).unwrap();
    fs::create_dir_all(dir.join("design/tools")).unwrap();
    fs::write(dir.join("design/prirucka.html"), "p").unwrap();
    fs::write(dir.join("design/assets/logo/a.svg"), "s").unwrap();
    fs::write(dir.join("design/tools/build.mjs"), "t").unwrap();
    dir
}

#[test]
fn kontrola_odkazu_do_design() {
    let dir = sample_design("check");
    let page = |link: &str| render("x.md", &format!("# X\n\n[a]({link})\n"), dev());
    let pages = [
        page("../../design/prirucka.html"),
        page("../../design/assets/logo/a.svg#x"),
        page("../../design/chybi.html"),
        page("../../design/tools/build.mjs"),
        page("../../design/../docs/x.md"),
    ];
    let mut errors = Vec::new();
    check_design_links("cs", &pages, dev(), &dir.join("design"), &mut errors);
    assert_eq!(errors.len(), 3, "{errors:?}");
    assert!(errors[0].contains("chybi.html"));
    assert!(errors[1].contains("tools/build.mjs"));
    assert!(errors[2].contains(".."));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn kopie_design_vynecha_nastroje_a_smaze_stare_soubory() {
    let dir = sample_design("copy");
    let out = dir.join("out/design");
    fs::create_dir_all(&out).unwrap();
    fs::write(out.join("stary.html"), "x").unwrap();
    copy_design(&dir.join("design"), &out).unwrap();
    assert!(out.join("prirucka.html").is_file());
    assert!(out.join("assets/logo/a.svg").is_file());
    assert!(!out.join("tools").exists());
    assert!(!out.join("stary.html").exists());
    let _ = fs::remove_dir_all(&dir);
}

const ROZCESTNIK: &str = "# Titulek

### Zaciname / Getting started

| Cesky | English | Obsah |
|---|---|---|
| [Zaciname](cs/zaciname.md) | [Getting started](en/getting-started.md) | start / starting |

### Prace / Using

| Cesky | English | Obsah |
|---|---|---|
| [Okna](cs/okna.md) | [Windows](en/windows.md) | okna |
";

#[test]
fn rozcestnik_se_cte_se_skupinami_a_jmeny() {
    let pages = parse_index(ROZCESTNIK).unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].files, ["zaciname.md".to_string(), "getting-started.md".to_string()]);
    assert_eq!(pages[0].names[1], "Getting started");
    assert_eq!(pages[0].group, ["Zaciname".to_string(), "Getting started".to_string()]);
    assert_eq!(pages[1].group[0], "Prace");
    assert_eq!(pages[1].descriptions[0], "okna");
    assert!(parse_index("# nic
").is_none());
}

#[test]
fn vystupni_cesty_knih() {
    assert_eq!(guide().web_dir("cs"), "prirucka/cs");
    assert_eq!(dev().web_dir("en"), "en");
    assert_eq!(guide().root_prefix(), "../../");
    assert_eq!(dev().root_prefix(), "../");
    assert_eq!(guide().search_file("en"), "search-prirucka-en.js");
    assert_eq!(dev().search_file("cs"), "search-cs.js");
    // Vychozi (prvni) kniha je prirucka.
    assert_eq!(BOOKS[0].out_sub, "prirucka");
}

#[test]
fn design_odkaz_z_prirucky_se_prepise_o_dve_urovne() {
    assert_eq!(rewrite_link("../../../design/a.html", guide()), "../../design/a.html");
}

#[test]
fn odkazy_mezi_knihami_se_kontroluji() {
    let dev_page = render("a.md", "# A

## Kotva
", dev());
    let guide_page = render(
        "b.md",
        "# B

[ok](../../cs/a.md#kotva) [kotva](../../cs/a.md#chybi) [strana](../../cs/c.md) [index](../../cs/index.html)
",
        guide(),
    );
    let back = render("c.md", "# C

[ok](../prirucka/cs/b.md) [chybi](../prirucka/cs/z.md)
", dev());
    let mut known: HashMap<String, &HashSet<String>> = HashMap::new();
    known.insert("cs/a.html".into(), &dev_page.anchors);
    known.insert("cs/c.html".into(), &back.anchors);
    known.insert("prirucka/cs/b.html".into(), &guide_page.anchors);
    let mut errors = Vec::new();
    check_links("prirucka/cs", std::slice::from_ref(&guide_page), &known, &mut errors);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("kotvu"), "{errors:?}");
    let mut errors = Vec::new();
    check_links("cs", std::slice::from_ref(&back), &known, &mut errors);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("z.md") || errors[0].contains("z.html"), "{errors:?}");
}

#[test]
fn rozlozeni_cesty_odkazu() {
    assert_eq!(resolve_web("prirucka/cs", "../../cs/x.html").as_deref(), Some("cs/x.html"));
    assert_eq!(resolve_web("cs", "x.html").as_deref(), Some("cs/x.html"));
    assert_eq!(resolve_web("cs", "../../x.html"), None);
}

/// Stranka "Prikazy shellu" je vygenerovana z metadat a aktualni
/// (po zmene metadat: `cargo xtask docs`).
#[test]
fn stranka_prikazu_odpovida_metadatum() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for (en, file) in [(false, "docs/cs/prikazy-shellu.md"), (true, "docs/en/shell-commands.md")] {
        let on_disk = std::fs::read_to_string(root.join(file)).unwrap_or_default().replace("\r\n", "\n");
        assert!(on_disk == shlang::doc::markdown(en), "{file} je zastarala: spustte cargo xtask docs");
    }
}
