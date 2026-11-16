//! Testy protokolu uctu a CTAP2 na hostiteli.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::ctap::cbor::{self, Value, int_map};
use crate::ctap::client::{Client, CtapError};
use crate::ctap::hid::{self, Assembler, cmd};
use crate::ctap::hidclient::HidTransport;
use crate::ctap::msg::{AuthData, flag, status};
use crate::ctap::pin::{self, Protocol, Shared};
use crate::ctap::soft::{SeedRng, SoftHid, SoftKey};
use crate::*;

// --- zpravy accountd ---

fn roundtrip(r: Request) {
    let b = r.encode();
    assert_eq!(Request::decode(&b).unwrap(), r);
    // Kazde zkraceni je chyba, ne panika.
    for n in 0..b.len() {
        assert!(Request::decode(&b[..n]).is_err(), "{r:?} zkraceno na {n}");
    }
}

#[test]
fn pozadavky_tam_a_zpet() {
    roundtrip(Request::List);
    roundtrip(Request::Names);
    roundtrip(Request::Phrase { uid: 1000 });
    roundtrip(Request::Auth { uid: 1000, method: Method::Password, secret: Secret::from("heslo") });
    roundtrip(Request::Unlock { session: 7, method: Method::Pin, secret: Secret::from("1234") });
    roundtrip(Request::LockSession { session: 3 });
    roundtrip(Request::WhoAmI);
    roundtrip(Request::SetPassword { old: Secret::from("a"), new: Secret::from("b") });
    roundtrip(Request::SetPin { password: Secret::from("a"), pin: Secret::default() });
    roundtrip(Request::EnrollKey { password: Secret::from("a"), key_pin: Secret::from("0000"), name: String::from("YubiKey") });
    roundtrip(Request::RemoveKey { password: Secret::from("a"), id: 3 });
    roundtrip(Request::SetPhrase { phrase: String::from("modra konvice") });
    roundtrip(Request::SshKeyAdd { password: Secret::from("a"), line: String::from("ssh-ed25519 AAAA alice@pc") });
    roundtrip(Request::SshKeyList);
    roundtrip(Request::SshKeyRemove { id: 2 });
    roundtrip(Request::SetRemote { password: Secret::from("a"), on: true });
    roundtrip(Request::SshProbe { user: String::from("alice"), addr: String::from("10.0.2.2"), key: vec![1, 2, 3] });
    roundtrip(Request::SshAuth {
        user: String::from("alice"),
        addr: String::from("10.0.2.2"),
        key: vec![1, 2, 3],
        data: vec![4; 40],
        sig: vec![5; 83],
    });
    assert!(Request::SshKeyAdd { password: Secret::default(), line: String::new() }.is_self());
    assert!(Request::SetRemote { password: Secret::default(), on: false }.is_self());
    assert!(Request::SshAuth { user: String::new(), addr: String::new(), key: vec![], data: vec![], sig: vec![] }.is_ssh());
    assert!(!Request::SshAuth { user: String::new(), addr: String::new(), key: vec![], data: vec![], sig: vec![] }.is_self());
    roundtrip(Request::EncryptHome { password: Secret::from("a") });
    roundtrip(Request::Lock);
    roundtrip(Request::Logout);
    roundtrip(Request::Elevate { action: 1, params: vec![1, 2, 3] });
    roundtrip(Request::SetKeyPin { pin: Secret::from("123456") });
    roundtrip(Request::CreateFirst { name: String::from("alice"), display: String::from("Alice"), password: Secret::from("x") });
}

#[test]
fn neplatne_zpravy() {
    assert_eq!(Request::decode(&[]), Err(Error::InvalidArgs));
    assert_eq!(Request::decode(&999u32.to_le_bytes()), Err(Error::NotSupported));
    // Neznama metoda.
    let mut b = Request::Auth { uid: 1, method: Method::Password, secret: Secret::default() }.encode();
    b[12] = 9;
    assert_eq!(Method::from_u8(5), Ok(Method::Recovery));
    assert_eq!(Request::decode(&b), Err(Error::InvalidArgs));
    // Prilis dlouhe tajemstvi.
    let long = Request::Auth { uid: 1, method: Method::Password, secret: Secret::new(vec![b'a'; MAX_SECRET + 1]) };
    assert_eq!(Request::decode(&long.encode()), Err(Error::InvalidArgs));
    // Data navic.
    let mut b = Request::List.encode();
    b.push(0);
    assert_eq!(Request::decode(&b), Err(Error::InvalidArgs));
}

