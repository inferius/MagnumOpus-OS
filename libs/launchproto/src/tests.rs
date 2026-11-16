//! Testy protokolu launchd (`cargo test -p launchproto`).

use alloc::string::String;
use alloc::vec;

use abi::Error;

use crate::{Command, Launch, Reader, Registry, Role, Source, Writer, flags, op, parse_reply, reply, request_head};

#[test]
fn launch_roundtrip() {
    let l = Launch {
        program: String::from("rg"),
        flags: flags::ALLOW_FOREIGN,
        roles: vec![Role::Job, Role::Fs, Role::Stdout],
        params: vec![1, 2, 3, 4],
    };
    let b = l.encode(9).unwrap();
    let mut r = Reader::new(&b);
    assert_eq!(request_head(&mut r).unwrap(), (op::LAUNCH, 9));
    assert_eq!(Launch::decode(&mut r).unwrap(), l);
}

#[test]
fn fd_role_repeats() {
    let l = Launch { program: String::from("x"), flags: 0, roles: vec![Role::Job, Role::Fd, Role::Stdout, Role::Fd], params: vec![] };
    let b = l.encode(2).unwrap();
    let mut r = Reader::new(&b);
    request_head(&mut r).unwrap();
    assert_eq!(Launch::decode(&mut r).unwrap(), l);
}

#[test]
fn launch_rejects_bad_input() {
    let mut l = Launch { program: String::from("x"), flags: 0, roles: vec![Role::Job; crate::MAX_ROLES + 1], params: vec![] };
    assert_eq!(l.encode(1).err(), Some(Error::InvalidArgs), "moc roli");
    l.roles = vec![Role::Job];
    l.flags = 1 << 30;
    assert_eq!(l.encode(1).err(), Some(Error::InvalidArgs), "neznamy priznak");
    // Opakovana role a neznama role.
    for roles in [&[1u8, 1][..], &[1, 99]] {
        let mut w = Writer::op(op::LAUNCH);
        w.u32(1).str("x").u32(0).u8(roles.len() as u8);
        for r in roles {
            w.u8(*r);
        }
        let b = w.finish();
        let mut r = Reader::new(&b);
        request_head(&mut r).unwrap();
        assert_eq!(Launch::decode(&mut r).err(), Some(Error::InvalidArgs), "{roles:?}");
    }
    // Zkracena zprava nepanikari.
    let b = Launch { program: String::from("abc"), flags: 0, roles: vec![Role::Job], params: vec![] }.encode(1).unwrap();
    for n in 8..b.len() - 1 {
        let mut r = Reader::new(&b[..n]);
        request_head(&mut r).unwrap();
        assert!(Launch::decode(&mut r).is_err(), "delka {n}");
    }
}

#[test]
fn reply_roundtrip() {
    let ok = reply(5, Ok(&[7, 8]));
    assert_eq!(parse_reply(&ok).unwrap(), (5, Ok(vec![7, 8])));
    let err = reply(6, Err((Error::NotMounted, "svazek data neni pripojen")));
    assert_eq!(parse_reply(&err).unwrap(), (6, Err((Error::NotMounted, String::from("svazek data neni pripojen")))));
}

#[test]
fn source_roundtrip() {
    for s in [
        Source::Initfs(String::from("hello-std")),
        Source::Path(String::from("/system/bin/rg")),
        Source::Volume { volume: String::from("data"), path: String::from("/svazky/data/apps/x/rg") },
    ] {
        let mut w = Writer::new();
        s.encode(&mut w);
        let b = w.finish();
        assert_eq!(Source::decode(&mut Reader::new(&b)).unwrap(), s);
    }
}

#[test]
fn registry_first_wins() {
    let mut reg = Registry::new();
    let c = |app: &str| Command { name: String::from("rg"), source: Source::Initfs(String::from(app)), app: String::from(app) };
    assert!(reg.add(c("a")));
    assert!(!reg.add(c("b")));
    assert_eq!(reg.lookup("rg").unwrap().app, "a");
    assert!(reg.lookup("x").is_none());
    assert!(crate::is_bare_name("rg"));
    assert!(!crate::is_bare_name("/system/bin/rg"));
    assert!(!crate::is_bare_name("./rg"));
}

#[test]
fn exec_roundtrip() {
    use crate::exec::{Exec, Mount, parse_reply as exec_reply_parse, reply as exec_reply};
    let e = Exec {
        program: String::from("ping"),
        cwd: String::from("/home/a"),
        args: vec![String::from("-c"), String::from("3"), String::from("10.0.2.2")],
        mounts: vec![Mount { name: String::from("/home/a"), target: String::from("/home/a"), read_only: true }],
        net: true,
    };
    assert_eq!(Exec::decode(&e.encode().unwrap()).unwrap(), e);
    // Zbytek za koncem, spatny priznak jen pro cteni, jina operace.
    let mut b = e.encode().unwrap();
    b.push(0);
    assert_eq!(Exec::decode(&b), Err(Error::InvalidArgs));
    let mut b = e.encode().unwrap();
    let last = b.len() - 1;
    b[last] = 2;
    assert_eq!(Exec::decode(&b), Err(Error::InvalidArgs));
    assert_eq!(Exec::decode(&[9, 0, 0, 0]), Err(Error::NotSupported));
    let many = Exec { args: vec![String::new(); crate::exec::MAX_ARGS + 1], ..e.clone() };
    assert_eq!(many.encode(), Err(Error::InvalidArgs));
    assert_eq!(exec_reply_parse(&exec_reply(Ok(()))).unwrap(), Ok(()));
    assert_eq!(exec_reply_parse(&exec_reply(Err((Error::NotFound, "ping: neni")))).unwrap(), Err((Error::NotFound, String::from("ping: neni"))));
}

/// Vsechny jedinecne role a nejvic fd v jedne zprave: projde kodovanim
/// i dekodovanim a vejde se do limitu handlu kanalu.
#[test]
fn all_roles_fit() {
    let unique: alloc::vec::Vec<Role> = (1..=255u8).filter_map(Role::from_u8).filter(|r| *r != Role::Fd).collect();
    assert_eq!(unique.len(), 10, "MAX_ROLES pocita s deseti jedinecnymi rolemi");
    assert!(unique.contains(&Role::Audio) && unique.contains(&Role::AudioLease));
    let mut roles = unique.clone();
    roles.extend(core::iter::repeat_n(Role::Fd, abi::params::MAX_FDS));
    assert_eq!(roles.len(), crate::MAX_ROLES);
    assert!(crate::MAX_ROLES <= abi::syscall::CHANNEL_MAX_HANDLES);
    let l = Launch { program: String::from("x"), flags: 0, roles, params: vec![] };
    let b = l.encode(3).unwrap();
    let mut r = Reader::new(&b);
    request_head(&mut r).unwrap();
    assert_eq!(Launch::decode(&mut r).unwrap(), l);
}
