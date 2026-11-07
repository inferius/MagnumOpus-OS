//! Testy generatoru dokumentace: prepis a kontrola odkazu do `design/`.

use super::*;

#[test]
fn odkaz_do_design_se_prepise_o_uroven_vys() {
    assert_eq!(rewrite_link("../../design/prirucka.html"), "../design/prirucka.html");
    assert_eq!(
        rewrite_link("../../design/okna.html#stavy"),
        "../design/okna.html#stavy"
    );
    // Ostatni odkazy se chovaji jako drive.
    assert_eq!(rewrite_link("vzhled.md#a"), "vzhled.html#a");
    assert_eq!(rewrite_link("https://x/../../design/a"), "https://x/../../design/a");
}

#[test]
fn render_sbira_prepsany_odkaz() {
    let r = render("x.md", "# X\n\n[p](../../design/prirucka.html)\n");
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
    let page = |link: &str| render("x.md", &format!("# X\n\n[a]({link})\n"));
    let pages = [
        page("../../design/prirucka.html"),
        page("../../design/assets/logo/a.svg#x"),
        page("../../design/chybi.html"),
        page("../../design/tools/build.mjs"),
        page("../../design/../docs/x.md"),
    ];
    let mut errors = Vec::new();
    check_design_links("cs", &pages, &dir.join("design"), &mut errors);
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
