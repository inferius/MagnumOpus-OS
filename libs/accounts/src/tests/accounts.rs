//! Zakladani uctu, prihlaseni vsemi metodami, zpomaleni, PIN, klice,
//! sifrovani domova (Engine nad MemStore a simulovanym klicem).

use accountproto::ctap::soft::{SeedRng, SoftKey};
use accountproto::{Method, Secret, flags};
use abi::Error;

use crate::engine::{Config, Engine, MemStore};
use crate::kdf::KdfParams;
use crate::throttle::PIN_ATTEMPTS;
use crate::{FileKey, Outcome};

type E = Engine<MemStore, FileKey, SeedRng>;

fn engine_with(store: MemStore, dev: bool) -> E {
    Engine::new(store, FileKey::new([5; 32]), SeedRng::new(42), Config { params: KdfParams::TEST, dev }).unwrap()
}

fn engine() -> E {
    engine_with(MemStore::default(), false)
}

const PW: &str = "spravne heslo";

fn alice(e: &mut E) -> u64 {
    let a = e.prepare_first("alice", "Alice", &Secret::from(PW), 0).unwrap();
    e.commit(a).unwrap()
}

fn ok(o: Outcome) -> Option<Secret> {
    match o {
        Outcome::Ok { home_key, .. } => home_key,
        other => panic!("ocekavan uspech, je {other:?}"),
    }
}

#[test]
fn prvni_ucet_ma_roli_a_jen_jednou() {
    let mut e = engine();
    let uid = alice(&mut e);
    assert_eq!(uid, 1000);
    let a = e.account(uid).unwrap();
    assert!(a.flags & flags::ADMIN != 0);
    assert_eq!(a.home, "/home/alice");
    assert_eq!(a.groups, vec![accountproto::ids::GROUP_USERS]);
    assert_eq!(e.prepare_first("bob", "Bob", &Secret::from(PW), 0).err(), Some(Error::AccessDenied));
    // Dalsi ucet (ukon spravy) ma dalsi cislo; jmeno jedinecne.
    let b = e.prepare("bob", "Bob", Some(&Secret::from(PW)), false, 0).unwrap();
    assert_eq!(e.commit(b).unwrap(), 1001);
    assert_eq!(e.prepare("alice", "A", Some(&Secret::from(PW)), false, 0).err(), Some(Error::AlreadyExists));
    assert_eq!(e.prepare("Bad Name", "A", Some(&Secret::from(PW)), false, 0).err(), Some(Error::InvalidArgs));
    assert_eq!(e.prepare("carol", "C", Some(&Secret::from("kratke")), false, 0).err(), Some(Error::InvalidArgs));
    // Posledni roli odebrat ani ucet s ni smazat nejde.
    assert_eq!(e.set_admin(1000, false), Err(Error::AccessDenied));
    assert_eq!(e.delete(1000).err(), Some(Error::AccessDenied));
    e.set_admin(1001, true).unwrap();
    e.set_admin(1000, false).unwrap();
}

#[test]
fn cislo_smazaneho_uctu_se_nepouzije_znovu() {
    let mut store = MemStore::default();
    {
        let mut e = engine_with(core::mem::take(&mut store), false);
        alice(&mut e);
        let b = e.prepare("bob", "Bob", Some(&Secret::from(PW)), false, 0).unwrap();
        e.commit(b).unwrap();
        e.delete(1001).unwrap();
        store = core::mem::take(&mut e.store);
    }
    let mut e = engine_with(store, false);
    let c = e.prepare("carol", "Carol", Some(&Secret::from(PW)), false, 0).unwrap();
    assert_eq!(c.uid, 1002);
}

#[test]
fn heslo_spravne_a_spatne() {
    let mut e = engine();
    let uid = alice(&mut e);
    assert!(ok(e.authenticate(uid, Method::Password, &Secret::from(PW), None, 10).unwrap()).is_none());
    // Heslo v jinem tvaru Unicode (NFD) je stejne heslo.
    let a = e.prepare("zofie", "Zofie", Some(&Secret::from("žluťoučký kůň")), false, 0).unwrap();
    let z = e.commit(a).unwrap();
    let nfd = "z\u{30c}lut\u{30c}ouc\u{30c}ky\u{301} ku\u{30a}n\u{30c}";
    assert!(e.authenticate(z, Method::Password, &Secret::from(nfd), None, 10).unwrap().is_ok());
    match e.authenticate(uid, Method::Password, &Secret::from("spatne heslo"), None, 10).unwrap() {
        Outcome::Denied { retry_after_ms: 0, .. } => {}
        o => panic!("{o:?}"),
    }
    // Neznamy ucet i ucet bez metody: odmitnuto stejne.
    assert!(matches!(e.authenticate(5555, Method::Password, &Secret::from(PW), None, 10).unwrap(), Outcome::Denied { .. }));
    assert!(matches!(e.authenticate(uid, Method::Pin, &Secret::from("1234"), None, 10).unwrap(), Outcome::Denied { .. }));
    assert!(matches!(e.authenticate(uid, Method::Dev, &Secret::default(), None, 10).unwrap(), Outcome::Denied { .. }));
}

