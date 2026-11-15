//! Prikaz `acl`: vypis, pridani a odebrani grantu nad `MemFs`, jmena
//! prijemcu od hostitele, srozumitelne chyby bez prava Spravovat.

use crate::acl::{self, Account, Acl, Grant, grant, rights};
use crate::eval::Interp;
use crate::host::{Fs, Host};
use crate::sig::Builtins;
use crate::value::Value;
use crate::vfs::mem::MemFs;

struct AclHost {
    fs: MemFs,
    emitted: Vec<Value>,
}

impl Host for AclHost {
    fn fs(&mut self) -> &mut dyn Fs {
        &mut self.fs
    }
    fn emit(&mut self, value: Value) {
        self.emitted.push(value);
    }
    fn note(&mut self, _text: &str) {}
    fn now(&mut self) -> Option<i64> {
        Some(0)
    }
    fn sleep(&mut self, _ns: i64) {}
    fn interrupted(&mut self) -> bool {
        false
    }
    fn confirm(&mut self, _question: &str, _detail: &Value) -> bool {
        false
    }
    fn accounts(&mut self) -> Result<Vec<Account>, abi::Error> {
        Ok(vec![
            Account { uid: 1000, name: "alice".into(), display: "Alice".into() },
            Account { uid: 1001, name: "bob".into(), display: "Bob".into() },
        ])
    }
}

fn host() -> AclHost {
    let mut h = AclHost { fs: MemFs::new(), emitted: Vec::new() };
    h.fs.put("/home/alice/Projekty/plan.txt", b"x");
    h
}

/// Spusti radek; vrati kod chyby a celou chybu (text pro kontrolu vet).
fn run(h: &mut AclHost, line: &str) -> Result<(), (String, String)> {
    let mut it = Interp::new(Box::new(Builtins), "/home/alice");
    it.run_line(h, line).result.map(|_| ()).map_err(|e| (e.code.clone(), e.report(line)))
}

fn grants(v: &Value) -> Vec<Value> {
    match v.field("grants") {
        Some(Value::List(g)) => g,
        other => panic!("grants: {other:?}"),
    }
}

#[test]
fn acl_vypis_bez_vlastniho_popisu() {
    let mut h = host();
    run(&mut h, "acl Projekty").unwrap();
    let v = h.emitted.last().unwrap();
    assert_eq!(v.field("path"), Some(Value::str("/home/alice/Projekty")));
    assert_eq!(v.field("owner"), Some(Value::str("alice")));
    assert_eq!(v.field("your_level"), Some(Value::str("manage")));
    assert_eq!(v.field("your_label"), Some(Value::str("Spravovat")));
    assert_eq!(v.field("inherits"), Some(Value::Bool(true)));
    assert!(grants(v).is_empty());
}

#[test]
fn acl_add_remove_a_zdedene() {
    let mut h = host();
    run(&mut h, "acl add bob edit Projekty --inherit").unwrap();
    let a = h.fs.acls.get("/home/alice/projekty").cloned().expect("popis zapsan");
    assert_eq!(a.grants, vec![Grant { who: 1001, rights: rights::EDIT | rights::EXECUTE, flags: grant::THIS | grant::INHERIT }]);
    let g = grants(h.emitted.last().unwrap());
    assert_eq!(g[0].field("who"), Some(Value::str("bob")));
    assert_eq!(g[0].field("kind"), Some(Value::str("user")));
    assert_eq!(g[0].field("label"), Some(Value::str("Upravovat")));
    assert_eq!(g[0].field("scope"), Some(Value::str("this+children")));

    // Zmena urovne nahradi grant, dalsi prijemce se prida.
    run(&mut h, "acl add bob view Projekty").unwrap();
    run(&mut h, "acl add users run Projekty").unwrap();
    let a = h.fs.acls.get("/home/alice/projekty").cloned().unwrap();
    assert_eq!(a.grants.len(), 2);
    assert!(a.grants.contains(&Grant { who: 1001, rights: rights::VIEW, flags: grant::THIS }));
    assert!(a.grants.contains(&Grant { who: acl::GROUP_USERS, rights: rights::RUN, flags: grant::THIS }));

    // Soubor ve slozce vidi zdedeny grant (jen ty s dedenim).
    run(&mut h, "acl add everyone view Projekty -i").unwrap();
    run(&mut h, "acl Projekty/plan.txt").unwrap();
    let g = grants(h.emitted.last().unwrap());
    assert_eq!(g.len(), 1, "{g:?}");
    assert_eq!(g[0].field("who"), Some(Value::str("everyone")));
    assert_eq!(g[0].field("kind"), Some(Value::str("group")));
    assert_eq!(g[0].field("scope"), Some(Value::str("inherited")));
    assert_eq!(g[0].field("from"), Some(Value::str("/home/alice/Projekty")));

    // Odebrani; posledni grant pryc = zadny vlastni popis.
    run(&mut h, "acl remove bob Projekty").unwrap();
    run(&mut h, "acl remove users Projekty").unwrap();
    run(&mut h, "acl remove everyone Projekty").unwrap();
    assert!(!h.fs.acls.contains_key("/home/alice/projekty"));
    let e = run(&mut h, "acl remove bob Projekty").unwrap_err();
    assert_eq!(e.0, "SH_ARGS");
    assert!(e.1.contains("nemá na /home/alice/Projekty vlastní grant"), "{}", e.1);
}

