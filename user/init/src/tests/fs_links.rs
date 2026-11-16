//! Symbolicke odkazy ve fsd: nasledovani v posledni i prostredni slozce,
//! relativni a absolutni cil, `..` po nasledovani, smycka (TOO_MANY_LINKS),
//! cil bez prava, cil mimo jmenny prostor relace, NOFOLLOW, lstat,
//! readlink, realpath, rename a unlink na odkazu (sada "souborovy system").

use fsproto::perm::{Acl, EVERYONE, Grant, Profile, grant, rights};
use fsproto::v2::{access, open_flags, share};
use fsproto::{FsClient, kind};
use libos::{Error, Startup};

use super::{Test, TestResult, devices, fail};
use crate::check_eq;

pub const TESTS: &[Test] = &[("symbolicke odkazy: nasledovani, smycka, prava, jmenny prostor relace", links), ("uklid: pracovni soubory sady", uklid)];

const ALICE: u64 = 1000;
const BOB: u64 = 1001;

fn fs() -> Result<&'static FsClient, alloc::string::String> {
    devices().and_then(|d| d.fs.as_ref()).ok_or_else(|| "sluzba fsd nebezi".into())
}

fn links(_: &Startup) -> TestResult {
    let f = fs()?;
    let traverse = Acl { flags: 0, grants: alloc::vec![Grant { who: EVERYONE, rights: rights::TRAVERSE, flags: grant::THIS }] };
    let mut g = super::restore::Restore::new(f);
    g.set_acl("/", Some(&traverse))?;
    let _ = f.create("/odkazy", kind::DIR);
    f.set_owner("/odkazy", ALICE).map_err(|e| fail("vlastnik", e))?;
    // Slozka bez grantu: Alice ji nevidi.
    let _ = f.create("/tajne", kind::DIR);
    let _ = f.create("/tajne/x", kind::FILE);
    let (_, a) = f.connect_as(ALICE, &[], &Profile::Standard.user_mounts()).map_err(|e| fail("relace alice", e))?;
    let _ = a.create("/odkazy/a", kind::DIR);
    let _ = a.create("/odkazy/a/b", kind::DIR);
    let _ = a.create("/odkazy/a/b/f", kind::FILE);
    a.write("/odkazy/a/b/f", 0, b"obsah").map_err(|e| fail("zapis f", e))?;
    let read = |p: &str| a.read(p, 0, 16);

    // Relativni cil od slozky odkazu, odkaz v prostredni slozce.
    a.symlink("/odkazy/rel", "a/b").map_err(|e| fail("symlink rel", e))?;
    check_eq!(read("/odkazy/rel/f"), Ok(b"obsah".to_vec()), "prostredni slozka");
    check_eq!(a.readlink("/odkazy/rel"), Ok(alloc::string::String::from("a/b")), "readlink");
    check_eq!(a.lstat("/odkazy/rel").map(|s| s.kind), Ok(kind::SYMLINK), "lstat = odkaz");
    check_eq!(a.stat("/odkazy/rel").map(|s| s.kind), Ok(kind::DIR), "stat nasleduje");
    check_eq!(a.symlink("/odkazy/rel", "jiny").err(), Some(Error::AlreadyExists), "existujici jmeno");
    // Absolutni cil v jmennem prostoru relace; `..` od fyzicke cesty.
    a.symlink("/odkazy/abs", "/odkazy/a").map_err(|e| fail("symlink abs", e))?;
    check_eq!(read("/odkazy/abs/b/f"), Ok(b"obsah".to_vec()), "absolutni cil");
    check_eq!(read("/odkazy/rel/../b/f"), Ok(b"obsah".to_vec()), ".. po nasledovani (od /odkazy/a/b)");
    check_eq!(a.realpath("/odkazy/rel/../../abs/b"), Ok(alloc::string::String::from("/odkazy/a/b")), "realpath");
    // Smycka a visici odkaz.
    a.symlink("/odkazy/s1", "s2").map_err(|e| fail("s1", e))?;
    a.symlink("/odkazy/s2", "s1").map_err(|e| fail("s2", e))?;
    check_eq!(a.stat("/odkazy/s1").err(), Some(Error::TooManyLinks), "smycka");
    a.symlink("/odkazy/visi", "neni").map_err(|e| fail("visi", e))?;
    check_eq!(a.stat("/odkazy/visi").err(), Some(Error::NotFound), "visici odkaz");
    check_eq!(a.lstat("/odkazy/visi").map(|s| s.kind), Ok(kind::SYMLINK), "lstat visiciho");
    // NOFOLLOW: odkaz na konci = TOO_MANY_LINKS (C ELOOP).
    check_eq!(a.open("/odkazy/abs", access::READ, share::ALL, open_flags::NOFOLLOW).err(), Some(Error::TooManyLinks), "NOFOLLOW");
    // Odkaz nic neprava: cil bez prava zustane neviditelny.
    a.symlink("/odkazy/tajne", "/tajne/x").map_err(|e| fail("tajne", e))?;
    check_eq!(a.stat("/odkazy/tajne").err(), Some(Error::NotFound), "cil bez prava");
    // Vytvoreni odkazu chce pravo vytvareni ve slozce.
    let (_, b) = f.connect_as(BOB, &[], &Profile::Standard.user_mounts()).map_err(|e| fail("relace bob", e))?;
    check_eq!(b.symlink("/odkazy/bob", "a").err(), Some(Error::NotFound), "Bob ve slozce Alice");
    // Prejmenovani a smazani pusobi na odkaz, ne na cil.
    a.rename("/odkazy/rel", "/odkazy/rel2").map_err(|e| fail("rename", e))?;
    check_eq!(a.lstat("/odkazy/rel2").map(|s| s.kind), Ok(kind::SYMLINK), "prejmenovany odkaz");
    a.remove("/odkazy/rel2").map_err(|e| fail("unlink", e))?;
    check_eq!(read("/odkazy/a/b/f"), Ok(b"obsah".to_vec()), "cil zustal");

    // Relace aplikace: cil mimo jeji pripojene cesty = NOT_FOUND.
    let _ = a.create("/odkazy/apps", kind::DIR);
    let _ = a.create("/odkazy/apps/ed", kind::DIR);
    let mounts = Profile::Standard.app_mounts("/odkazy", "ed", &[]);
    let (_, app) = f.connect_as(ALICE, &[], &mounts).map_err(|e| fail("relace aplikace", e))?;
    let _ = app.create("/app/p.txt", kind::FILE);
    app.symlink("/app/uvnitr", "/app/p.txt").map_err(|e| fail("uvnitr", e))?;
    check_eq!(app.stat("/app/uvnitr").map(|s| s.kind), Ok(kind::FILE), "cil v relaci");
    app.symlink("/app/ven", "/odkazy/a/b/f").map_err(|e| fail("ven", e))?;
    check_eq!(app.stat("/app/ven").err(), Some(Error::NotFound), "absolutni cil mimo relaci");
    app.symlink("/app/nahoru", "../../a/b/f").map_err(|e| fail("nahoru", e))?;
    check_eq!(app.stat("/app/nahoru").err(), Some(Error::NotFound), "relativni cil nad korenem relace");
    // Stejny odkaz v relaci uzivatele cil vidi (resi se v jejim prostoru).
    check_eq!(a.stat("/odkazy/apps/ed/ven").map(|s| s.kind), Ok(kind::FILE), "stejny odkaz v relaci uzivatele");
    for p in [
        "/odkazy/abs",
        "/odkazy/s1",
        "/odkazy/s2",
        "/odkazy/visi",
        "/odkazy/tajne",
        "/odkazy/apps/ed/uvnitr",
        "/odkazy/apps/ed/ven",
        "/odkazy/apps/ed/nahoru",
    ] {
        let _ = a.remove(p);
    }
    Ok(())
}

/// Posledni test sady: smaze odkazy a jejich cile (`state.rs` jinak sadu
/// oznaci za selhanou; bezi i po chybe drivejsich testu).
fn uklid(_: &Startup) -> TestResult {
    super::restore::remove_paths(&["/odkazy", "/tajne"])
}
