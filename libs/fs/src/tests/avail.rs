//! Dostupnost hodnot ve `Stat` (masky `known` a `hidden`): znama hodnota,
//! skryta pro nedostatek prav a nedostupna.

use super::{assert_clean, new_fs};
use crate::{Acl, EVERYONE, Grant, Kind, Principal, avail, grant, rights};

const ALICE: u64 = 1000;
const BOB: u64 = 1001;

fn acl(grants: &[(u64, u8, u8)]) -> Option<Acl> {
    Some(Acl {
        flags: 0,
        grants: grants.iter().map(|&(who, rights, flags)| Grant { who, rights, flags }).collect(),
    })
}

#[test]
fn file_and_dir_values_are_known() {
    let mut fs = new_fs(4096);
    fs.create("/prazdna", Kind::Dir).unwrap();
    fs.create("/a.txt", Kind::File).unwrap();
    fs.write("/a.txt", 0, b"abc").unwrap();
    // Prazdna slozka: nuly jsou zname hodnoty, ne skryte.
    let d = fs.stat("/prazdna").unwrap();
    assert_eq!((d.tree_bytes, d.tree_files, d.tree_dirs), (0, 0, 0));
    assert_eq!((d.known, d.hidden), (avail::ALL, 0));
    // Soubor: souhrny podstromu k nemu nepatri (nedostupne).
    let f = fs.stat("/a.txt").unwrap();
    assert_eq!((f.known, f.hidden), (avail::ALL & !avail::TREE, 0));
    assert_clean(&mut fs);
}

#[test]
fn tree_summary_without_manage_is_hidden() {
    let mut fs = new_fs(8192);
    fs.set_acl("/", acl(&[(EVERYONE, rights::TRAVERSE, grant::THIS)])).unwrap();
    fs.create("/home", Kind::Dir).unwrap();
    fs.set_owner("/home", ALICE).unwrap();
    fs.set_principal(Some(Principal::user(ALICE, &[])));
    fs.create("/home/prazdna", Kind::Dir).unwrap();
    fs.create("/home/plna", Kind::Dir).unwrap();
    fs.create("/home/plna/a.txt", Kind::File).unwrap();
    fs.write("/home/plna/a.txt", 0, b"tajne").unwrap();
    fs.set_acl("/home", acl(&[(BOB, rights::EDIT, grant::BOTH)])).unwrap();
    // Vlastnik vidi souhrny (MANAGE).
    let own = fs.stat("/home/plna").unwrap();
    assert_eq!((own.tree_bytes, own.tree_files), (5, 1));
    assert_eq!((own.known, own.hidden), (avail::ALL, 0));
    fs.set_principal(Some(Principal::user(BOB, &[])));
    let tree = avail::SIZE | avail::TREE;
    // Bez MANAGE: plna i prazdna slozka vypadaji stejne - nuly a skryte.
    for path in ["/home/plna", "/home/prazdna"] {
        let st = fs.stat(path).unwrap();
        assert_eq!(st.rights & rights::MANAGE, 0, "{path}");
        assert_eq!((st.tree_bytes, st.tree_files, st.tree_dirs), (0, 0, 0), "{path}");
        assert_eq!((st.known, st.hidden), (avail::ALL & !tree, tree), "{path}");
    }
    // Soubor bez MANAGE: velikost je videt (pravo ke cteni), souhrny
    // k souboru nepatri - nic skryteho.
    let f = fs.stat("/home/plna/a.txt").unwrap();
    assert_eq!(f.size, 5);
    assert_eq!((f.known, f.hidden), (avail::ALL & !avail::TREE, 0));
    // Handle dava stejne masky jako stat cesty.
    let h = fs.open("/home/plna", crate::access::READ, crate::share::ALL).unwrap();
    let hs = fs.stat_h(h).unwrap();
    assert_eq!((hs.known, hs.hidden), (avail::ALL & !tree, tree));
    fs.close(h).unwrap();
    fs.set_principal(None);
    assert_clean(&mut fs);
}

#[test]
fn dataset_boundary_values_are_unavailable() {
    let mut fs = new_fs(8192);
    fs.create_dataset("/repo").unwrap();
    fs.create("/tmp", Kind::Dir).unwrap();
    // Klon korene: vnoreny dataset /repo je v nem jen hranice.
    fs.aside_begin("/", "/tmp/w").unwrap();
    let st = fs.stat("/tmp/w/repo").unwrap();
    assert_eq!(st.kind, Kind::Dataset);
    // Casy, velikost, vlastnik a souhrny FS u hranice nevede.
    assert_eq!((st.known, st.hidden), (avail::RIGHTS, 0));
    fs.aside_discard("/tmp/w").unwrap();
    assert_clean(&mut fs);
}