#[test]
fn acl_cislo_vlastni_maska_a_nededi() {
    let mut h = host();
    h.fs.acls.insert(
        "/home/alice/projekty".into(),
        Acl { protected: true, grants: vec![Grant { who: 1_000_005, rights: rights::READ | rights::WRITE, flags: grant::THIS }] },
    );
    h.fs.acls.insert("/home/alice".into(), Acl { protected: false, grants: vec![Grant { who: 1001, rights: rights::VIEW, flags: grant::INHERIT }] });
    run(&mut h, "acl Projekty").unwrap();
    let v = h.emitted.last().unwrap();
    assert_eq!(v.field("inherits"), Some(Value::Bool(false)));
    let g = grants(v);
    // Nededi: grant z /home/alice se neukaze.
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].field("who"), Some(Value::str("skupina 1000005")));
    assert_eq!(g[0].field("level"), Some(Value::str("custom")));
    assert_eq!(g[0].field("label"), Some(Value::str("Vlastní (čtení, zápis)")));
    // Prijemce cislem, odebrani zachova `protected`.
    run(&mut h, "acl add 1002 manage Projekty").unwrap();
    run(&mut h, "acl remove 1000005 Projekty").unwrap();
    let a = h.fs.acls.get("/home/alice/projekty").cloned().unwrap();
    assert!(a.protected);
    assert_eq!(a.grants, vec![Grant { who: 1002, rights: rights::ALL, flags: grant::THIS }]);
}

#[test]
fn acl_chyby_jsou_srozumitelne() {
    let mut h = host();
    h.fs.rights = rights::VIEW;
    let e = run(&mut h, "acl add bob edit Projekty").unwrap_err();
    assert_eq!(e.0, "ACCESS_DENIED");
    assert!(e.1.contains("smí je měnit jen vlastník nebo kdo má právo Spravovat"), "{}", e.1);
    assert!(h.fs.acls.is_empty());
    // Vypis bez prava Spravovat jde, ukaze vasi uroven.
    run(&mut h, "acl Projekty").unwrap();
    assert_eq!(h.emitted.last().unwrap().field("your_label"), Some(Value::str("Zobrazit")));

    let mut h = host();
    for (line, needle) in [
        ("acl add karel edit Projekty", "neznámý uživatel nebo skupina 'karel'"),
        ("acl add bob write Projekty", "neznámá úroveň 'write'"),
        ("acl add bob edit", "chybí cesta"),
        ("acl add bob edit Projekty navic", "moc argumentů"),
        ("acl", "chybí argument 'path'"),
    ] {
        let e = run(&mut h, line).unwrap_err();
        assert_eq!(e.0, "SH_ARGS", "{line}");
        assert!(e.1.contains(needle), "{line}: {}", e.1);
    }
    let e = run(&mut h, "acl nic").unwrap_err();
    assert_eq!(e.0, "NOT_FOUND");
    assert!(e.1.contains("neexistuje nebo k ní nemáte žádné právo"), "{}", e.1);
    assert!(h.fs.acls.is_empty());
}

#[test]
fn acl_urovne_a_jmena() {
    assert_eq!(acl::level_rights("view"), Some(rights::VIEW));
    assert_eq!(acl::level_rights("manage"), Some(rights::ALL));
    assert_eq!(acl::level_rights("Zobrazit"), None);
    for (w, label, r) in acl::LEVELS {
        assert_eq!(acl::level_of(*r), (String::from(*w), String::from(*label)));
    }
    assert_eq!(acl::level_of(0).1, "Vlastní (nic)");
    // Cisty EDIT bez spousteni neni Upravovat (shoda s kartou Opravneni).
    assert_eq!(acl::level_of(rights::EDIT).0, "custom");
    assert_eq!(acl::level_rights("edit"), Some(rights::EDIT | rights::EXECUTE));
    let acc = [Account { uid: 1000, name: "alice".into(), display: "Alice".into() }];
    assert_eq!(acl::who_of("alice", &acc), Some(1000));
    assert_eq!(acl::who_of("everyone", &acc), Some(acl::EVERYONE));
    assert_eq!(acl::name_of(1007, &acc), "uživatel 1007");
    assert_eq!(acl::kind_of(acl::EVERYONE), "group");
}
