//! Vzdaleny pristup: klice SSH uctu (heslo pri pridani, otisk, limity),
//! povoleni pristupu, prihlaseni klicem (podpis overeny tady, jmeno
//! a klic v podepsanych datech, opakovani relace, zpomaleni na dalku
//! oddelene od mistniho).

use accountproto::ctap::soft::SeedRng;
use accountproto::{Method, Secret, flags};
use abi::Error;
use alloc::vec::Vec;
use ed25519_dalek::{Signer, SigningKey};
use ssh::keys::PublicKey;

use crate::engine::{Config, Engine, MemStore};
use crate::kdf::KdfParams;
use crate::record::Account;
use crate::remote::{RemoteAuth, RemoteGuard};
use crate::{FileKey, Outcome};

type E = Engine<MemStore, FileKey, SeedRng>;
const PW: &str = "spravne heslo";
const ADDR: &str = "10.0.2.2";

fn alice() -> (E, u64) {
    let mut e = Engine::new(MemStore::default(), FileKey::new([5; 32]), SeedRng::new(42), Config { params: KdfParams::TEST, dev: false }).unwrap();
    let a = e.prepare_first("alice", "Alice", &Secret::from(PW), 0).unwrap();
    let uid = e.commit(a).unwrap();
    (e, uid)
}

fn key(seed: u8) -> (SigningKey, PublicKey) {
    let s = SigningKey::from_bytes(&[seed; 32]);
    let k = PublicKey { algo: ssh::keys::Algo::Ed25519, key: s.verifying_key().to_bytes(), comment: "alice@pc".into() };
    (s, k)
}

fn put(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(&(s.len() as u32).to_be_bytes());
    out.extend_from_slice(s);
}

/// Podepsana data (RFC 4252, 7) a podpis.
fn signed(s: &SigningKey, k: &PublicKey, session: &[u8], user: &str) -> (Vec<u8>, Vec<u8>) {
    let mut d = Vec::new();
    put(&mut d, session);
    d.push(50);
    put(&mut d, user.as_bytes());
    put(&mut d, b"ssh-connection");
    put(&mut d, b"publickey");
    d.push(1);
    put(&mut d, b"ssh-ed25519");
    put(&mut d, &k.blob());
    let mut sig = Vec::new();
    put(&mut sig, b"ssh-ed25519");
    put(&mut sig, &s.sign(&d).to_bytes());
    (d, sig)
}

#[test]
fn klice_ssh_s_heslem_a_otiskem() {
    let (mut e, uid) = alice();
    let (_, k) = key(1);
    let line = k.line();
    assert_eq!(e.ssh_key_add(uid, &Secret::from("spatne heslo!"), &line, 1).err(), Some(Error::AccessDenied));
    let (id, fp) = e.ssh_key_add(uid, &Secret::from(PW), &line, 1).unwrap();
    assert_eq!(fp, k.fingerprint());
    assert_eq!(e.ssh_key_add(uid, &Secret::from(PW), &line, 1).err(), Some(Error::AlreadyExists));
    assert_eq!(e.ssh_key_add(uid, &Secret::from(PW), "ssh-rsa AAAA x", 1).err(), Some(Error::InvalidArgs));
    let list = e.ssh_keys(uid).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!((list[0].id, list[0].algo.as_str(), list[0].comment.as_str()), (id, "ssh-ed25519", "alice@pc"));
    // Zaznam na disku: klice prezijou ulozeni, verze 1 se cte bez nich.
    let a = e.account(uid).unwrap().clone();
    assert_eq!(Account::decode(&a.encode()).unwrap(), a);
    let old = Account::decode(&a.encode_format(1)).unwrap();
    assert!(old.ssh_keys.is_empty() && old.name == "alice");
    e.ssh_key_remove(uid, id).unwrap();
    assert_eq!(e.ssh_key_remove(uid, id), Err(Error::NotFound));
    assert!(e.ssh_keys(uid).unwrap().is_empty());
}

