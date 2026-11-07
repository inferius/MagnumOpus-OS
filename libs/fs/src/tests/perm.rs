//! Opravneni: vlastnik, granty a dedicnost, viditelnost, sdilene popisy,
//! prehrani logu, handly, zakaz spousteni, snapshoty.

use abi::Error;

use super::{assert_clean, new_fs, remount};
use crate::layout::feature;
use crate::{Acl, EVERYONE, Fs, Grant, Kind, MemDevice, Principal, access, acl_flags, grant, rights, share};

const ALICE: u64 = 1000;
const BOB: u64 = 1001;
const TEAM: u64 = 5000;

fn alice() -> Option<Principal> {
    Some(Principal::user(ALICE, &[]))
}

fn bob() -> Option<Principal> {
    Some(Principal::user(BOB, &[TEAM]))
}

fn acl(grants: &[(u64, u8, u8)]) -> Option<Acl> {
    Some(Acl {
        flags: 0,
        grants: grants.iter().map(|&(who, rights, flags)| Grant { who, rights, flags }).collect(),
    })
}

/// Svazek se strukturou jako po instalaci: / pruchozi pro kazdeho,
/// /home/alice patri Alici.
fn home_fs() -> Fs<MemDevice> {
    let mut fs = new_fs(8192);
    fs.set_acl("/", acl(&[(EVERYONE, rights::TRAVERSE, grant::THIS)])).unwrap();
    fs.create("/home", Kind::Dir).unwrap();
    fs.set_acl("/home", acl(&[(EVERYONE, rights::TRAVERSE, grant::THIS)])).unwrap();
    fs.create("/home/alice", Kind::Dir).unwrap();
    fs.set_owner("/home/alice", ALICE).unwrap();
    fs.create("/system", Kind::Dir).unwrap();
    fs
}

#[test]
fn owner_has_everything_others_see_nothing() {
    let mut fs = home_fs();
    fs.set_principal(alice());
    fs.create("/home/alice/dopis.txt", Kind::File).unwrap();
    fs.write("/home/alice/dopis.txt", 0, b"tajne").unwrap();
    let st = fs.stat("/home/alice/dopis.txt").unwrap();
    assert_eq!(st.owner, ALICE);
    assert_eq!(st.rights, rights::ALL);
    // Systemova slozka: neviditelna (ne ACCESS_DENIED - ani jmeno).
    assert_eq!(fs.stat("/system").err(), Some(Error::NotFound));
    assert_eq!(fs.create("/system/x", Kind::File).err(), Some(Error::NotFound));
    // / jde projit, ale ne vypsat.
    assert_eq!(fs.list("/").err(), Some(Error::AccessDenied));
    fs.set_principal(bob());
    assert_eq!(fs.stat("/home/alice").err(), Some(Error::NotFound));
    assert_eq!(fs.read("/home/alice/dopis.txt", 0, 10).err(), Some(Error::NotFound));
    assert_eq!(fs.remove("/home/alice/dopis.txt").err(), Some(Error::NotFound));
    // Kontrola svazku jen pro system.
    assert_eq!(fs.check(false).err(), Some(Error::AccessDenied));
    fs.set_principal(None);
    assert_clean(&mut fs);
}

#[test]
fn grants_inherit_and_protected_folder_stops_them() {
    let mut fs = home_fs();
    fs.set_principal(alice());
    fs.create("/home/alice/sdilene", Kind::Dir).unwrap();
    fs.create("/home/alice/sdilene/soukrome", Kind::Dir).unwrap();
    fs.create("/home/alice/sdilene/soukrome/denik.txt", Kind::File).unwrap();
    fs.create("/home/alice/sdilene/plan.txt", Kind::File).unwrap();
    // Tym smi /home/alice jen projit; sdilene upravovat (dedi se).
    fs.set_acl("/home/alice", acl(&[(TEAM, rights::TRAVERSE, grant::THIS)])).unwrap();
    fs.set_acl("/home/alice/sdilene", acl(&[(TEAM, rights::EDIT, grant::BOTH)])).unwrap();
    // Soukroma slozka uvnitr sdilene: bez zakazu, jen prerusi dedicnost.
    fs.set_acl("/home/alice/sdilene/soukrome", Some(Acl { flags: acl_flags::PROTECTED, grants: vec![] }))
        .unwrap();
    fs.set_principal(bob());
    assert_eq!(fs.list("/home/alice").err(), Some(Error::AccessDenied));
    let names: Vec<_> = fs.list("/home/alice/sdilene").unwrap().into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["plan.txt"]);
    fs.write("/home/alice/sdilene/plan.txt", 0, b"od Boba").unwrap();
    fs.create("/home/alice/sdilene/novy.txt", Kind::File).unwrap();
    assert_eq!(fs.stat("/home/alice/sdilene/novy.txt").unwrap().owner, BOB);
    assert_eq!(fs.stat("/home/alice/sdilene/soukrome").err(), Some(Error::NotFound));
    assert_eq!(fs.read("/home/alice/sdilene/soukrome/denik.txt", 0, 1).err(), Some(Error::NotFound));
    // Souhrny podstromu bez MANAGE nevidi.
    let st = fs.stat("/home/alice/sdilene").unwrap();
    assert_eq!(st.rights, rights::EDIT);
    assert_eq!(st.tree_files, 0);
    // Opravneni menit nesmi.
    assert_eq!(fs.set_acl("/home/alice/sdilene", None).err(), Some(Error::AccessDenied));
    assert_eq!(fs.set_owner("/home/alice/sdilene/plan.txt", BOB).err(), Some(Error::AccessDenied));
    fs.set_principal(None);
    assert_clean(&mut fs);
}