#[test]
fn zpomaleni_po_chybach_a_ulozeni_pred_overenim() {
    let mut e = engine();
    let uid = alice(&mut e);
    let bad = Secret::from("spatne heslo");
    for _ in 0..2 {
        assert!(matches!(e.authenticate(uid, Method::Password, &bad, None, 100).unwrap(), Outcome::Denied { retry_after_ms: 0, .. }));
    }
    // 3. chyba: 30 s.
    assert!(matches!(e.authenticate(uid, Method::Password, &bad, None, 100).unwrap(), Outcome::Denied { retry_after_ms: 30_000, .. }));
    // V dobe cekani se nic nepocita (ani spravne heslo).
    let saves = e.store.saves;
    assert!(matches!(e.authenticate(uid, Method::Password, &Secret::from(PW), None, 110).unwrap(), Outcome::LockedOut { retry_after_ms: 20_000 }));
    assert_eq!(e.store.saves, saves, "pokus v cekani nic neulozil");
    // Po cekani: 4. chyba = 60 s.
    assert!(matches!(e.authenticate(uid, Method::Password, &bad, None, 130).unwrap(), Outcome::Denied { retry_after_ms: 60_000, .. }));
    // Pocitadlo prezije restart sluzby (je v DB).
    let store = core::mem::take(&mut e.store);
    let mut e = engine_with(store, false);
    assert!(matches!(e.authenticate(uid, Method::Password, &Secret::from(PW), None, 140).unwrap(), Outcome::LockedOut { .. }));
    ok(e.authenticate(uid, Method::Password, &Secret::from(PW), None, 200).unwrap());
    assert_eq!(e.account(uid).unwrap().failures, 0);
}

#[test]
fn pocitadlo_se_ulozi_pred_overenim() {
    // Zapis selze: overeni se vubec neprovede (jinak by slo hadat bez
    // pocitani pri plnem disku).
    let mut e = engine();
    let uid = alice(&mut e);
    e.store.fail = true;
    assert_eq!(e.authenticate(uid, Method::Password, &Secret::from(PW), None, 1).err(), Some(Error::IoError));
}

#[test]
fn pin_s_klicem_zarizeni_a_blokace() {
    let mut e = engine();
    let uid = alice(&mut e);
    assert_eq!(e.set_pin(uid, &Secret::from(PW), &Secret::from("12a4"), 0), Err(Error::InvalidArgs));
    assert_eq!(e.set_pin(uid, &Secret::from("spatne heslo"), &Secret::from("1234"), 0), Err(Error::AccessDenied));
    e.set_pin(uid, &Secret::from(PW), &Secret::from("1234"), 0).unwrap();
    assert!(e.list()[0].flags & flags::PIN_WEAK != 0, "bez TPM varovat");
    ok(e.authenticate(uid, Method::Pin, &Secret::from("1234"), None, 1000).unwrap());
    // PIN zavisi na klici zarizeni: s jinym klicem zarizeni neprojde.
    let store = MemStore { files: e.store.files.clone(), meta: e.store.meta, ..Default::default() };
    let mut other = Engine::new(store, FileKey::new([6; 32]), SeedRng::new(1), Config { params: KdfParams::TEST, dev: false }).unwrap();
    assert!(matches!(other.authenticate(uid, Method::Pin, &Secret::from("1234"), None, 1000).unwrap(), Outcome::Denied { .. }));
    // 5 spatnych PINu = PIN zablokovany (cas mezi pokusy dost dlouhy).
    let mut t = 2000;
    for left in (0..PIN_ATTEMPTS).rev() {
        match e.authenticate(uid, Method::Pin, &Secret::from("0000"), None, t).unwrap() {
            Outcome::Denied { pin_left, .. } => assert_eq!(pin_left, left),
            o => panic!("{o:?}"),
        }
        t += 1000;
    }
    assert!(matches!(e.authenticate(uid, Method::Pin, &Secret::from("1234"), None, t).unwrap(), Outcome::Denied { .. }));
    // Heslo PIN odblokuje.
    ok(e.authenticate(uid, Method::Password, &Secret::from(PW), None, t + 1000).unwrap());
    ok(e.authenticate(uid, Method::Pin, &Secret::from("1234"), None, t + 1001).unwrap());
    // Smazani PINu.
    e.set_pin(uid, &Secret::from(PW), &Secret::default(), t + 2000).unwrap();
    assert!(e.account(uid).unwrap().pin.is_none());
}