#[test]
fn prihlaseni_klicem_jen_s_povolenim() {
    let (mut e, uid) = alice();
    let (s, k) = key(2);
    e.ssh_key_add(uid, &Secret::from(PW), &k.line(), 1).unwrap();
    let mut g = RemoteGuard::new();
    let blob = k.blob();
    let (d, sig) = signed(&s, &k, b"relace-1", "alice");
    // Vzdaleny pristup vypnuty (vychozi): ani spravny klic neprojde.
    assert!(!e.ssh_probe(&g, "alice", ADDR, &blob, 2));
    assert!(matches!(e.ssh_auth(&mut g, "alice", ADDR, &blob, &d, &sig, 2), RemoteAuth::Denied { .. }));
    // Zapnout chce heslo.
    assert_eq!(e.set_remote(uid, &Secret::from("spatne heslo!"), true, 3), Err(Error::AccessDenied));
    e.set_remote(uid, &Secret::from(PW), true, 3).unwrap();
    assert!(e.account(uid).unwrap().flags & flags::REMOTE != 0);
    let mut g = RemoteGuard::new();
    assert!(e.ssh_probe(&g, "alice", ADDR, &blob, 4));
    let (d, sig) = signed(&s, &k, b"relace-2", "alice");
    assert_eq!(e.ssh_auth(&mut g, "alice", ADDR, &blob, &d, &sig, 4), RemoteAuth::Ok { uid });
    // Stejna relace podruhe (opakovani podpisu): ne.
    assert!(matches!(e.ssh_auth(&mut g, "alice", ADDR, &blob, &d, &sig, 4), RemoteAuth::Denied { .. }));
    // Podepsana data pro jineho uzivatele nebo jiny klic: ne.
    let (d2, sig2) = signed(&s, &k, b"relace-3", "bob");
    assert!(matches!(e.ssh_auth(&mut g, "alice", ADDR, &blob, &d2, &sig2, 5), RemoteAuth::Denied { .. }));
    let (s3, k3) = key(3);
    let (d3, sig3) = signed(&s3, &k3, b"relace-4", "alice");
    assert!(matches!(e.ssh_auth(&mut g, "alice", ADDR, &k3.blob(), &d3, &sig3, 5), RemoteAuth::Denied { .. }));
    assert!(!e.ssh_probe(&g, "alice", ADDR, &k3.blob(), 5));
    // Vypnuti bez hesla.
    e.set_remote(uid, &Secret::from(""), false, 6).unwrap();
    assert!(!e.ssh_probe(&RemoteGuard::new(), "alice", ADDR, &blob, 6));
}

#[test]
fn zpomaleni_na_dalku_neblokuje_mistni() {
    let (mut e, uid) = alice();
    let (s, k) = key(4);
    e.ssh_key_add(uid, &Secret::from(PW), &k.line(), 1).unwrap();
    e.set_remote(uid, &Secret::from(PW), true, 1).unwrap();
    let mut g = RemoteGuard::new();
    let (_, bad) = key(5);
    let mut last = RemoteAuth::Ok { uid: 0 };
    for i in 0..3u8 {
        let (d, sig) = signed(&s, &bad, &[b'x', i], "alice");
        last = e.ssh_auth(&mut g, "alice", ADDR, &bad.blob(), &d, &sig, 10);
    }
    // Po 3 neuspesich ceka adresa i jmeno.
    assert_eq!(last, RemoteAuth::Denied { retry_after_ms: 30_000 });
    let (d, sig) = signed(&s, &k, b"ok-1", "alice");
    assert_eq!(e.ssh_auth(&mut g, "alice", ADDR, &k.blob(), &d, &sig, 11), RemoteAuth::Denied { retry_after_ms: 29_000 });
    // Jina adresa na stejne jmeno taky ceka (rozlozeny utok).
    assert!(matches!(e.ssh_auth(&mut g, "alice", "10.0.2.3", &k.blob(), &d, &sig, 11), RemoteAuth::Denied { .. }));
    // Mistni prihlaseni heslem bez cekani (pocitadla oddelena).
    assert!(matches!(e.authenticate(uid, Method::Password, &Secret::from(PW), None, 12).unwrap(), Outcome::Ok { .. }));
    // Po uplynuti prodlevy projde a pocitadla se vynuluji.
    let (d, sig) = signed(&s, &k, b"ok-2", "alice");
    assert_eq!(e.ssh_auth(&mut g, "alice", ADDR, &k.blob(), &d, &sig, 41), RemoteAuth::Ok { uid });
    // Neznamy ucet: stejne odmitnuti jako cizi klic.
    let (d, sig) = signed(&s, &k, b"ok-3", "mallory");
    assert!(matches!(e.ssh_auth(&mut g, "mallory", ADDR, &k.blob(), &d, &sig, 42), RemoteAuth::Denied { .. }));
}
