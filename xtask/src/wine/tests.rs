use super::wsl::{TOOLS, parse_version};
use super::{WINE, source_key};

#[test]
fn wine_pinned() {
    assert_eq!(WINE.sha256.len(), 64);
    assert!(WINE.url.starts_with("https://dl.winehq.org/") && WINE.url.ends_with(WINE.file));
    assert!(WINE.file.starts_with(WINE.top));
}

#[test]
fn llvm_version_parse() {
    assert_eq!(parse_version("Ubuntu clang version 23.1.6 (++2026...)"), Some((23, 1)));
    assert_eq!(parse_version("Ubuntu LLD 23.1.6 (compatible with GNU linkers)"), Some((23, 1)));
    assert_eq!(parse_version("bez cisla"), None);
    assert!(TOOLS.iter().any(|t| t.name == "clang-23" && t.min == Some((23, 1))));
}

#[test]
fn key_changes_with_patches() {
    let a = source_key("abc", &[(String::from("p"), b"1".to_vec())]);
    let b = source_key("abc", &[(String::from("p"), b"2".to_vec())]);
    assert_ne!(a, b);
    assert_eq!(a, source_key("abc", &[(String::from("p"), b"1".to_vec())]));
}