#[test]
fn zmena_hesla() {
    let mut e = engine();
    let uid = alice(&mut e);
    assert_eq!(e.set_password(uid, &Secret::from("spatne heslo"), &Secret::from("nove heslo 1"), 0), Err(Error::AccessDenied));
    assert_eq!(e.set_password(uid, &Secret::from(PW), &Secret::from("kratke"), 0), Err(Error::InvalidArgs));
    e.set_password(uid, &Secret::from(PW), &Secret::from("nove heslo 1"), 0).unwrap();
    assert!(matches!(e.authenticate(uid, Method::Password, &Secret::from(PW), None, 1).unwrap(), Outcome::Denied { .. }));
    ok(e.authenticate(uid, Method::Password, &Secret::from("nove heslo 1"), None, 2).unwrap());
}

#[test]
fn prepocet_slabsich_parametru() {
    let mut e = engine();
    let uid = alice(&mut e);
    e.cfg.params = KdfParams { m_kib: 128, t: 2, p: 1 };
    ok(e.authenticate(uid, Method::Password, &Secret::from(PW), None, 1).unwrap());
    assert_eq!(e.account(uid).unwrap().password.as_ref().unwrap().params, e.cfg.params);
    ok(e.authenticate(uid, Method::Password, &Secret::from(PW), None, 2).unwrap());
}

#[test]
fn vyvojove_prihlaseni() {
    let mut e = engine();
    assert_eq!(e.prepare_dev(0).err(), Some(Error::AccessDenied));
    let mut e = engine_with(MemStore::default(), true);
    let d = e.prepare_dev(0).unwrap().unwrap();
    let uid = e.commit(d).unwrap();
    assert!(e.prepare_dev(0).unwrap().is_none());
    assert!(!e.has_users(), "vyvojovy ucet neni ucet cloveka");
    ok(e.authenticate(uid, Method::Dev, &Secret::default(), None, 0).unwrap());
    assert!(matches!(e.authenticate(uid, Method::Password, &Secret::from(""), None, 0).unwrap(), Outcome::Denied { .. }));
    assert_eq!(e.set_admin(uid, true), Err(Error::AccessDenied));
    // Stejna DB bez povoleni vyvoje: vyvojovy ucet se neprihlasi ani neukaze.
    let store = core::mem::take(&mut e.store);
    let mut e = engine_with(store, false);
    assert!(e.list().is_empty());
    assert!(matches!(e.authenticate(uid, Method::Dev, &Secret::default(), None, 0).unwrap(), Outcome::Denied { .. }));
}

#[test]
fn bezpecnostni_klic_s_pinem() {
    let mut e = engine();
    let uid = alice(&mut e);
    let mut key = SoftKey::with_pin(7, "2580");
    // Klic s PINem bez PINu klice registrovat nejde.
    assert_eq!(e.enroll_key(uid, &Secret::from(PW), &Secret::default(), "YubiKey", &mut key, 0), Err(Error::AccessDenied));
    let id = e.enroll_key(uid, &Secret::from(PW), &Secret::from("2580"), "YubiKey", &mut key, 0).unwrap();
    assert!(e.account(uid).unwrap().keys[0].uv);
    assert!(e.list()[0].has(Method::Key));
    ok(e.authenticate(uid, Method::Key, &Secret::from("2580"), Some(&mut key), 10).unwrap());
    // Bez PINu klice (jen dotek): klic registrovany s UV to odmitne.
    assert!(matches!(e.authenticate(uid, Method::Key, &Secret::default(), Some(&mut key), 11).unwrap(), Outcome::Denied { .. }));
    // Spatny PIN klice.
    assert!(matches!(e.authenticate(uid, Method::Key, &Secret::from("1111"), Some(&mut key), 12).unwrap(), Outcome::Denied { .. }));
    // Bez doteku.
    key.touch = false;
    assert!(matches!(e.authenticate(uid, Method::Key, &Secret::from("2580"), Some(&mut key), 100).unwrap(), Outcome::Denied { .. }));
    key.touch = true;
    // Jiny klic (neregistrovany).
    let mut other = SoftKey::with_pin(8, "2580");
    assert!(matches!(e.authenticate(uid, Method::Key, &Secret::from("2580"), Some(&mut other), 1000).unwrap(), Outcome::Denied { .. }));
    // Opakovana registrace stejneho klice.
    assert_eq!(e.enroll_key(uid, &Secret::from(PW), &Secret::from("2580"), "znovu", &mut key, 2000), Err(Error::AlreadyExists));
    e.remove_key(uid, &Secret::from(PW), id, 3000).unwrap();
    assert!(e.account(uid).unwrap().keys.is_empty());
}