#[test]
fn tajemstvi_se_nevypise() {
    let s = Secret::from("tajne-heslo");
    assert_eq!(alloc::format!("{s:?} {s}"), "*** ***");
    let r = Request::Auth { uid: 1, method: Method::Password, secret: s };
    assert!(!alloc::format!("{r:?}").contains("tajne"));
}

#[test]
fn kanaly_operaci() {
    assert!(Request::List.is_login());
    assert!(!Request::List.is_self());
    assert!(Request::WhoAmI.is_self());
    assert!(Request::SetKeyPin { pin: Secret::default() }.is_self());
    assert!(!Request::CreateFirst { name: String::new(), display: String::new(), password: Secret::default() }.is_self());
}

#[test]
fn odpovedi() {
    for a in [
        AuthReply::Ok { session: 5 },
        AuthReply::Denied { retry_after_ms: 30_000, pin_left: 2 },
        AuthReply::LockedOut { retry_after_ms: 1 },
    ] {
        assert_eq!(AuthReply::decode(&a.encode()).unwrap(), a);
    }
    assert_eq!(parse_reply(&reply_ok(&[1, 2])).unwrap(), &[1, 2]);
    assert_eq!(parse_reply(&reply_err(Error::AccessDenied)), Err(Error::AccessDenied));
    let list = vec![AccountSummary { uid: 1000, name: "alice".into(), display: "Alice".into(), methods: 3, flags: flags::ADMIN }];
    assert_eq!(decode_list(&encode_list(&list)).unwrap(), list);
    assert!(list[0].has(Method::Pin) && !list[0].has(Method::Key));
    let w = WhoAmI {
        summary: list[0].clone(),
        groups: vec![ids::GROUP_USERS],
        home: "/home/alice".into(),
        phrase: "modra".into(),
        session: 1,
        keys: vec![(1, "klic".into(), true)],
    };
    assert_eq!(WhoAmI::decode(&w.encode()).unwrap(), w);
}

#[test]
fn zpravy_initu() {
    for m in [
        ToInit::OpenSession { uid: 1000, groups: vec![ids::GROUP_USERS], home: "/home/alice".into() },
        ToInit::CloseSession { session: 1 },
        ToInit::LockSession { session: 1 },
        ToInit::UnlockSession { session: 1 },
        ToInit::CreateHome { uid: 1000, name: "alice".into() },
        ToInit::DeleteHome { uid: 1000, name: "alice".into() },
        ToInit::ListSessions,
        ToInit::UnlockHome { uid: 1000, key: Secret::new(vec![7; 32]) },
    ] {
        assert_eq!(ToInit::decode(&m.encode()).unwrap(), m);
    }
    let s = vec![(1, 1000, false), (2, 1001, true)];
    assert_eq!(decode_sessions(&encode_sessions(&s)).unwrap(), s);
    assert_eq!(decode_session_closed(&encode_session_closed(9)).unwrap(), 9);
}

#[test]
fn jmena() {
    assert!(valid_name("alice"));
    assert!(valid_name("a-b_9"));
    for bad in ["", "Alice", "9a", "a b", "a/b", "..", "ž", "abcdefghijabcdefghijabcdefghijabc"] {
        assert!(!valid_name(bad), "{bad}");
    }
    assert!(valid_display("Žluťoučký kůň"));
    assert!(!valid_display(""));
    assert!(!valid_display("a\nb"));
    assert!(valid_phrase(""));
    assert!(Method::Password.bit() == 1 && Method::Dev.bit() == 8);
}

#[test]
fn ukony_spravy() {
    use crate::elevate::{Params, action, decode_check, encode_check};
    for p in [
        Params::AccountCreate { name: "bob".into(), display: "Bob".into(), password: Secret::from("heslo bob"), admin: false },
        Params::AccountDelete { uid: 1001 },
        Params::ResetPassword { uid: 1001, password: Secret::from("nove heslo") },
        Params::RoleSet { uid: 1001, on: true },
    ] {
        assert_eq!(Params::decode(p.action(), &p.encode()).unwrap(), p);
    }
    assert_eq!(Params::decode(action::FIRST_SERVICE, &[]), Err(Error::NotSupported));
    assert_eq!(Params::decode(99, &[]), Err(Error::InvalidArgs));
    let b = encode_check(&[7; 16], 6, b"disk0");
    assert_eq!(decode_check(&b).unwrap(), (vec![7; 16], 6, b"disk0".to_vec()));
    assert!(decode_check(&encode_check(&[7; 3], 6, b"")).is_err());
}