#[test]
fn read_only_grant_and_rename_rules() {
    let mut fs = home_fs();
    fs.set_principal(alice());
    fs.create("/home/alice/ctení", Kind::Dir).unwrap();
    fs.create("/home/alice/ctení/a.txt", Kind::File).unwrap();
    fs.set_acl("/home/alice", acl(&[(BOB, rights::TRAVERSE, grant::THIS)])).unwrap();
    fs.set_acl("/home/alice/ctení", acl(&[(BOB, rights::VIEW, grant::BOTH)])).unwrap();
    fs.set_principal(bob());
    assert_eq!(fs.read("/home/alice/ctení/a.txt", 0, 1).unwrap(), b"");
    assert_eq!(fs.write("/home/alice/ctení/a.txt", 0, b"x").err(), Some(Error::AccessDenied));
    assert_eq!(fs.truncate("/home/alice/ctení/a.txt", 5).err(), Some(Error::AccessDenied));
    assert_eq!(fs.remove("/home/alice/ctení/a.txt").err(), Some(Error::AccessDenied));
    assert_eq!(fs.rename("/home/alice/ctení/a.txt", "/home/alice/ctení/b.txt").err(), Some(Error::AccessDenied));
    assert_eq!(fs.create("/home/alice/ctení/c.txt", Kind::File).err(), Some(Error::AccessDenied));
    // Otevreni: pristup podle prav.
    assert!(fs.open("/home/alice/ctení/a.txt", access::READ, share::ALL).is_ok());
    assert_eq!(
        fs.open("/home/alice/ctení/a.txt", access::READ | access::WRITE, share::ALL).err(),
        Some(Error::AccessDenied)
    );
    fs.set_principal(None);
    assert_clean(&mut fs);
}

#[test]
fn descriptors_are_shared_and_released() {
    let mut fs = home_fs();
    fs.set_principal(alice());
    for n in ["a", "b", "c"] {
        let p = alloc::format!("/home/alice/{n}");
        fs.create(&p, Kind::File).unwrap();
        // Poradi grantu nehraje roli: stejny popis po normalizaci.
        let g = if n == "b" {
            acl(&[(TEAM, rights::READ, grant::THIS), (BOB, rights::VIEW, grant::THIS)])
        } else {
            acl(&[(BOB, rights::VIEW, grant::THIS), (TEAM, rights::READ, grant::THIS)])
        };
        fs.set_acl(&p, g).unwrap();
    }
    fs.set_principal(None);
    assert_eq!(fs.test_store().sb.features_ro_compat & feature::RO_COMPAT_ACL, feature::RO_COMPAT_ACL);
    let a = fs.permissions("/home/alice/a").unwrap();
    assert_eq!(a.acl, fs.permissions("/home/alice/b").unwrap().acl);
    let (_, ia) = fs.test_inode("/home/alice/a").unwrap();
    let (_, ib) = fs.test_inode("/home/alice/b").unwrap();
    assert_eq!(ia.acl, ib.acl);
    fs.remove("/home/alice/a").unwrap();
    fs.set_acl("/home/alice/b", None).unwrap();
    assert_clean(&mut fs);
    fs.remove("/home/alice/c").unwrap();
    fs.commit().unwrap();
    assert_clean(&mut fs);
}

#[test]
fn owner_and_acl_survive_log_replay() {
    let mut fs = home_fs();
    fs.commit().unwrap();
    fs.set_principal(alice());
    fs.create("/home/alice/d", Kind::Dir).unwrap();
    fs.symlink("/home/alice/odkaz", "d").unwrap();
    fs.set_acl("/home/alice/d", acl(&[(BOB, rights::VIEW, grant::BOTH)])).unwrap();
    fs.set_principal(None);
    fs.set_owner("/home/alice/d", BOB).unwrap();
    fs.fsync().unwrap();
    let mut fs = remount(fs);
    let p = fs.permissions("/home/alice/d").unwrap();
    assert_eq!(p.owner, BOB);
    assert_eq!(p.acl, acl(&[(BOB, rights::VIEW, grant::BOTH)]));
    assert_eq!(fs.stat("/home/alice/odkaz").unwrap().owner, ALICE);
    assert_clean(&mut fs);
}

