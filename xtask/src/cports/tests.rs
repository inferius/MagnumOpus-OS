use super::configure::expand;
use super::tools::Tool;
use super::{Build, PORTS};

#[test]
fn configure_placeholders() {
    let vars = [("prefix", String::from("/p")), ("cc", String::from("clang")), ("host", String::from("x86_64-unknown-magnumopus"))];
    assert_eq!(expand("--prefix={prefix}", &vars), "--prefix=/p");
    assert_eq!(expand("--cc={cc} --host={host}", &vars), "--cc=clang --host=x86_64-unknown-magnumopus");
    // Nezname zastupne nechat (chybu ohlasi skript, ne tiche smazani).
    assert_eq!(expand("{nezname}", &vars), "{nezname}");
}

#[test]
fn every_tool_has_advice() {
    for t in [Tool::CMake, Tool::Ninja, Tool::Sh, Tool::Make] {
        assert!(!t.advice().is_empty());
    }
}

#[test]
fn ports_are_pinned() {
    for p in PORTS {
        assert_eq!(p.sha256.len(), 64, "{}: SHA-256", p.name);
        assert!(p.url.starts_with("https://"), "{}: jen HTTPS", p.name);
        assert!(p.url.ends_with(p.file), "{}: jmeno souboru z URL", p.name);
        if let Build::CMake { options, .. } = &p.build {
            assert!(options.iter().all(|o| o.starts_with("-D")), "{}: volby CMake", p.name);
        }
    }
}