#[test]
fn systemovy_soubor_parametry_a_cesty() {
    use crate::elevate::{FileOp, SystemFile, system_path};
    let f = SystemFile { op: FileOp::Write, path: "/system/config/x.toml".into(), len: 5, hash: u128::MAX - 3 };
    assert_eq!(SystemFile::decode(&f.encode()).unwrap(), f);
    for ok in ["/system/config", "/system/config/vzhled/system.toml", "/system/a.b"] {
        assert_eq!(system_path(ok), Ok(()), "{ok}");
    }
    for bad in ["/system", "/system/", "/home/x", "system/x", "/system//x", "/system/x/", "/system/./x", "/system/x/../accounts"] {
        assert_eq!(system_path(bad), Err(Error::InvalidArgs), "{bad}");
    }
    // Databaze uctu nikdy, ani s roli spravy.
    for bad in ["/system/accounts", "/system/accounts/users/1000"] {
        assert_eq!(system_path(bad), Err(Error::AccessDenied), "{bad}");
    }
    let bad = SystemFile { path: "/system/accounts/key".into(), ..f.clone() };
    assert_eq!(SystemFile::decode(&bad.encode()), Err(Error::AccessDenied));
    let mut b = f.encode();
    b[0] = 9;
    assert_eq!(SystemFile::decode(&b), Err(Error::InvalidArgs));
}

#[test]
fn rezim_vyvojare_jen_s_feature() {
    let mut w = wire::Writer::op(crate::op::DEV_MODE);
    w.u8(crate::DEV_ON);
    let raw = w.finish();
    #[cfg(not(feature = "dev-mode"))]
    assert_eq!(crate::Request::decode(&raw), Err(Error::NotSupported));
    #[cfg(feature = "dev-mode")]
    {
        let r = crate::Request::decode(&raw).unwrap();
        assert_eq!(r, crate::Request::DevMode { set: crate::DEV_ON });
        assert!(r.is_self());
        assert_eq!(r.encode(), raw);
    }
}

// --- CBOR ---

#[test]
fn cbor_vektory_rfc8949() {
    let cases: &[(Value, &[u8])] = &[
        (Value::Uint(0), &[0x00]),
        (Value::Uint(23), &[0x17]),
        (Value::Uint(24), &[0x18, 0x18]),
        (Value::Uint(1000), &[0x19, 0x03, 0xe8]),
        (Value::Uint(1_000_000), &[0x1a, 0x00, 0x0f, 0x42, 0x40]),
        (Value::int(-1), &[0x20]),
        (Value::int(-1000), &[0x39, 0x03, 0xe7]),
        (Value::Bytes(vec![1, 2, 3, 4]), &[0x44, 1, 2, 3, 4]),
        (Value::text("IETF"), &[0x64, 0x49, 0x45, 0x54, 0x46]),
        (Value::Array(vec![Value::Uint(1), Value::Uint(2)]), &[0x82, 1, 2]),
        (Value::Bool(true), &[0xf5]),
        (Value::Null, &[0xf6]),
    ];
    for (v, b) in cases {
        assert_eq!(&v.encode(), b, "{v:?}");
        assert_eq!(&cbor::decode(b).unwrap(), v);
    }
}

#[test]
fn cbor_kanonicke_poradi_klicu() {
    // CTAP2: kratsi klic drive, pak podle bajtu; cisla pred textem.
    let m = Value::Map(vec![
        (Value::text("bb"), Value::Uint(1)),
        (Value::text("a"), Value::Uint(2)),
        (Value::int(-1), Value::Uint(3)),
        (Value::Uint(10), Value::Uint(4)),
        (Value::Uint(1), Value::Uint(5)),
    ]);
    let e = m.encode();
    assert_eq!(e, vec![0xa5, 0x01, 0x05, 0x0a, 0x04, 0x20, 0x03, 0x61, b'a', 0x02, 0x62, b'b', b'b', 0x01]);
}