#[test]
fn handle_listing_hides_and_noexec_blocks_execute() {
    let mut fs = home_fs();
    fs.create_dataset("/home/alice/stazene").unwrap();
    fs.set_owner("/home/alice/stazene", ALICE).unwrap();
    fs.set_principal(alice());
    fs.create("/home/alice/stazene/program", Kind::File).unwrap();
    fs.create("/home/alice/verejne", Kind::Dir).unwrap();
    fs.create("/home/alice/verejne/skryte", Kind::File).unwrap();
    fs.create("/home/alice/verejne/videt", Kind::File).unwrap();
    fs.set_acl("/home/alice", acl(&[(BOB, rights::TRAVERSE, grant::THIS)])).unwrap();
    fs.set_acl("/home/alice/verejne", acl(&[(BOB, rights::LIST | rights::TRAVERSE, grant::THIS)])).unwrap();
    fs.set_acl("/home/alice/verejne/videt", acl(&[(BOB, rights::READ, grant::THIS)])).unwrap();
    // Spusteni jde, dokud dataset nema zakaz.
    let h = fs.open("/home/alice/stazene/program", access::EXECUTE, share::ALL).unwrap();
    fs.close(h).unwrap();
    fs.set_noexec("/home/alice/stazene", true).unwrap();
    assert!(fs.noexec("/home/alice/stazene").unwrap());
    assert_eq!(
        fs.open("/home/alice/stazene/program", access::EXECUTE, share::ALL).err(),
        Some(Error::AccessDenied)
    );
    // Zakaz plati i pro system.
    fs.set_principal(None);
    assert_eq!(
        fs.open("/home/alice/stazene/program", access::EXECUTE, share::ALL).err(),
        Some(Error::AccessDenied)
    );
    fs.set_principal(bob());
    let h = fs.open("/home/alice/verejne", access::READ, share::ALL).err();
    // Bob nema READ k adresari - otevrit pro cteni nejde, jen pro vypis.
    assert_eq!(h, Some(Error::AccessDenied));
    let h = fs.open("/home/alice/verejne", 0, share::ALL).unwrap();
    let names: Vec<_> = fs.list_h(h).unwrap().into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["videt"]);
    // Dataset Alice Bob nevidi ani v seznamu datasetu.
    assert!(fs.datasets().unwrap().is_empty());
    fs.close(h).unwrap();
    fs.set_principal(None);
    assert_eq!(fs.datasets().unwrap().len(), 1);
    assert_clean(&mut fs);
}

#[test]
fn snapshots_use_their_own_permissions() {
    let mut fs = home_fs();
    fs.create_dataset("/home/alice/projekt").unwrap();
    fs.set_owner("/home/alice/projekt", ALICE).unwrap();
    fs.set_principal(alice());
    fs.create("/home/alice/projekt/zprava.txt", Kind::File).unwrap();
    fs.write("/home/alice/projekt/zprava.txt", 0, b"v1").unwrap();
    fs.set_acl("/home/alice", acl(&[(BOB, rights::TRAVERSE, grant::THIS)])).unwrap();
    fs.set_acl("/home/alice/projekt", acl(&[(BOB, rights::VIEW, grant::BOTH)])).unwrap();
    fs.snapshot_of("/home/alice/projekt", "s1").unwrap();
    // Potom Alice soubor zneviditelni (soukromy popis).
    fs.set_acl(
        "/home/alice/projekt/zprava.txt",
        Some(Acl { flags: acl_flags::PROTECTED, grants: vec![] }),
    )
    .unwrap();
    fs.set_principal(bob());
    assert_eq!(fs.read("/home/alice/projekt/zprava.txt", 0, 2).err(), Some(Error::NotFound));
    // Ve snapshotu plati opravneni z doby snapshotu.
    assert_eq!(fs.snapshot_read_of("/home/alice/projekt", "s1", "/zprava.txt", 0, 2).unwrap(), b"v1");
    assert_eq!(fs.snapshots_of("/home/alice/projekt").unwrap().len(), 1);
    // Snapshot delat nesmi (MANAGE), diff take ne.
    assert_eq!(fs.snapshot_of("/home/alice/projekt", "s2").err(), Some(Error::AccessDenied));
    assert_eq!(fs.diff("/home/alice/projekt", "s1", None).err(), Some(Error::AccessDenied));
    fs.set_principal(None);
    assert_clean(&mut fs);
}

#[test]
fn invalid_descriptors_are_refused() {
    let mut fs = home_fs();
    // System neni prijemce grantu, prazdna prava ani neznamy priznak nejdou.
    for bad in [
        acl(&[(0, rights::READ, grant::THIS)]),
        acl(&[(BOB, 0, grant::THIS)]),
        acl(&[(BOB, rights::READ, 0)]),
        acl(&[(BOB, rights::READ, 4)]),
        Some(Acl { flags: 2, grants: vec![] }),
    ] {
        assert_eq!(fs.set_acl("/home", bad).err(), Some(Error::InvalidArgs));
    }
    let many = Acl { flags: 0, grants: (1..=257).map(|w| Grant { who: w, rights: 1, flags: 1 }).collect() };
    assert_eq!(fs.set_acl("/home", Some(many)).err(), Some(Error::InvalidArgs));
    assert_clean(&mut fs);
}
