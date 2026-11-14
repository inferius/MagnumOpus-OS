//! Testy `scanf` na hostiteli.

use crate::scanf::{Sink, scan_bytes};

#[derive(Default)]
struct Rec {
    ints: Vec<(u8, i64)>,
    floats: Vec<f64>,
    strs: Vec<Vec<u8>>,
}

impl Sink for Rec {
    fn int(&mut self, len: u8, v: i64) {
        self.ints.push((len, v));
    }
    fn float(&mut self, _len: u8, v: f64) {
        self.floats.push(v);
    }
    fn bytes(&mut self, b: &[u8], _nul: bool) {
        self.strs.push(b.to_vec());
    }
    fn ptr(&mut self, v: usize) {
        self.ints.push((9, v as i64));
    }
}

#[test]
fn numbers_strings_sets() {
    let mut r = Rec::default();
    let n = scan_bytes(b"  42 -0x1f 017 3.5e2 slovo", b"%d %i %o %lf %s", &mut r);
    assert_eq!(n, 5);
    assert_eq!(r.ints, vec![(0, 42), (0, -31), (0, 15)]);
    assert_eq!(r.floats, vec![350.0]);
    assert_eq!(r.strs, vec![b"slovo".to_vec()]);

    let mut r = Rec::default();
    assert_eq!(scan_bytes(b"key=value;rest", b"%[^=]=%[a-z];%3c", &mut r), 3);
    assert_eq!(r.strs, vec![b"key".to_vec(), b"value".to_vec(), b"res".to_vec()]);

    let mut r = Rec::default();
    assert_eq!(scan_bytes(b"12abc", b"%2d%*s%n", &mut r), 1, "%* a %n se nepocitaji");
    assert_eq!(r.ints, vec![(0, 12), (0, 5)]);

    let mut r = Rec::default();
    assert_eq!(scan_bytes(b"", b"%d", &mut r), -1, "prazdny vstup = EOF");
    assert_eq!(scan_bytes(b"x", b"%d", &mut r), 0, "neshoda = 0");
    assert_eq!(scan_bytes(b"10:20", b"%d:%lld", &mut r), 2);
}
