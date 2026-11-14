use super::*;

fn chars() -> impl Iterator<Item = char> {
    (0..=0x10FFFFu32).filter_map(char::from_u32)
}

#[test]
fn version_matches_rust_core() {
    let (a, b, c) = char::UNICODE_VERSION;
    assert_eq!(UNICODE_VERSION, (a, b, c), "tabulky jsou z jine verze UCD nez core::char: cargo xtask unicode-tables");
}

#[test]
fn classes_match_rust_core() {
    for ch in chars() {
        let c = ch as u32;
        assert_eq!(is_alpha(c), ch.is_alphabetic(), "alpha U+{c:04X}");
        assert_eq!(is_lower(c), ch.is_lowercase(), "lower U+{c:04X}");
        assert_eq!(is_upper(c), ch.is_uppercase(), "upper U+{c:04X}");
        if ch.is_whitespace() && !matches!(c, 0xA0 | 0x2007 | 0x202F) {
            assert!(is_space(c), "space U+{c:04X}");
        }
        assert_eq!(is_cntrl(c) && c < 0x2000, ch.is_control(), "cntrl U+{c:04X}");
    }
}

#[test]
fn simple_case_matches_single_char_full_mapping() {
    for ch in chars() {
        let c = ch as u32;
        let mut up = ch.to_uppercase();
        if let (Some(u), None) = (up.next(), up.next()) {
            assert_eq!(to_upper(c), u as u32, "upper U+{c:04X}");
        }
        let mut lo = ch.to_lowercase();
        if let (Some(l), None) = (lo.next(), lo.next()) {
            assert_eq!(to_lower(c), l as u32, "lower U+{c:04X}");
        }
    }
}

#[test]
fn ascii_and_known_points() {
    assert!(is_alpha('z' as u32) && !is_alpha('1' as u32));
    assert!(is_digit('7' as u32) && !is_digit(0x0663), "arabsko-indicke cislice nejsou isdigit");
    assert!(is_xdigit('F' as u32) && !is_xdigit('g' as u32));
    assert!(!is_space(0xA0) && !is_space(0x2007) && is_space(0x2003) && is_space(0x3000));
    assert!(is_blank(0x09) && is_blank(0x3000) && !is_blank(0x0A));
    assert!(is_cntrl(0x7F) && is_cntrl(0x85) && is_cntrl(0x2028) && !is_cntrl('a' as u32));
    assert!(is_print(' ' as u32) && !is_graph(' ' as u32));
    assert!(is_punct('!' as u32) && is_punct(0x20AC) && !is_punct('a' as u32));
    assert!(is_print(0xE000), "soukrome znaky se tisknou");
    assert!(!is_print(0xD800) && !is_print(0x0378) && !is_print(0x110000));
    assert_eq!(to_upper('a' as u32), 'A' as u32);
    assert_eq!(to_upper(0x00FF), 0x0178);
    assert_eq!(to_lower(0x0130), 0x0069, "I s teckou: jednoduchy prevod");
    assert_eq!(to_upper(0x00DF), 0x00DF, "ostre s nema jednoduchy prevod");
    assert_eq!(to_upper(0x010D), 0x010C, "c s hackem");
    assert_eq!(to_lower(0x0158), 0x0159, "R s hackem");
    assert_eq!(to_upper(0x110000), 0x110000);
}

#[test]
fn widths() {
    assert_eq!(width(0), 0);
    assert_eq!(width('a' as u32), 1);
    assert_eq!(width('\n' as u32), -1);
    assert_eq!(width(0x0301), 0, "kombinujici carka");
    assert_eq!(width(0x00AD), 1, "mekky rozdelovnik");
    assert_eq!(width(0x200B), 0);
    assert_eq!(width(0x1160), 0, "samohlaska hangul");
    assert_eq!(width(0x4E00), 2);
    assert_eq!(width(0x3000), 2);
    assert_eq!(width(0x1F600), 2, "emoji");
    assert_eq!(width(0x20000), 2, "CJK rozsireni B");
    assert_eq!(width(0x2A6DF), 2, "CJK rozsireni B, posledni");
    assert_eq!(width(0x3FFFD), -1, "neprirazene je netisknutelne");
    assert_eq!(width(0x010D), 1);
    assert_eq!(width(0xD800), -1);
}
