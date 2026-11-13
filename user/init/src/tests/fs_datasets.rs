//! Seznam datasetu (`DATASETS`, `fsproto::datasets`): viditelnost podle
//! prav a pripojenych cest, `Stat` korene s maskami dostupnosti a puvod
//! klonu ve trech stavech (znamy / skryty / neexistuje).

use alloc::string::String;
use alloc::vec::Vec;

use fsproto::datasets::{DatasetEntry, Origin};
use fsproto::perm::{Acl, EVERYONE, Grant, Mount, Profile, grant, rights};
use fsproto::{FsClient, avail};
use libos::{Error, Startup};

use super::{TestResult, devices, fail};
use crate::{check, check_eq};

/// Uzivatel bez prav k puvodnimu datasetu.
const DAVE: u64 = 1003;

fn fs() -> Result<&'static FsClient, String> {
    devices().and_then(|d| d.fs.as_ref()).ok_or_else(|| "sluzba fsd nebezi".into())
}

fn find<'a>(list: &'a [DatasetEntry], path: &str) -> Option<&'a DatasetEntry> {
    list.iter().find(|d| d.path == path)
}

fn paths(list: &[DatasetEntry]) -> Vec<String> {
    list.iter().map(|d| d.path.clone()).collect()
}

pub fn datasets(_: &Startup) -> TestResult {
    let f = fs()?;
    for p in ["/f2-dsl-b", "/f2-dsl-a"] {
        let _ = f.delete_dataset(p);
    }
    f.create_dataset("/f2-dsl-a").map_err(|e| fail("create_dataset", e))?;
    f.aside_begin("/f2-dsl-a", "/f2-dsl-b").map_err(|e| fail("klon (aside_begin)", e))?;

    // System: koren, puvodni dataset i klon se znamym puvodem.
    let all = f.datasets().map_err(|e| fail("datasets", e))?;
    let root = find(&all, "/").ok_or_else(|| alloc::format!("chybi koren: {:?}", paths(&all)))?;
    check_eq!(root.origin, Origin::None, "koren neni klon");
    check!(root.root.known & avail::TREE != 0, "system zna souhrn korene");
    check_eq!(find(&all, "/f2-dsl-a").map(|d| d.origin.clone()), Some(Origin::None), "puvodni dataset");
    let b = find(&all, "/f2-dsl-b").ok_or("chybi klon")?;
    check_eq!(b.origin, Origin::Known(String::from("/f2-dsl-a")), "puvod klonu");
    check!(paths(&all).windows(2).all(|w| w[0] <= w[1]), "serazeno podle cesty");

    // Uzivatel s pravem jen ke klonu: puvod je skryty, souhrn korene take
    // (bez MANAGE), puvodni dataset v seznamu neni.
    let traverse = Acl { flags: 0, grants: alloc::vec![Grant { who: EVERYONE, rights: rights::TRAVERSE, flags: grant::THIS }] };
    f.set_acl("/", Some(&traverse)).map_err(|e| fail("acl /", e))?;
    let own = Acl { flags: 0, grants: alloc::vec![Grant { who: DAVE, rights: rights::LIST | rights::TRAVERSE, flags: grant::THIS }] };
    f.set_acl("/f2-dsl-b", Some(&own)).map_err(|e| fail("acl klonu", e))?;
    let (_, dave) = f.connect_as(DAVE, &[], &Profile::Standard.user_mounts()).map_err(|e| fail("connect_as", e))?;
    let seen = dave.datasets().map_err(|e| fail("datasets uzivatele", e))?;
    check!(find(&seen, "/f2-dsl-a").is_none(), "puvodni dataset neni videt");
    let b = find(&seen, "/f2-dsl-b").ok_or_else(|| alloc::format!("klon chybi: {:?}", paths(&seen)))?;
    check_eq!(b.origin, Origin::Hidden, "puvod skryty");
    check!(b.root.hidden & avail::TREE != 0 && b.root.known & avail::TREE == 0, "souhrn skryty bez MANAGE");

    // Relace aplikace s jedinou pripojenou cestou: cesta relace, bez korene.
    let (_, app) = f.connect_as(DAVE, &[], &[Mount::new("/app", "/f2-dsl-b", 0)]).map_err(|e| fail("relace aplikace", e))?;
    let seen = app.datasets().map_err(|e| fail("datasets aplikace", e))?;
    check_eq!(paths(&seen), alloc::vec![String::from("/app")], "jen pripojeny dataset");
    drop(app);
    drop(dave);

    // Smazani puvodu s zivym klonem FS dnes odmitne (`BAD_STATE`); stav
    // "puvod neexistuje" je pojistka pro budoucnost a pro poskozeny zaznam.
    match f.delete_dataset("/f2-dsl-a") {
        Ok(()) => {
            let all = f.datasets().map_err(|e| fail("datasets po smazani", e))?;
            check_eq!(find(&all, "/f2-dsl-b").map(|d| d.origin.clone()), Some(Origin::Gone), "puvod neexistuje");
        }
        Err(Error::BadState) => {}
        Err(e) => return Err(fail("delete_dataset puvodu", e)),
    }
    f.delete_dataset("/f2-dsl-b").map_err(|e| fail("uklid klonu", e))?;
    let _ = f.delete_dataset("/f2-dsl-a");
    Ok(())
}
