//! Testy: zpravy a jmena, vektory z RFC 6605 (ECDSA) a RFC 8080 (Ed25519,
//! s opravou errata 4935), validator nad testovaci hierarchii.

use std::string::String;
use std::vec;
use std::vec::Vec;

use crate::dnssec::chain::{Fetch, Security, Validator};
use crate::dnssec::{Dnskey, Ds, Rrsig, verify_rrset};
use crate::message::{CLASS_IN, Message, Record, rcode};
use crate::name::{self, Name};
use crate::rtype;
use crate::testzone::{Hierarchy, name};

fn b64(s: &str) -> Vec<u8> {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes().filter(|&c| c != b'=') {
        acc = acc << 6 | A.iter().position(|&x| x == c).unwrap() as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn rr(owner: &str, t: u16, rdata: Vec<u8>) -> Record {
    Record {
        name: name(owner),
        rtype: t,
        class: CLASS_IN,
        ttl: 3600,
        rdata,
    }
}

fn dnskey(owner: &str, flags: u16, algorithm: u8, key: &str) -> Dnskey {
    let mut rd = flags.to_be_bytes().to_vec();
    rd.push(3);
    rd.push(algorithm);
    rd.extend(b64(key));
    Dnskey::parse(&rr(owner, rtype::DNSKEY, rd)).unwrap()
}

#[allow(clippy::too_many_arguments)]
fn rrsig(owner: &str, covered: u16, algorithm: u8, labels: u8, exp: u32, inc: u32, tag: u16, signer: &str, sig: &str) -> Rrsig {
    let mut rd = covered.to_be_bytes().to_vec();
    rd.push(algorithm);
    rd.push(labels);
    rd.extend_from_slice(&3600u32.to_be_bytes());
    rd.extend_from_slice(&exp.to_be_bytes());
    rd.extend_from_slice(&inc.to_be_bytes());
    rd.extend_from_slice(&tag.to_be_bytes());
    name(signer).write(&mut rd, false);
    rd.extend(b64(sig));
    Rrsig::parse(&rr(owner, rtype::RRSIG, rd)).unwrap()
}

/// YYYYMMDDHHMMSS -> sekundy od 1970.
fn gt(s: &str) -> u32 {
    let n = |a: usize, b: usize| s[a..b].parse::<i64>().unwrap();
    let (y, m, d) = (n(0, 4), n(4, 6), n(6, 8));
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    (days * 86_400 + n(8, 10) * 3600 + n(10, 12) * 60 + n(12, 14)) as u32
}

#[test]
fn rfc6605_p256() {
    let k = dnskey("example.net.", 257, 13, "GojIhhXUN/u4v54ZQqGSnyhWJwaubCvTmeexv7bR6edbkrSqQpF64cYbcB7wNcP+e+MAnLr+Wi9xMWyQLc8NAA==");
    assert_eq!(k.key_tag(), 55648);
    let ds = Ds {
        key_tag: 55648,
        algorithm: 13,
        digest_type: 2,
        digest: hex("b4c8c1fe2e7477127b27115656ad6256f424625bf5c1e2770ce6d6e37df61d17"),
    };
    assert!(ds.matches(&k));
    let a = rr("www.example.net.", rtype::A, vec![192, 0, 2, 1]);
    let sig = rrsig(
        "www.example.net.",
        rtype::A,
        13,
        3,
        gt("20100909100439"),
        gt("20100812100439"),
        55648,
        "example.net.",
        "qx6wLYqmh+l9oCKTN6qIc+bw6ya+KJ8oMz0YP107epXAyGmt+3SNruPFKG7tZoLBLlUzGGus7ZwmwWep666VCw==",
    );
    let now = gt("20100820000000");
    assert_eq!(verify_rrset(&[&a], &sig, &k, now), Ok(None));
    // Jina adresa, propadly podpis.
    let b = rr("www.example.net.", rtype::A, vec![192, 0, 2, 2]);
    assert!(verify_rrset(&[&b], &sig, &k, now).is_err());
    assert!(verify_rrset(&[&a], &sig, &k, gt("20101001000000")).is_err());
}

#[test]
fn rfc6605_p384() {
    let k = dnskey(
        "example.net.",
        257,
        14,
        "xKYaNhWdGOfJ+nPrL8/arkwf2EY3MDJ+SErKivBVSum1w/egsXvSADtNJhyem5RCOpgQ6K8X1DRSEkrbYQ+OB+v8/uX45NBwY8rp65F6Glur8I/mlVNgF6W/qTI37m40",
    );
    assert_eq!(k.key_tag(), 10771);
    let ds = Ds {
        key_tag: 10771,
        algorithm: 14,
        digest_type: 4,
        digest: hex("72d7b62976ce06438e9c0bf319013cf801f09ecc84b8d7e9495f27e305c6a9b0563a9b5f4d288405c3008a946df983d6"),
    };
    assert!(ds.matches(&k));
    let a = rr("www.example.net.", rtype::A, vec![192, 0, 2, 1]);
    let sig = rrsig(
        "www.example.net.",
        rtype::A,
        14,
        3,
        gt("20100909102025"),
        gt("20100812102025"),
        10771,
        "example.net.",
        "/L5hDKIvGDyI1fcARX3z65qrmPsVz73QD1Mr5CEqOiLP95hxQouuroGCeZOvzFaxsT8Glr74hbavRKayJNuydCuzWTSSPdz7wnqXL5bdcJzusdnI0RSMROxxwGipWcJm",
    );
    assert_eq!(verify_rrset(&[&a], &sig, &k, gt("20100820000000")), Ok(None));
}

#[test]
fn rfc8080_ed25519() {
    // Erratum 4935: algoritmus 15 a 2 navesti v RRSIG.
    let mx = {
        let mut rd = 10u16.to_be_bytes().to_vec();
        name("mail.example.com.").write(&mut rd, false);
        rr("example.com.", rtype::MX, rd)
    };
    for (key, tag, ds, sig) in [
        (
            "l02Woi0iS8Aa25FQkUd9RMzZHJpBoRQwAQEX1SxZJA4=",
            3613,
            "3aa5ab37efce57f737fc1627013fee07bdf241bd10f3b1964ab55c78e79a304b",
            "oL9krJun7xfBOIWcGHi7mag5/hdZrKWw15jPGrHpjQeRAvTdszaPD+QLs3fx8A4M3e23mRZ9VrbpMngwcrqNAg==",
        ),
        (
            "zPnZ/QwEe7S8C5SPz2OfS5RR40ATk2/rYnE9xHIEijs=",
            35217,
            "401781b934e392de492ec77ae2e15d70f6575a1c0bc59c5275c04ebe80c6614c",
            "zXQ0bkYgQTEFyfLyi9QoiY6D8ZdYo4wyUhVioYZXFdT410QPRITQSqJSnzQoSm5poJ7gD7AQR0O7KuI5k2pcBg==",
        ),
    ] {
        let k = dnskey("example.com.", 257, 15, key);
        assert_eq!(k.key_tag(), tag);
        let d = Ds { key_tag: tag, algorithm: 15, digest_type: 2, digest: hex(ds) };
        assert!(d.matches(&k));
        let s = rrsig("example.com.", rtype::MX, 15, 2, 1_440_021_600, 1_438_207_200, tag, "example.com.", sig);
        assert_eq!(verify_rrset(&[&mx], &s, &k, 1_439_000_000), Ok(None), "klic {tag}");
    }
}

#[test]
fn names_and_messages() {
    let n = Name::parse("WWW.Example.COM.").unwrap();
    assert_eq!(n, Name::parse("www.example.com").unwrap());
    assert_eq!(n.to_wire(true), b"\x03www\x07example\x03com\x00");
    assert!(n.is_in(&name("example.com.")) && !name("example.com.").is_in(&n));
    // Kanonicke poradi (RFC 4034 6.1).
    let order = ["example.", "a.example.", "yljkjljk.a.example.", "Z.a.example.", "zABC.a.EXAMPLE.", "z.example.", "*.z.example."];
    let mut names: Vec<Name> = order.iter().map(|s| name(s)).collect();
    names.reverse();
    names.sort_by(|a, b| a.canonical_cmp(b));
    let sorted: Vec<String> = names.iter().map(Name::to_text).collect();
    assert_eq!(sorted, ["example", "a.example", "yljkjljk.a.example", "Z.a.example", "zABC.a.EXAMPLE", "z.example", "*.z.example"]);
    assert!(Name::parse(&"a.".repeat(130)).is_err());
    assert!(Name::parse("a..b").is_err());

    // Komprese: odpoved s ukazateli.
    let mut m = vec![0x12, 0x34, 0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0];
    m.extend(b"\x03www\x07example\x03com\x00\x00\x05\x00\x01");
    m.extend([0xC0, 12, 0, 5, 0, 1, 0, 0, 0, 60, 0, 6]);
    m.extend([3, b'f', b'o', b'o', 0xC0, 16]);
    let msg = Message::parse(&m).unwrap();
    assert_eq!(msg.answers[0].target().unwrap(), name("foo.example.com."));
    assert_eq!(msg.answers[0].rdata, b"\x03foo\x07example\x03com\x00");
    // Smycka ukazatelu, ukazatel dopredu, useknuta zprava, obri pocty.
    let mut loop_msg = m.clone();
    loop_msg[12] = 0xC0;
    loop_msg[13] = 12;
    assert!(Message::parse(&loop_msg).is_err());
    assert!(Message::parse(&m[..m.len() - 3]).is_err());
    let mut big = m.clone();
    big[6] = 0xFF;
    assert!(Message::parse(&big).is_err());
    assert!(name::read(&[0xC0, 5, 0, 0, 0, 0], 0).is_err());
    // Dotaz s DO a tam a zpet.
    let q = Message::query(7, &name("example.test."), rtype::A, true);
    let back = Message::parse(&q.to_wire()).unwrap();
    assert_eq!(back, q);
}

struct Local<'a>(&'a Hierarchy, usize);

impl Fetch for Local<'_> {
    fn query(&mut self, n: &Name, qtype: u16) -> Option<Message> {
        self.1 += 1;
        Some(self.0.ask(n, qtype))
    }
}