#[test]
fn bezpecnostni_klic_bez_pinu_a_klon() {
    let mut e = engine();
    let uid = alice(&mut e);
    let mut key = SoftKey::new(9);
    e.enroll_key(uid, &Secret::from(PW), &Secret::default(), "SoloKey", &mut key, 0).unwrap();
    ok(e.authenticate(uid, Method::Key, &Secret::default(), Some(&mut key), 1).unwrap());
    // Pocitadlo podpisu neroste = mozny klon klice.
    key.frozen_counter = true;
    assert!(matches!(e.authenticate(uid, Method::Key, &Secret::default(), Some(&mut key), 2).unwrap(), Outcome::Denied { .. }));
}

#[test]
fn sifrovani_domova_vsemi_metodami() {
    let mut e = engine();
    let uid = alice(&mut e);
    let p = e.prepare_crypt(uid, &Secret::from(PW), 0).unwrap();
    let home = p.home_key.clone();
    let recovery = p.recovery.clone();
    e.commit_crypt(uid, p.crypt).unwrap();
    assert_eq!(e.prepare_crypt(uid, &Secret::from(PW), 0).err(), Some(Error::AlreadyExists));
    assert!(e.list()[0].flags & flags::ENCRYPTED != 0);
    // Heslo rozbali klic domova.
    assert_eq!(ok(e.authenticate(uid, Method::Password, &Secret::from(PW), None, 1).unwrap()), Some(home.clone()));
    // PIN a klic dostanou obal pri nastaveni.
    e.set_pin(uid, &Secret::from(PW), &Secret::from("1234"), 2).unwrap();
    assert_eq!(ok(e.authenticate(uid, Method::Pin, &Secret::from("1234"), None, 3).unwrap()), Some(home.clone()));
    let mut key = SoftKey::with_pin(10, "2580");
    e.enroll_key(uid, &Secret::from(PW), &Secret::from("2580"), "klic", &mut key, 4).unwrap();
    assert_eq!(ok(e.authenticate(uid, Method::Key, &Secret::from("2580"), Some(&mut key), 5).unwrap()), Some(home.clone()));
    // Zapomenute heslo: obnovovaci klic prihlasi a dovoli nastavit nove.
    assert!(e.list()[0].has(Method::Recovery));
    assert_eq!(ok(e.authenticate(uid, Method::Recovery, &recovery, None, 6).unwrap()), Some(home.clone()));
    e.set_password(uid, &recovery, &Secret::from("uplne nove heslo"), 7).unwrap();
    assert_eq!(ok(e.authenticate(uid, Method::Password, &Secret::from("uplne nove heslo"), None, 8).unwrap()), Some(home.clone()));
    // Spatny obnovovaci klic.
    assert!(matches!(e.authenticate(uid, Method::Recovery, &Secret::from("AAAAA-AAAAA"), None, 9).unwrap(), Outcome::Denied { .. }));
}

#[test]
fn spatne_heslo_neprozradi_klic_domova() {
    let mut e = engine();
    let uid = alice(&mut e);
    let p = e.prepare_crypt(uid, &Secret::from(PW), 0).unwrap();
    e.commit_crypt(uid, p.crypt).unwrap();
    assert!(matches!(e.authenticate(uid, Method::Password, &Secret::from("spatne heslo"), None, 1).unwrap(), Outcome::Denied { .. }));
    // Zaznam na disku neobsahuje klic domova v otevrenem tvaru.
    let home = ok(e.authenticate(uid, Method::Password, &Secret::from(PW), None, 100).unwrap()).unwrap();
    let file = &e.store.files[0].1;
    assert!(!file.windows(32).any(|w| w == home.expose()));
}

#[test]
fn poskozeny_zaznam_neshodi_ostatni() {
    let mut e = engine();
    let uid = alice(&mut e);
    let b = e.prepare("bob", "Bob", Some(&Secret::from(PW)), false, 0).unwrap();
    e.commit(b).unwrap();
    let mut store = core::mem::take(&mut e.store);
    store.files[1].1[10] ^= 1;
    let mut e = engine_with(store, false);
    assert_eq!(e.broken.len(), 1);
    assert_eq!(e.accounts().len(), 1);
    ok(e.authenticate(uid, Method::Password, &Secret::from(PW), None, 1).unwrap());
    // Cislo poskozeneho uctu se nepouzije (metadata to drzi).
    let c = e.prepare("carol", "Carol", Some(&Secret::from(PW)), false, 0).unwrap();
    assert_eq!(c.uid, 1002);
}

#[test]
fn fraze() {
    let mut e = engine();
    let uid = alice(&mut e);
    e.set_phrase(uid, "modrá konvice").unwrap();
    assert_eq!(e.account(uid).unwrap().phrase, "modrá konvice");
    assert_eq!(e.set_phrase(uid, "a\nb"), Err(Error::InvalidArgs));
}
