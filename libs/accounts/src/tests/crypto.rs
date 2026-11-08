//! Argon2id podle testovacich vektoru, obaly, obnovovaci klic, format
//! zaznamu, tabulka zpomaleni.

use alloc::vec;

use accountproto::Secret;
use accountproto::ctap::soft::SeedRng;
use argon2::{Algorithm, Argon2, AssociatedData, ParamsBuilder, Version};

use crate::kdf::{self, KdfParams};
use crate::record::{Account, Meta};
use crate::throttle;
use crate::wrap::{self, kind};

fn hex(s: &str) -> vec::Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

#[test]
fn argon2id_rfc9106_kap_5_3() {
    // RFC 9106, 5.3: m=32 KiB, t=3, p=4, heslo 32x 0x01, sul 16x 0x02,
    // tajemstvi 8x 0x03, pridana data 12x 0x04, vystup 32 B.
    let params = ParamsBuilder::new()
        .m_cost(32)
        .t_cost(3)
        .p_cost(4)
        .data(AssociatedData::new(&[4u8; 12]).unwrap())
        .output_len(32)
        .build()
        .unwrap();
    let a = Argon2::new_with_secret(&[3u8; 8], Algorithm::Argon2id, Version::V0x13, params).unwrap();
    let mut out = [0u8; 32];
    a.hash_password_into(&[1u8; 32], &[2u8; 16], &mut out).unwrap();
    assert_eq!(out.to_vec(), hex("0d640df58d78766c08c037a34a8b53c9d01ef0452d75b65eb52520e96b01e659"));
}

#[test]
fn argon2id_referencni_vektor() {
    // Referencni implementace (test.c): argon2id v19, t=2, m=64 MiB, p=1,
    // "password", "somesalt".
    let p = argon2::Params::new(1 << 16, 2, 1, Some(32)).unwrap();
    let a = Argon2::new(Algorithm::Argon2id, Version::V0x13, p);
    let mut out = [0u8; 32];
    a.hash_password_into(b"password", b"somesalt", &mut out).unwrap();
    assert_eq!(out.to_vec(), hex("09316115d5cf24ed5a15a31a3ba326e5cf32edc24702987c02b6566f61913cf7"));
}

#[test]
fn derive_deli_vystup_na_dve_poloviny() {
    let d1 = kdf::derive(&KdfParams::TEST, b"heslo", &[1; 16]).unwrap();
    let d2 = kdf::derive(&KdfParams::TEST, b"heslo", &[1; 16]).unwrap();
    let d3 = kdf::derive(&KdfParams::TEST, b"heslo", &[2; 16]).unwrap();
    assert_eq!(d1.verifier(), d2.verifier());
    assert_ne!(d1.verifier(), d1.wrap_material());
    assert_ne!(d1.verifier(), d3.verifier());
    assert!(KdfParams::QEMU.weaker_than(&KdfParams::HW));
    assert!(!KdfParams::HW.weaker_than(&KdfParams::QEMU));
}

#[test]
fn obal_klice_domova() {
    let mut rng = SeedRng::new(1);
    let home = wrap::new_home_key(&mut rng);
    let w = wrap::wrap(&mut rng, 1000, kind::PASSWORD, 0, &[7; 32], &home).unwrap();
    assert_eq!(wrap::unwrap(1000, &w, &[7; 32]).unwrap(), home);
    // Spatne tajemstvi, jiny uzivatel, jiny druh: nic.
    assert!(wrap::unwrap(1000, &w, &[8; 32]).is_err());
    assert!(wrap::unwrap(1001, &w, &[7; 32]).is_err());
    let mut w2 = w.clone();
    w2.kind = kind::PIN;
    assert!(wrap::unwrap(1000, &w2, &[7; 32]).is_err());
    let mut w3 = w.clone();
    w3.ct[0] ^= 1;
    assert!(wrap::unwrap(1000, &w3, &[7; 32]).is_err());
}

#[test]
fn obnovovaci_klic() {
    let mut rng = SeedRng::new(2);
    let (raw, text) = wrap::new_recovery(&mut rng);
    let t = core::str::from_utf8(text.expose()).unwrap();
    assert_eq!(t.len(), 35 + 6, "7 skupin po 5 znacich s pomlckami: {t}");
    assert_eq!(t.split('-').count(), 7);
    assert_eq!(wrap::parse_recovery(text.expose()).unwrap(), raw);
    // Mala pismena, mezery misto pomlcek, O misto 0.
    let loose: alloc::string::String = t.to_lowercase().replace('-', " ").replace('0', "o");
    assert_eq!(wrap::parse_recovery(loose.as_bytes()).unwrap(), raw);
    // Preklep: jiny znak.
    let mut typo = text.expose().to_vec();
    typo[3] = if typo[3] == b'A' { b'B' } else { b'A' };
    assert!(wrap::parse_recovery(&typo).is_none());
    assert!(wrap::parse_recovery(b"").is_none());
    assert!(wrap::parse_recovery(b"heslo").is_none());
}

#[test]
fn zaznam_tam_a_zpet_a_poskozeni() {
    let mut a = Account::new(1000, "alice", "Alice Nováková", 5);
    a.phrase = "modrá konvice".into();
    a.password = Some(kdf::Verifier { params: KdfParams::QEMU, salt: [1; 16], value: [2; 32] });
    a.keys.push(crate::record::KeyRec {
        id: 1,
        name: "YubiKey".into(),
        cred_id: vec![1, 2, 3],
        public: vec![4, 5],
        sign_count: 7,
        uv: true,
        hmac_secret: true,
    });
    let mut rng = SeedRng::new(3);
    let home = Secret::new(vec![9; 32]);
    a.crypt = Some(crate::record::Crypt { wraps: vec![wrap::wrap(&mut rng, 1000, kind::PASSWORD, 0, &[1; 32], &home).unwrap()] });
    let b = a.encode();
    assert_eq!(Account::decode(&b).unwrap(), a);
    for i in [0usize, 5, b.len() / 2, b.len() - 1] {
        let mut c = b.clone();
        c[i] ^= 0x40;
        assert_eq!(Account::decode(&c), Err(abi::Error::Corrupted), "bajt {i}");
    }
    assert!(Account::decode(&b[..b.len() - 1]).is_err());
    let m = Meta { next_uid: 1005, next_gid: 1_000_003 };
    assert_eq!(Meta::decode(&m.encode()).unwrap(), m);
    // Parametry Argon2id mimo meze = poskozeny zaznam (ne gigabajty pameti).
    let mut w = wire::Writer::new();
    KdfParams { m_kib: 1 << 30, t: 1, p: 1 }.put(&mut w);
    assert!(KdfParams::get(&mut wire::Reader::new(&w.finish())).is_err());
}

#[test]
fn tabulka_zpomaleni() {
    let expect = [(0, 0), (1, 0), (2, 0), (3, 30), (4, 60), (5, 120), (6, 300), (7, 900), (50, 900)];
    for (f, s) in expect {
        assert_eq!(throttle::delay_secs(f), s, "{f} chyb");
    }
    assert_eq!(throttle::wait_ms(3, 100, 100), 30_000);
    assert_eq!(throttle::wait_ms(3, 100, 110), 20_000);
    assert_eq!(throttle::wait_ms(3, 100, 200), 0);
    // Hodiny pozpatku: cekat celou dobu.
    assert_eq!(throttle::wait_ms(3, 100, 50), 30_000);
}