fn validator(h: &Hierarchy) -> Validator {
    Validator::new(vec![Ds::parse(&h.anchor()).unwrap()])
}

const NOW: u32 = 1_800_000_000;

fn check(h: &Hierarchy, v: &mut Validator, q: &str, t: u16) -> (Security, Message) {
    let resp = h.ask(&name(q), t);
    let mut f = Local(h, 0);
    (v.validate(&mut f, &name(q), t, &resp, NOW), resp)
}

#[test]
fn validator_secure_and_insecure() {
    let h = Hierarchy::standard();
    let mut v = validator(&h);
    for (q, t) in [
        ("www.example.test.", rtype::A),
        ("alias.example.test.", rtype::A),
        ("v6.example.test.", rtype::AAAA),
        ("host.test.", rtype::A),
        ("foo.wild.example.test.", rtype::A),
    ] {
        let (s, resp) = check(&h, &mut v, q, t);
        assert_eq!(s, Security::Secure, "{q}");
        assert!(!resp.answers.is_empty(), "{q}");
    }
    // Neexistence: NXDOMAIN (NSEC3 i NSEC), NODATA.
    for (q, t, code) in [
        ("nope.example.test.", rtype::A, rcode::NXDOMAIN),
        ("nope.test.", rtype::A, rcode::NXDOMAIN),
        ("www.example.test.", rtype::AAAA, rcode::NOERROR),
        ("host.test.", rtype::TXT, rcode::NOERROR),
        ("wild.example.test.", rtype::A, rcode::NOERROR),
    ] {
        let (s, resp) = check(&h, &mut v, q, t);
        assert_eq!((s, resp.rcode(), resp.answers.len()), (Security::Secure, code, 0), "{q}");
    }
    // Nepodepsana delegace: data bez overeni.
    let (s, resp) = check(&h, &mut v, "www.unsigned.test.", rtype::A);
    assert_eq!((s, resp.answers.len()), (Security::Insecure, 1));
    // DS jineho klice: podvrzene.
    let (s, _) = check(&h, &mut v, "www.bad.test.", rtype::A);
    assert!(matches!(s, Security::Bogus(_)), "{s:?}");
}

