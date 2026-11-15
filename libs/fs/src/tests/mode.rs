//! Mod POSIX jako pohled na granty (`mode.rs`): cista logika.

use alloc::vec;

use crate::mode::*;
use crate::perm::{Acl, EVERYONE, Grant, Inherited, acl_flags, grant, rights};

const OWNER: u64 = 1000;
const USERS: u64 = 100;

fn inh(grants: &[(u64, u8)]) -> Inherited {
    Inherited { grants: grants.iter().map(|&(who, rights)| Grant { who, rights, flags: grant::BOTH }).collect() }
}

#[test]
fn bits_and_rights() {
    assert_eq!(bits_of(rights::READ | rights::WRITE, false), 6);
    assert_eq!(bits_of(rights::LIST | rights::TRAVERSE, true), 5);
    assert_eq!(bits_of(rights::CREATE, true), 2);
    assert_eq!(rights_of(7, false), rights::READ | rights::WRITE | rights::EXECUTE);
    assert_eq!(rights_of(5, true), rights::LIST | rights::READ | rights::TRAVERSE);
    for b in 0..8 {
        assert_eq!(bits_of(rights_of(b, true), true), b);
        assert_eq!(bits_of(rights_of(b, false), false), b);
    }
}

#[test]
fn mode_is_view_of_grants() {
    // Vlastnik vzdy rw- (soubor) / rwx (adresar); group = jine granty, other = EVERYONE.
    assert_eq!(mode_of(OWNER, false, &[]), 0o600);
    assert_eq!(mode_of(OWNER, true, &[]), 0o700);
    let g = [
        Grant { who: USERS, rights: rights::LIST | rights::CREATE | rights::TRAVERSE, flags: grant::BOTH },
        Grant { who: EVERYONE, rights: rights::LIST | rights::TRAVERSE, flags: grant::THIS },
        Grant { who: OWNER, rights: rights::READ, flags: grant::THIS },
    ];
    assert_eq!(mode_of(OWNER, true, &g), 0o775);
}

#[test]
fn create_mode_only_narrows() {
    // /tmp: skupina uzivatelu LIST|CREATE (dedi se); mkdir(..., 0700).
    let above = inh(&[(USERS, rights::LIST | rights::CREATE | rights::TRAVERSE)]);
    let acl = apply(OWNER, true, &above, None, 0o700, false).expect("zuzeni");
    assert!(acl.flags & acl_flags::PROTECTED != 0);
    assert!(acl.grants.is_empty(), "skupina bez bitu zmizi");
    assert_eq!(mode_of(OWNER, true, &effective(&above, Some(&acl))), 0o700);
    // Mod sirsi nez zdedene granty nic nepridava (a nic nemeni).
    assert_eq!(apply(OWNER, true, &above, None, 0o777, false), None);
    // Castecne zuzeni: skupina jen r-x.
    let acl = apply(OWNER, true, &above, None, 0o750, false).unwrap();
    let g = effective(&above, Some(&acl));
    assert_eq!(mode_of(OWNER, true, &g), 0o750);
    assert!(g.iter().all(|x| x.rights & rights::CREATE == 0));
    // Bez zdedenych grantu: beze zmeny.
    assert_eq!(apply(OWNER, false, &Inherited::default(), None, 0o600, false), None);
}

#[test]
fn chmod_may_widen() {
    let above = Inherited::default();
    // chmod o+r souboru: pribude grant EVERYONE jen pro objekt.
    let acl = apply(OWNER, false, &above, None, 0o644, true).unwrap();
    assert_eq!(acl.grants, vec![Grant { who: EVERYONE, rights: rights::READ, flags: grant::THIS }]);
    assert_eq!(mode_of(OWNER, false, &effective(&above, Some(&acl))), 0o604, "group bez prijemce zustane 0");
    // A zpet: chmod 600 grant zrusi.
    let back = apply(OWNER, false, &above, Some(&acl), 0o600, true).unwrap();
    assert!(effective(&above, Some(&back)).is_empty());
    // Prava mimo mod (MANAGE) trida s bity podrzi.
    let own = Acl { flags: 0, grants: vec![Grant { who: USERS, rights: rights::READ | rights::MANAGE, flags: grant::THIS }] };
    let n = apply(OWNER, false, &above, Some(&own), 0o660, true).unwrap();
    let g = effective(&above, Some(&n));
    assert_eq!(g[0].rights, rights::READ | rights::WRITE | rights::MANAGE);
    // Granty jen pro potomky adresare mod nemeni.
    let kids = Acl { flags: 0, grants: vec![Grant { who: USERS, rights: rights::READ, flags: grant::INHERIT }] };
    assert_eq!(apply(OWNER, true, &above, Some(&kids), 0o700, true), None);
}
