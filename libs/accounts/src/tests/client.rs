//! Klice klienta SSH: vytvoreni a import (heslo), odemceni, zmena hesla
//! klice prebali, novy heslo od spravy je zahodi, potvrzene servery,
//! zaznam formatu 3 (a cteni formatu 2).

use accountproto::ctap::soft::SeedRng;
use accountproto::Secret;
use abi::Error;

use crate::engine::{Config, Engine, MemStore};
use crate::kdf::KdfParams;
use crate::record::Account;
use crate::FileKey;

type E = Engine<MemStore, FileKey, SeedRng>;
const PW: &str = "spravne heslo";
const PW2: &str = "nove heslo 123";

fn alice() -> (E, u64) {
    let mut e = Engine::new(MemStore::default(), FileKey::new([5; 32]), SeedRng::new(42), Config { params: KdfParams::TEST, dev: false }).unwrap();
    let a = e.prepare_first("alice", "Alice", &Secret::from(PW), 0).unwrap();
    let uid = e.commit(a).unwrap();
    (e, uid)
}

#[test]
fn vytvoreni_import_a_odemceni() {
    let (mut e, uid) = alice();
    assert_eq!(e.client_key_create(uid, &Secret::from("spatne"), "x", None, 10).err(), Some(Error::AccessDenied));
    let (id, pk) = e.client_key_create(uid, &Secret::from(PW), "alice@mo", None, 10).unwrap();
    // Import: seed ze souboru, stejny klic podruhe ne.
    let seed = Secret::new(vec![7; 32]);
    let (id2, pk2) = e.client_key_create(uid, &Secret::from(PW), "import", Some(&seed), 11).unwrap();
    assert_eq!(pk2.key, ssh::keys::ed25519_public(&[7; 32]).key);
    assert_eq!(e.client_key_create(uid, &Secret::from(PW), "znovu", Some(&seed), 12).err(), Some(Error::AlreadyExists));
    let list = e.client_keys(uid).unwrap();
    assert_eq!(list.iter().map(|k| k.id).collect::<Vec<_>>(), vec![id, id2]);
    assert_eq!(list[0].public.key, pk.key);
    assert_eq!(list[0].fingerprint, pk.fingerprint());
    // Odemceni: seedy sedi s verejnymi klici; spatne heslo nic.
    let seeds = e.client_unlock(uid, &Secret::from(PW), 20).unwrap();
    assert_eq!(seeds.len(), 2);
    for (i, s) in &seeds {
        let seed: [u8; 32] = s.expose().try_into().unwrap();
        let k = list.iter().find(|k| k.id == *i).unwrap();
        assert_eq!(ssh::keys::ed25519_public(&seed).key, k.public.key);
    }
    assert_eq!(e.client_unlock(uid, &Secret::from("spatne"), 21).err(), Some(Error::AccessDenied));
    // Soukrome casti nejsou v zaznamu nezabalene.
    let rec = e.account(uid).unwrap().encode();
    assert!(!rec.windows(32).any(|w| w == [7u8; 32]));
    e.client_key_remove(uid, id2).unwrap();
    assert_eq!(e.client_keys(uid).unwrap().len(), 1);
    assert_eq!(e.client_key_remove(uid, id2).err(), Some(Error::NotFound));
}

#[test]
fn zmena_hesla_prebali_reset_zahodi() {
    let (mut e, uid) = alice();
    let (_, pk) = e.client_key_create(uid, &Secret::from(PW), "k", None, 10).unwrap();
    e.set_password(uid, &Secret::from(PW), &Secret::from(PW2), 30).unwrap();
    assert_eq!(e.client_unlock(uid, &Secret::from(PW), 40).err(), Some(Error::AccessDenied));
    let seeds = e.client_unlock(uid, &Secret::from(PW2), 41).unwrap();
    let seed: [u8; 32] = seeds[0].1.expose().try_into().unwrap();
    assert_eq!(ssh::keys::ed25519_public(&seed).key, pk.key);
    // Nove heslo od spravy: stare se nezna, klice nejdou otevrit.
    e.reset_password(uid, &Secret::from("heslo od spravy")).unwrap();
    assert!(e.client_keys(uid).unwrap().is_empty());
}

#[test]
fn potvrzene_servery() {
    let (mut e, uid) = alice();
    let (id, _) = e.client_key_create(uid, &Secret::from(PW), "k", None, 10).unwrap();
    assert!(!e.client_host_known(uid, id, "server.lan"));
    e.client_host_approve(uid, id, "server.lan").unwrap();
    assert!(e.client_host_known(uid, id, "SERVER.lan"));
    assert!(!e.client_host_known(uid, id, "jiny"));
    assert_eq!(e.client_host_approve(uid, id, "s mezerou").err(), Some(Error::InvalidArgs));
    assert_eq!(e.client_host_approve(uid, 99, "x").err(), Some(Error::NotFound));
}

#[test]
fn zaznam_formatu_3_a_2() {
    let (mut e, uid) = alice();
    e.client_key_create(uid, &Secret::from(PW), "k", None, 10).unwrap();
    let id = e.client_keys(uid).unwrap()[0].id;
    e.client_host_approve(uid, id, "[host]:2222").unwrap();
    let a = e.account(uid).unwrap().clone();
    assert_eq!(Account::decode(&a.encode()).unwrap(), a);
    // Starsi format 2 se cte (bez klicu klienta).
    let old = Account::decode(&a.encode_format(2)).unwrap();
    assert!(old.client_keys.is_empty());
    assert_eq!(old.ssh_keys, a.ssh_keys);
}