#[test]
fn validator_rejects_tampering() {
    let h = Hierarchy::standard();
    let mut v = validator(&h);
    let q = name("www.example.test.");
    let mut f = Local(&h, 0);
    // Zmenena adresa.
    let mut resp = h.ask(&q, rtype::A);
    resp.answers[0].rdata = vec![6, 6, 6, 6];
    assert!(matches!(v.validate(&mut f, &q, rtype::A, &resp, NOW), Security::Bogus(_)));
    // Odstraneny podpis.
    let mut resp = h.ask(&q, rtype::A);
    resp.answers.retain(|r| r.rtype != rtype::RRSIG);
    assert!(matches!(v.validate(&mut f, &q, rtype::A, &resp, NOW), Security::Bogus(_)));
    // Podvrzene NXDOMAIN pro existujici jmeno (bez dukazu).
    let mut resp = h.ask(&q, rtype::A);
    resp.answers.clear();
    resp.flags |= rcode::NXDOMAIN as u16;
    assert!(matches!(v.validate(&mut f, &q, rtype::A, &resp, NOW), Security::Bogus(_)));
    // Podvrzene NODATA s dukazy jineho jmena.
    let other = h.ask(&name("nope.example.test."), rtype::A);
    let mut resp = h.ask(&q, rtype::A);
    resp.answers.clear();
    resp.authority = other.authority;
    assert!(matches!(v.validate(&mut f, &q, rtype::A, &resp, NOW), Security::Bogus(_)));
    // Propadle podpisy (rok 2103).
    let resp = h.ask(&q, rtype::A);
    let mut v2 = validator(&h);
    assert_ne!(v2.validate(&mut f, &q, rtype::A, &resp, 4_200_000_000), Security::Secure);
    // Spatna kotva.
    let mut anchor = Ds::parse(&h.anchor()).unwrap();
    anchor.digest[0] ^= 1;
    let mut wrong = Validator::new(vec![anchor]);
    assert!(matches!(wrong.validate(&mut f, &q, rtype::A, &resp, NOW), Security::Bogus(_)));
}

#[test]
fn validator_caches_chain() {
    let h = Hierarchy::standard();
    let mut v = validator(&h);
    let q = name("www.example.test.");
    let resp = h.ask(&q, rtype::A);
    let mut f = Local(&h, 0);
    assert_eq!(v.validate(&mut f, &q, rtype::A, &resp, NOW), Security::Secure);
    let first = f.1;
    let mut f2 = Local(&h, 0);
    assert_eq!(v.validate(&mut f2, &q, rtype::A, &resp, NOW), Security::Secure);
    assert!(first > 0 && f2.1 == 0, "druhe overeni z mezipameti: {first} / {}", f2.1);
}
