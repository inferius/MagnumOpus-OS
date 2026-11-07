//! Testy s vektory z RFC 8439 (ChaCha20) a RFC 7693 (BLAKE2s).

extern crate std;

use std::vec::Vec;

use crate::{Blake2s, Generator, SEED_BITS, chacha20_block};

fn hex(s: &str) -> Vec<u8> {
    let s: std::string::String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn chacha20_rfc8439_block() {
    // RFC 8439, 2.3.2.
    let key: [u8; 32] = core::array::from_fn(|i| i as u8);
    let nonce: [u8; 12] = hex("000000090000004a00000000").try_into().unwrap();
    let out = chacha20_block(&key, 1, &nonce);
    let expected = hex(
        "10f1e7e4d13b5915500fdd1fa32071c4c7d1f4c733c068030422aa9ac3d46c4e
         d2826446079faa0914c2d705d98b02a2b5129cd1de164eb9cbd083e8a2503c4e",
    );
    assert_eq!(out.as_slice(), expected.as_slice());
}

#[test]
fn blake2s_rfc7693_abc() {
    // RFC 7693, priloha B.
    let mut h = Blake2s::new();
    h.update(b"abc");
    let expected = hex("508c5e8c327c14e2e1a72ba34eeb452f37458b209ed63a294d999b4c86675982");
    assert_eq!(h.finalize().as_slice(), expected.as_slice());
}

#[test]
fn blake2s_empty_and_chunked() {
    let expected_empty = hex("69217a3079908094e11121d042354a7c1f55b6482ca1a51e1b250dfd1ed0eef9");
    assert_eq!(
        Blake2s::new().finalize().as_slice(),
        expected_empty.as_slice()
    );
    // Po castech = najednou, i pres hranice bloku 64 B.
    let data: Vec<u8> = (0..300u32).map(|i| i as u8).collect();
    let mut a = Blake2s::new();
    a.update(&data);
    let mut b = Blake2s::new();
    for chunk in data.chunks(7) {
        b.update(chunk);
    }
    assert_eq!(a.finalize(), b.finalize());
    // Presne 64 B: posledni blok se komprimuje s priznakem `last`.
    let mut c = Blake2s::new();
    c.update(&data[..64]);
    let mut d = Blake2s::new();
    d.update(&data[..32]);
    d.update(&data[32..64]);
    assert_eq!(c.finalize(), d.finalize());
}

#[test]
fn generator_seeding_and_key_erasure() {
    let mut g = Generator::new();
    assert!(!g.is_seeded());
    g.add(&[1; 16], 64);
    g.reseed();
    assert!(!g.is_seeded(), "64 bitu nestaci");
    // Kredit nejvys 8 bitu na bajt.
    g.add(&[2; 8], 1000);
    assert_eq!(g.pending_bits(), 64);
    g.add(&[3; 32], SEED_BITS);
    g.reseed();
    assert!(g.is_seeded());
    assert_eq!(g.pending_bits(), 0);

    let mut a = [0u8; 100];
    let mut b = [0u8; 100];
    g.fill(&mut a);
    g.fill(&mut b);
    assert_ne!(a, b, "po kazdem pozadavku novy klic");
    assert!(a.iter().any(|&x| x != 0));
}

#[test]
fn generator_inputs_change_output() {
    let mut g1 = Generator::new();
    let mut g2 = Generator::new();
    g1.add(b"zdroj", 0);
    g2.add(b"jiny zdroj", 0);
    g1.reseed();
    g2.reseed();
    let (mut a, mut b) = ([0u8; 32], [0u8; 32]);
    g1.fill(&mut a);
    g2.fill(&mut b);
    assert_ne!(a, b);
    // Stejne vstupy = stejny vystup (deterministicke, zadny skryty stav).
    let mut g3 = Generator::new();
    g3.add(b"zdroj", 0);
    g3.reseed();
    let mut c = [0u8; 32];
    g3.fill(&mut c);
    assert_eq!(a, c);
}
