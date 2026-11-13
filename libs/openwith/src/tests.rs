//! Testy asociaci: `cargo test -p openwith`.

use alloc::vec;

use crate::{Registry, Target, extension, parse};

const FILES: &str = r#"
[app]
id = "org.magnumopus.files"
name = "Soubory"

[open]
action = "files.show"
folders = true
reveal = true

[[action]]
id = "files.show"
title = "Ukazat"

[[action.param]]
name = "path"
type = "path"
required = true
"#;

const EDITOR: &str = r#"
[app]
id = "org.example.editor"
name = "Editor"

[open]
action = "editor.open"
extensions = ["TXT", ".md"]

[[action]]
id = "editor.open"
title = "Otevrit"

[[action.param]]
name = "path"
type = "path"
required = true
"#;

#[test]
fn parses_open_section() {
    let a = parse("editor", EDITOR).unwrap().unwrap();
    assert_eq!(a.app, "org.example.editor");
    assert_eq!(a.extensions, vec!["txt", "md"]);
    assert!(!a.folders && !a.reveal);
    assert_eq!(parse("demo", "[app]\nid = \"x\"\n").unwrap(), None, "bez [open]");
}

#[test]
fn rejects_undeclared_or_wrong_action() {
    let no_action = FILES.replace("action = \"files.show\"\nfolders", "action = \"files.nic\"\nfolders");
    assert!(parse("gui-files", &no_action).unwrap_err().contains("neni deklarovana"));
    let text_param = FILES.replace("type = \"path\"", "type = \"text\"");
    assert!(parse("gui-files", &text_param).unwrap_err().contains("parametr path"));
    let optional = FILES.replace("required = true", "required = false");
    assert!(parse("gui-files", &optional).is_err());
}

#[test]
fn resolve_folder_extension_and_reveal() {
    let (r, errors) = Registry::build([("gui-files", FILES), ("editor", EDITOR), ("demo", "[app]\nid = \"d\"\n")]);
    assert!(errors.is_empty());
    assert!(matches!(r.resolve("/home/a/Dokumenty", true), Some(Target::Open(a)) if a.program == "gui-files"));
    assert!(matches!(r.resolve("/home/a/Poznamky.TXT", false), Some(Target::Open(a)) if a.program == "editor"));
    assert!(matches!(r.resolve("/home/a/obr.bmp", false), Some(Target::Reveal(a)) if a.program == "gui-files"));
    assert!(matches!(r.resolve("/home/a/.profile", false), Some(Target::Reveal(_))), "jmeno s teckou na zacatku nema priponu");
    let (only_editor, _) = Registry::build([("editor", EDITOR)]);
    assert_eq!(only_editor.resolve("/a/b.bmp", false), None, "bez spravce souboru nic");
    assert_eq!(only_editor.resolve("/a", true), None);
    let (_, errors) = Registry::build([("vadny", "[open]\naction = \"x\"\n")]);
    assert_eq!(errors.len(), 1, "vadny manifest jako chyba");
}

#[test]
fn extensions() {
    assert_eq!(extension("/a/b.tar.GZ"), "gz");
    assert_eq!(extension("/a/.bashrc"), "");
    assert_eq!(extension("/a.b/c"), "");
    assert_eq!(extension("x."), "");
}

#[test]
fn messages() {
    use crate::msg::{Reply, parse_request, request};
    assert_eq!(parse_request(&request("/home/a/b c.txt")), Some("/home/a/b c.txt"));
    assert_eq!(parse_request(b"open relativni"), None);
    assert_eq!(parse_request(b"jine /a"), None);
    for r in [Reply::Opened("org.x".into()), Reply::Revealed("org.y".into()), Reply::Failed("cesta neexistuje".into())] {
        assert_eq!(Reply::decode(&r.encode()), Some(r));
    }
    assert_eq!(Reply::decode(b"nesmysl"), None);
}