#[test]
fn cbor_odmita_nesmysly() {
    let bad: &[&[u8]] = &[
        &[],
        &[0x5f],             // neurcita delka
        &[0xc0, 0x00],       // tag
        &[0xf9, 0x00, 0x00], // float16
        &[0x44, 1, 2],       // kratke bajty
        &[0x62, 0xff, 0xfe], // neplatne UTF-8
        &[0xa2, 0x01, 0x00, 0x01, 0x00], // duplicitni klic
        &[0x01, 0x02],       // data navic
        &[0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff], // obri pole
    ];
    for b in bad {
        assert!(cbor::decode(b).is_err(), "{b:x?}");
    }
    // Hluboke vnoreni.
    let deep: Vec<u8> = core::iter::repeat_n(0x81u8, 20).chain([0x00]).collect();
    assert!(cbor::decode(&deep).is_err());
}

// --- CTAPHID ---

#[test]
fn ctaphid_rozdeleni_a_slozeni() {
    for len in [0usize, 1, 57, 58, 59 + 57, 1000, hid::MAX_MESSAGE] {
        let data: Vec<u8> = (0..len).map(|i| i as u8).collect();
        let ps = hid::fragment(0x1234_5678, cmd::CBOR, &data).unwrap();
        let mut a = Assembler::default();
        let mut got = None;
        for (i, p) in ps.iter().enumerate() {
            let r = a.push(p).unwrap();
            assert_eq!(r.is_some(), i + 1 == ps.len());
            got = r.or(got);
        }
        let m = got.unwrap();
        assert_eq!((m.cid, m.command, m.data), (0x1234_5678, cmd::CBOR, data));
    }
    assert!(hid::fragment(1, cmd::CBOR, &vec![0; hid::MAX_MESSAGE + 1]).is_err());
}

#[test]
fn ctaphid_spatne_poradi() {
    let ps = hid::fragment(7, cmd::CBOR, &[0u8; 200]).unwrap();
    let mut a = Assembler::default();
    assert!(a.push(&ps[1]).is_err(), "pokracovani bez zacatku");
    a.push(&ps[0]).unwrap();
    assert!(a.push(&ps[2]).is_err(), "preskocene poradi");
    let mut a = Assembler::default();
    a.push(&ps[0]).unwrap();
    let mut other = ps[1];
    other[3] = 8;
    assert!(a.push(&other).is_err(), "cizi kanal");
}

// --- protokol PIN ---

#[test]
fn pin_protokoly_sifrovani_a_autentizace() {
    let mut rng = SeedRng::new(1);
    let ka = p256::SecretKey::random(&mut rng);
    let (x, y) = crate::ctap::cose::xy(&ka.public_key());
    let peer = crate::ctap::cose::ecdh_key(&x, &y);
    for proto in [Protocol::V1, Protocol::V2] {
        let (platform, shared) = pin::encapsulate(&mut rng, proto, &peer).unwrap();
        let theirs = pin::decapsulate(&ka, proto, &platform).unwrap();
        let ct = shared.encrypt(&mut rng, &[5u8; 32]).unwrap();
        assert_eq!(ct.len(), if proto == Protocol::V2 { 48 } else { 32 });
        assert_eq!(theirs.decrypt(&ct).unwrap(), vec![5u8; 32]);
        let tag = shared.authenticate(b"zprava");
        assert_eq!(tag.len(), if proto == Protocol::V2 { 32 } else { 16 });
        assert!(theirs.verify(b"zprava", &tag));
        assert!(!theirs.verify(b"zprava!", &tag));
        assert!(shared.encrypt(&mut rng, &[0u8; 15]).is_none());
    }
    // Bod mimo krivku.
    let bad = crate::ctap::cose::ecdh_key(&[1; 32], &[2; 32]);
    assert!(pin::encapsulate(&mut rng, Protocol::V2, &bad).is_none());
    assert!(pin::pad_pin(b"123").is_none());
    assert!(pin::pad_pin(&[b'1'; 64]).is_none());
    let _ = Shared::derive(Protocol::V2, &[0; 32]);
}

// --- CTAP2 nad simulovanym klicem ---

const CDH: [u8; 32] = [0x11; 32];

fn register(key: &mut SoftKey, rng: &mut SeedRng, pin_value: Option<&str>) -> crate::ctap::Credential {
    let mut c = Client::new(&mut *key).unwrap();
    let token = pin_value.map(|p| c.pin_token(rng, p.as_bytes()).unwrap());
    c.make_credential(&CDH, &1000u64.to_le_bytes(), "alice", &[], token.as_ref()).unwrap()
}

