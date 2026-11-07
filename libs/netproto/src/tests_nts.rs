//! Testy NTS (RFC 8915): zaznamy NTS-KE, pozadavek a odpoved NTP s AES-SIV,
//! odmitnuti podvrzenych a cizich odpovedi.

use alloc::vec;
use alloc::vec::Vec;

use crate::nts::{self, Error, record, server};
use crate::ntp;
use abi::time::WallTime;

const MASTER: [u8; 32] = [7; 32];
const C2S: [u8; 32] = [1; 32];
const S2C: [u8; 32] = [2; 32];
const UID: [u8; 32] = [9; 32];

fn t(secs: i64) -> WallTime {
    WallTime { secs, nanos: 0 }
}

#[test]
fn ke_records() {
    let req = nts::ke_request();
    assert!(server::ke_request_complete(&req));
    assert!(!server::ke_request_complete(&req[..req.len() - 2]));
    // Kriticke END, NTPv4, AES-SIV-CMAC-256.
    assert_eq!(&req[..6], &[0x80, 0x01, 0, 2, 0, 0]);
    assert_eq!(&req[6..12], &[0, 4, 0, 2, 0, 15]);
    assert_eq!(&req[12..], &[0x80, 0, 0, 0]);

    let cookies = vec![vec![1u8; 96], vec![2u8; 96]];
    let reply = server::ke_reply(&cookies, Some(1123));
    let r = nts::parse_ke_reply(&reply).unwrap().unwrap();
    assert_eq!((r.cookies, r.port, r.server), (cookies, Some(1123), None));
    // Neuplna odpoved = cekat na dalsi data.
    assert_eq!(nts::parse_ke_reply(&reply[..reply.len() - 1]), Ok(None));

    let rec = |kind: u16, body: &[u8]| {
        let mut v = Vec::new();
        v.extend_from_slice(&kind.to_be_bytes());
        v.extend_from_slice(&(body.len() as u16).to_be_bytes());
        v.extend_from_slice(body);
        v
    };
    let end = rec(record::CRITICAL, &[]);
    let proto = rec(record::CRITICAL | record::NEXT_PROTOCOL, &[0, 0]);
    let aead = rec(record::AEAD, &[0, 15]);
    let cookie = rec(record::COOKIE, &[5; 8]);
    let ok = [proto.clone(), aead.clone(), cookie.clone(), end.clone()].concat();
    assert!(nts::parse_ke_reply(&ok).unwrap().is_some());
    // Chyba serveru, chybejici cookie, jiny AEAD, neznamy kriticky zaznam.
    assert_eq!(nts::parse_ke_reply(&[rec(record::CRITICAL | record::ERROR, &[0, 1]), end.clone()].concat()), Err(Error::Server(1)));
    assert!(nts::parse_ke_reply(&[proto.clone(), aead.clone(), end.clone()].concat()).is_err());
    assert!(nts::parse_ke_reply(&[proto.clone(), rec(record::AEAD, &[0, 16]), cookie.clone(), end.clone()].concat()).is_err());
    assert!(nts::parse_ke_reply(&[proto.clone(), aead.clone(), cookie.clone(), rec(record::CRITICAL | 99, &[]), end.clone()].concat()).is_err());
    // Neznamy nekriticky zaznam se preskoci.
    assert!(nts::parse_ke_reply(&[proto, aead, cookie, rec(99, &[1, 2]), end].concat()).unwrap().is_some());
    assert_eq!(nts::key_context(true), [0, 0, 0, 15, 1]);
}

/// Pozadavek klienta a odpoved serveru s novymi cookies.
fn exchange(cookie: &[u8], placeholders: usize) -> (Vec<u8>, Vec<u8>) {
    let req = nts::build_request(t(1000), &UID, cookie, placeholders, &C2S, &[3; 16]);
    let v = server::verify_request(&MASTER, &req).expect("server prijme pozadavek");
    assert_eq!((v.uid.as_slice(), v.s2c, v.placeholders), (&UID[..], S2C, placeholders));
    let header = ntp::build_reply(&req, t(1001), t(1002)).unwrap();
    let fresh: Vec<Vec<u8>> = (0..=placeholders).map(|i| server::make_cookie(&MASTER, &[i as u8; 16], &C2S, &S2C)).collect();
    (req, server::reply(&header, &v, &fresh, &[4; 16]))
}

#[test]
fn ntp_request_reply() {
    let cookie = server::make_cookie(&MASTER, &[0; 16], &C2S, &S2C);
    assert_eq!(cookie.len() % 4, 0);
    assert_eq!(server::open_cookie(&MASTER, &cookie), Some((C2S, S2C)));
    let (req, reply) = exchange(&cookie, 2);
    assert_eq!(req.len() % 4, 0);
    let r = nts::parse_reply(t(1000), &UID, &S2C, &reply).unwrap();
    assert_eq!(r.ntp.receive, t(1001));
    assert_eq!(r.cookies.len(), 3);
    assert!(r.cookies.iter().all(|c| server::open_cookie(&MASTER, c) == Some((C2S, S2C))));
}

#[test]
fn ntp_rejects() {
    let cookie = server::make_cookie(&MASTER, &[0; 16], &C2S, &S2C);
    let (req, reply) = exchange(&cookie, 0);
    // Spatny klic, cizi identifikator, cizi cas odeslani.
    assert_eq!(nts::parse_reply(t(1000), &UID, &C2S, &reply), Err(Error::Unauthenticated));
    assert_eq!(nts::parse_reply(t(1000), &[8; 32], &S2C, &reply), Err(Error::NotOurs));
    assert_eq!(nts::parse_reply(t(999), &UID, &S2C, &reply), Err(Error::NotOurs));
    // Zmeneny bajt hlavicky (cas serveru) i sifrovane casti.
    for at in [33, reply.len() - 3] {
        let mut bad = reply.clone();
        bad[at] ^= 1;
        assert_eq!(nts::parse_reply(t(1000), &UID, &S2C, &bad), Err(Error::Unauthenticated), "bajt {at}");
    }
    // Odpoved bez autentizatoru (utocnik ho odstrani).
    let header = ntp::build_reply(&req, t(1001), t(1002)).unwrap();
    let mut stripped = Vec::from(&header[..]);
    stripped.extend_from_slice(&reply[48..48 + 36]);
    assert_eq!(nts::parse_reply(t(1000), &UID, &S2C, &stripped), Err(Error::Unauthenticated));
    // NTSN = nove NTS-KE.
    assert_eq!(nts::parse_reply(t(1000), &UID, &S2C, &server::nak(&req, &UID)), Err(Error::CookieRejected));
    // Server odmitne cizi cookie a zmeneny pozadavek.
    let foreign = server::make_cookie(&[6; 32], &[0; 16], &C2S, &S2C);
    assert!(server::verify_request(&MASTER, &nts::build_request(t(1), &UID, &foreign, 0, &C2S, &[3; 16])).is_none());
    let mut tampered = req.clone();
    tampered[45] ^= 1;
    assert!(server::verify_request(&MASTER, &tampered).is_none());
}
