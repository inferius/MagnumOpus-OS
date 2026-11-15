use std::collections::BTreeMap;

use super::*;

#[test]
fn unicode_data_ranges_and_case() {
    let text = "0041;LATIN CAPITAL LETTER A;Lu;0;L;;;;;N;;;;0061;\n\
                0061;LATIN SMALL LETTER A;Ll;0;L;;;;;N;;;0041;;0041\n\
                3400;<CJK Ideograph Extension A, First>;Lo;0;L;;;;;N;;;;;\n\
                4DBF;<CJK Ideograph Extension A, Last>;Lo;0;L;;;;;N;;;;;\n";
    let ud = parse_unicode_data(text).unwrap();
    assert_eq!(&ud.gc[0x3400], b"Lo");
    assert_eq!(&ud.gc[0x4000], b"Lo");
    assert_eq!(&ud.gc[0x4DC0], b"Cn");
    assert_eq!(ud.lower.get(&0x41), Some(&0x61));
    assert_eq!(ud.upper.get(&0x61), Some(&0x41));
}

#[test]
fn property_and_wide_defaults() {
    let p = parse_property("0041..0043    ; Alphabetic # x\n0044 ; Other\n", "Alphabetic").unwrap();
    assert!(p[0x41] && p[0x43] && !p[0x44]);
    let w = parse_wide("# @missing: 3400..4DBF; W\n3400..3401;N  # x\n4E00;W\n").unwrap();
    assert!(!w[0x3400] && w[0x3402] && w[0x4E00] && !w[0x41]);
}

#[test]
fn runs_merge_stride_one_and_two() {
    let mut m = BTreeMap::new();
    for c in 0x61..=0x7A {
        m.insert(c, c - 32);
    }
    for c in (0x101..=0x105).step_by(2) {
        m.insert(c, c - 1);
    }
    m.insert(0xFF, 0x178);
    assert_eq!(case_runs(&m), vec![(0x61, 0x7A, 1, -32), (0xFF, 0xFF, 1, 121), (0x101, 0x105, 2, -1)]);
    assert_eq!(ranges(&[false, true, true, false, true]), vec![(1, 2), (4, 4)]);
}