#[test]
fn ctap2_registrace_a_prihlaseni_bez_pinu() {
    let mut rng = SeedRng::new(2);
    let mut key = SoftKey::new(10);
    let cred = register(&mut key, &mut rng, None);
    assert!(cred.hmac_secret);
    assert_eq!(cred.flags & flag::UV, 0);
    let allow = vec![(cred.id.clone(), cred.public.clone())];
    let mut c = Client::new(&mut key).unwrap();
    let a = c.get_assertion(&mut rng, &[0x22; 32], &allow, None, None).unwrap();
    assert_eq!(a.id, cred.id);
    assert!(a.sign_count > cred.sign_count);
    assert!(a.hmac_output.is_none());
}

#[test]
fn ctap2_pin_klice_a_hmac_secret() {
    for (proto, ctap20) in [(vec![2, 1], false), (vec![1], false), (vec![1], true)] {
        let mut rng = SeedRng::new(3);
        let mut key = SoftKey::new(11);
        key.protocols = proto.clone();
        key.ctap20_only = ctap20;
        // Klic bez PINu: platforma PIN nastavi.
        {
            let mut c = Client::new(&mut key).unwrap();
            assert!(!c.pin_set());
            c.set_pin(&mut rng, b"4321").unwrap();
            assert_eq!(c.pin_retries().unwrap(), 8);
        }
        let cred = register(&mut key, &mut rng, Some("4321"));
        assert_ne!(cred.flags & flag::UV, 0, "registrace s PINem = UV");
        let allow = vec![(cred.id.clone(), cred.public.clone())];
        let salt = [0x33u8; 32];
        let mut c = Client::new(&mut key).unwrap();
        let t = c.pin_token(&mut rng, b"4321").unwrap();
        let a1 = c.get_assertion(&mut rng, &[1; 32], &allow, Some(&t), Some(&salt)).unwrap();
        assert_ne!(a1.flags & flag::UV, 0);
        let t = c.pin_token(&mut rng, b"4321").unwrap();
        let a2 = c.get_assertion(&mut rng, &[2; 32], &allow, Some(&t), Some(&salt)).unwrap();
        // Stejna sul a stejny klic = stejne tajemstvi (obal klice domova).
        assert_eq!(a1.hmac_output, a2.hmac_output);
        assert!(a1.hmac_output.is_some());
        // Jina sul = jine tajemstvi; bez UV jine nez s UV.
        let a3 = c.get_assertion(&mut rng, &[3; 32], &allow, None, Some(&salt)).unwrap();
        assert_ne!(a3.hmac_output, a1.hmac_output);
        assert_eq!(a3.flags & flag::UV, 0);
        let t = c.pin_token(&mut rng, b"4321").unwrap();
        let a4 = c.get_assertion(&mut rng, &[4; 32], &allow, Some(&t), Some(&[0x34; 32])).unwrap();
        assert_ne!(a4.hmac_output, a1.hmac_output, "proto {proto:?}");
    }
}

#[test]
fn ctap2_spatny_pin_a_blokace() {
    let mut rng = SeedRng::new(4);
    let mut key = SoftKey::with_pin(12, "1111");
    let mut c = Client::new(&mut key).unwrap();
    assert!(c.pin_set());
    for left in (1..8).rev() {
        assert_eq!(c.pin_token(&mut rng, b"9999").err(), Some(CtapError::Status(status::PIN_INVALID)));
        assert_eq!(c.pin_retries().unwrap(), left);
    }
    assert_eq!(c.pin_token(&mut rng, b"9999").err(), Some(CtapError::Status(status::PIN_BLOCKED)));
    assert_eq!(c.pin_token(&mut rng, b"1111").err(), Some(CtapError::Status(status::PIN_BLOCKED)));
}

#[test]
fn ctap2_klic_s_pinem_vyzaduje_token_pri_registraci() {
    let mut key = SoftKey::with_pin(13, "1111");
    let mut c = Client::new(&mut key).unwrap();
    assert_eq!(
        c.make_credential(&CDH, b"u", "alice", &[], None).err(),
        Some(CtapError::Status(status::PUAT_REQUIRED))
    );
}

#[test]
fn ctap2_odmitnuti() {
    let mut rng = SeedRng::new(5);
    let mut key = SoftKey::new(14);
    let cred = register(&mut key, &mut rng, None);
    let allow = vec![(cred.id.clone(), cred.public.clone())];
    // Bez doteku.
    key.touch = false;
    let mut c = Client::new(&mut key).unwrap();
    assert_eq!(c.get_assertion(&mut rng, &CDH, &allow, None, None).err(), Some(CtapError::Status(status::OPERATION_DENIED)));
    key.touch = true;
    // Neznamy klic.
    let unknown = vec![(vec![9u8; 32], cred.public.clone())];
    let mut c = Client::new(&mut key).unwrap();
    assert_eq!(c.get_assertion(&mut rng, &CDH, &unknown, None, None).err(), Some(CtapError::Status(status::NO_CREDENTIALS)));
    // Podpis jinym klicem (ulozeny verejny klic jineho prihlasovaciho udaje).
    let mut other = SoftKey::new(15);
    let cred2 = register(&mut other, &mut rng, None);
    let wrong = vec![(cred.id.clone(), cred2.public.clone())];
    let mut c = Client::new(&mut key).unwrap();
    assert_eq!(c.get_assertion(&mut rng, &CDH, &wrong, None, None).err(), Some(CtapError::Invalid));
    // Opakovana registrace stejneho klice (excludeList).
    let mut c = Client::new(&mut key).unwrap();
    assert_eq!(
        c.make_credential(&CDH, b"u", "alice", &[cred.id.clone()], None).err(),
        Some(CtapError::Status(status::CREDENTIAL_EXCLUDED))
    );
}

#[test]
fn ctap2_eddsa() {
    let mut rng = SeedRng::new(6);
    let mut key = SoftKey::new(16);
    key.prefer_ed25519 = true;
    let cred = register(&mut key, &mut rng, None);
    assert!(matches!(cred.public, crate::ctap::PublicKey::Ed25519(_)));
    let allow = vec![(cred.id.clone(), cred.public.clone())];
    let mut c = Client::new(&mut key).unwrap();
    c.get_assertion(&mut rng, &CDH, &allow, None, None).unwrap();
}

#[test]
fn ctap2_podvrzena_authdata() {
    // Podpis nad jinymi daty nez authData || cdh neprojde overenim.
    let mut rng = SeedRng::new(7);
    let mut key = SoftKey::new(17);
    let cred = register(&mut key, &mut rng, None);
    assert!(!cred.public.verify(b"cokoli", &[0x30, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01]));
    let ad = AuthData::parse(&AuthData { rp_id_hash: [0; 32], flags: 1, sign_count: 5, credential: None, extensions: None }.encode()).unwrap();
    assert_eq!(ad.sign_count, 5);
    assert!(AuthData::parse(&[0; 36]).is_none());
    let mut extra = AuthData { rp_id_hash: [0; 32], flags: 1, sign_count: 5, credential: None, extensions: None }.encode();
    extra.push(0);
    assert!(AuthData::parse(&extra).is_none(), "data navic za authData");
}

#[test]
fn ctap2_pres_ctaphid() {
    let mut rng = SeedRng::new(8);
    let dev = SoftHid::new(SoftKey::with_pin(18, "2468"));
    let t = HidTransport::open(dev, [1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
    assert_ne!(t.channel(), hid::BROADCAST);
    let mut c = Client::new(t).unwrap();
    let tok = c.pin_token(&mut rng, b"2468").unwrap();
    let cred = c.make_credential(&CDH, b"u", "alice", &[], Some(&tok)).unwrap();
    assert!(c.transport.up_needed, "klic hlasil cekani na dotek");
    let allow = vec![(cred.id.clone(), cred.public.clone())];
    let tok = c.pin_token(&mut rng, b"2468").unwrap();
    let a = c.get_assertion(&mut rng, &CDH, &allow, Some(&tok), Some(&[9; 32])).unwrap();
    assert!(a.hmac_output.is_some());
    assert_eq!(c.transport.dev.key.credentials(), 1);
}

#[test]
fn ctap2_nesmyslna_odpoved_klice() {
    struct Bad(Vec<u8>);
    impl crate::ctap::Transport for Bad {
        fn transact(&mut self, _r: &[u8]) -> crate::ctap::CtapResult<Vec<u8>> {
            Ok(self.0.clone())
        }
    }
    assert_eq!(Client::new(Bad(vec![])).err(), Some(CtapError::Invalid));
    assert_eq!(Client::new(Bad(vec![0, 0xff])).err(), Some(CtapError::Invalid));
    assert_eq!(Client::new(Bad(vec![0x31])).err(), Some(CtapError::Status(0x31)));
    // Klic jen U2F (verze bez FIDO_2).
    let info = int_map(vec![(1, Value::Array(vec![Value::text("U2F_V2")])), (3, Value::bytes(&[0; 16]))]);
    let mut b = vec![0];
    b.extend_from_slice(&info.encode());
    assert_eq!(Client::new(Bad(b)).err(), Some(CtapError::Unsupported));
}

#[test]
fn jmena_tam_a_zpet() {
    let l = vec![
        AccountName { uid: 1000, name: String::from("alice"), display: String::from("Alice Nováková") },
        AccountName { uid: 1001, name: String::from("bob"), display: String::new() },
    ];
    assert_eq!(decode_names(&encode_names(&l)).unwrap(), l);
    assert!(decode_names(&encode_names(&l)[..5]).is_err());
    assert!(!Request::Names.is_login() && !Request::Names.is_self());
}

#[test]
fn ssh_odpovedi_tam_a_zpet() {
    use crate::ssh::{SshAuthReply, SshKey, decode_keys, encode_keys};
    for r in [
        SshAuthReply::Ok { uid: 1000, session: 7, ticket: vec![9; crate::ssh::TICKET] },
        SshAuthReply::Denied { retry_after_ms: 30_000 },
        SshAuthReply::Locked,
    ] {
        let b = r.encode();
        assert_eq!(SshAuthReply::decode(&b).unwrap(), r);
        assert!(SshAuthReply::decode(&b[..b.len() - 1]).is_err());
    }
    let keys = vec![SshKey { id: 1, algo: "ssh-ed25519".into(), fingerprint: "SHA256:x".into(), comment: "alice@pc".into(), added: 5 }];
    assert_eq!(decode_keys(&encode_keys(&keys)).unwrap(), keys);
    let m = ToInit::OpenRemote { uid: 1000, groups: vec![ids::GROUP_USERS], home: "/home/alice".into(), peer: "10.0.2.2".into() };
    assert_eq!(ToInit::decode(&m.encode()).unwrap(), m);
}

#[test]
fn klienti_ssh_a_vyzvy() {
    use crate::agent::*;
    roundtrip(Request::ClientKeyCreate { password: Secret::from("a"), comment: String::from("alice@mo") });
    roundtrip(Request::ClientKeyImport { password: Secret::from("a"), private: Secret::from("-----BEGIN") });
    roundtrip(Request::ClientKeyList);
    roundtrip(Request::ClientKeyRemove { id: 3 });
    for r in [Request::ClientKeyCreate { password: Secret::default(), comment: String::new() }, Request::ClientKeyList, Request::ClientKeyRemove { id: 1 }] {
        assert!(r.is_self(), "{r:?}");
    }
    let s = AgentReq::Sign { id: 2, host: String::from("[h]:22"), data: alloc::vec![1, 2, 3] };
    assert_eq!(AgentReq::decode(&s.encode()).unwrap(), s);
    assert_eq!(AgentReq::decode(&AgentReq::Identities.encode()).unwrap(), AgentReq::Identities);
    assert_eq!(AgentReq::decode(&[9, 0, 0, 0]), Err(Error::NotSupported));
    let ids = alloc::vec![Identity { id: 1, blob: alloc::vec![5; 51], comment: String::from("k") }];
    assert_eq!(decode_identities(&encode_identities(&ids)).unwrap(), ids);
    assert_eq!(decode_created(&encode_created(4, "ssh-ed25519 AAAA k", "SHA256:x")).unwrap(), (4, String::from("ssh-ed25519 AAAA k"), String::from("SHA256:x")));
    for p in [Prompt::Secret { id: 1, question: String::from("Heslo") }, Prompt::Confirm { id: 2, question: String::from("Server?") }] {
        assert_eq!(Prompt::decode(&p.encode()).unwrap(), p);
    }
    let a = PromptAnswer { id: 7, ok: true, text: Secret::from("tajne") };
    assert_eq!(PromptAnswer::decode(&a.encode()).unwrap(), a);
}
