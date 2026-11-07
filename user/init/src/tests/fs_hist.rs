//! Historie FS pres sluzbu fsd (faze 2): zurnal zmen a cekani na zmeny,
//! kos, commity, vetve a merge, "zkusit zmeny bokem", atributy.

use fsproto::hist::jop;
use fsproto::{FsClient, kind};
use libos::{Error, Startup};

use super::{Test, TestResult, devices, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("zurnal zmen a cekani na zmeny (WATCH)", journal_watch),
    ("kos: smazat, obnovit, vysypat", trash),
    ("commit, vetev a merge", branch_merge),
    ("zkusit zmeny bokem: prevzit a zahodit", aside),
    ("rozsirene atributy a expirace", xattrs),
    ("kontrola konzistence po historii", check_clean),
];

fn fs() -> Result<&'static FsClient, alloc::string::String> {
    devices()
        .and_then(|d| d.fs.as_ref())
        .ok_or_else(|| "sluzba fsd nebezi".into())
}

fn journal_watch(_: &Startup) -> TestResult {
    let f = fs()?;
    f.commit().map_err(|e| fail("commit", e))?;
    // Vychozi pozice z pohledu (zurnal mohl byt prorezany retenci).
    let v = f.view("/").map_err(|e| fail("view", e))?;
    f.release_view("/", &v.snapshot).map_err(|e| fail("release_view", e))?;
    let last = v.seq;
    f.create("/f2-zurnal.txt", kind::FILE).map_err(|e| fail("create", e))?;
    f.write("/f2-zurnal.txt", 0, b"obsah").map_err(|e| fail("zapis", e))?;
    // Pred potvrzenim cekani vyprsi prazdne.
    let w = f.connect().map_err(|e| fail("connect", e))?;
    let p = w.journal_watch("/", last + 1, 100, 50).map_err(|e| fail("watch (limit)", e))?;
    let names: alloc::vec::Vec<_> = p.entries.iter().filter(|e| e.name == "f2-zurnal.txt").collect();
    check!(names.is_empty(), "zmena bez potvrzeni nesmi byt videt");
    f.fsync().map_err(|e| fail("fsync", e))?;
    let p = w.journal_watch("/", last + 1, 100, 1000).map_err(|e| fail("watch", e))?;
    check!(p.entries.iter().any(|e| e.op == jop::CREATE && e.name == "f2-zurnal.txt"), "CREATE v zurnalu");
    check!(p.entries.iter().any(|e| e.op == jop::CONTENT), "CONTENT v zurnalu");
    // Pohled: snapshot + pozice.
    let v = f.view("/").map_err(|e| fail("view", e))?;
    check!(v.seq >= p.entries.last().map_or(0, |e| e.seq), "pozice pohledu");
    f.snapshot_stat(&v.snapshot, "/f2-zurnal.txt").map_err(|e| fail("soubor v pohledu", e))?;
    f.release_view("/", &v.snapshot).map_err(|e| fail("release_view", e))?;
    Ok(())
}

fn trash(_: &Startup) -> TestResult {
    let f = fs()?;
    let _ = f.create("/f2-kos", kind::DIR);
    f.create("/f2-kos/smazat.txt", kind::FILE).map_err(|e| fail("create", e))?;
    f.write("/f2-kos/smazat.txt", 0, b"neztratit").map_err(|e| fail("zapis", e))?;
    let id = f.trash("/f2-kos/smazat.txt").map_err(|e| fail("trash", e))?;
    check_eq!(f.stat("/f2-kos/smazat.txt").err(), Some(Error::NotFound), "po smazani do kose");
    let list = f.trash_list("/").map_err(|e| fail("trash_list", e))?;
    check!(list.iter().any(|e| e.id == id && e.original_dir == "/f2-kos"), "zaznam v kosi");
    let p = f.trash_restore("/", id, None).map_err(|e| fail("restore", e))?;
    check_eq!(p.as_str(), "/f2-kos/smazat.txt", "obnoveno na puvodni misto");
    check_eq!(f.read("/f2-kos/smazat.txt", 0, 20).map_err(|e| fail("cteni", e))?, b"neztratit".to_vec(), "obsah");
    let id = f.trash("/f2-kos").map_err(|e| fail("trash slozky", e))?;
    f.trash_purge("/", id).map_err(|e| fail("purge", e))?;
    check!(!f.trash_list("/").map_err(|e| fail("trash_list", e))?.iter().any(|e| e.id == id), "vysypano");
    Ok(())
}

/// Commit, vetev a merge v malem datasetu (cely koren by byl v QEMU pomaly).
fn branch_merge(_: &Startup) -> TestResult {
    let f = fs()?;
    match f.create_dataset("/f2-repo") {
        Ok(_) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(fail("create_dataset", e)),
    }
    let _ = f.create("/f2-repo/a.txt", kind::FILE);
    f.write("/f2-repo/a.txt", 0, b"radek 1
radek 2
").map_err(|e| fail("zapis", e))?;
    let c1 = f.commit_version("/f2-repo", "zaklad").map_err(|e| fail("commit", e))?;
    let head = f.head("/f2-repo").map_err(|e| fail("head", e))?;
    check_eq!(head.map(|h| h.id), Some(c1.id), "posledni commit");
    f.branch("/f2-repo", "f2-vetev", "/f2-vetev").map_err(|e| fail("branch", e))?;
    let b = f.branches("/f2-repo").map_err(|e| fail("branches", e))?;
    check!(b.iter().any(|b| b.name == "f2-vetev" && b.path == "/f2-vetev"), "vetev ve vypisu");
    f.write("/f2-vetev/a.txt", 0, b"radek 1
radek 2
radek 3
").map_err(|e| fail("zapis ve vetvi", e))?;
    f.create("/f2-vetev/novy.txt", kind::FILE).map_err(|e| fail("novy ve vetvi", e))?;
    let cb = f.commit_version("/f2-vetev", "vetev").map_err(|e| fail("commit vetve", e))?;
    check_eq!(cb.parents.clone(), alloc::vec![c1.id], "rodic vetve");
    let r = f.merge("/f2-repo", "/f2-vetev").map_err(|e| fail("merge", e))?;
    check!(r.conflicts.is_empty(), alloc::format!("konflikty: {:?}", r.conflicts).as_str());
    check_eq!(r.commit.as_ref().map(|c| c.parents.len()), Some(2), "commit slouceni");
    check_eq!(f.read("/f2-repo/a.txt", 0, 100).map_err(|e| fail("cteni", e))?, b"radek 1
radek 2
radek 3
".to_vec(), "slouceny obsah");
    f.stat("/f2-repo/novy.txt").map_err(|e| fail("novy soubor po merge", e))?;
    let log = f.log("/f2-repo", 10).map_err(|e| fail("log", e))?;
    check!(log.len() >= 3, "historie commitu");
    f.delete_branch("/f2-repo", "f2-vetev").map_err(|e| fail("delete_branch", e))?;
    check_eq!(f.stat("/f2-vetev").err(), Some(Error::NotFound), "vetev zrusena");
    Ok(())
}

fn aside(_: &Startup) -> TestResult {
    let f = fs()?;
    let _ = f.create("/f2-repo/bokem.txt", kind::FILE);
    f.write("/f2-repo/bokem.txt", 0, b"original").map_err(|e| fail("zapis", e))?;
    f.aside_begin("/f2-repo", "/f2-pokus").map_err(|e| fail("aside_begin", e))?;
    f.truncate("/f2-pokus/bokem.txt", 0).map_err(|e| fail("zkraceni v klonu", e))?;
    f.write("/f2-pokus/bokem.txt", 0, b"ZMENENO").map_err(|e| fail("zmena v klonu", e))?;
    let ch = f.aside_changes("/f2-pokus").map_err(|e| fail("aside_changes", e))?;
    check!(ch.iter().any(|c| c.path == "/bokem.txt"), "zmena v prehledu");
    check_eq!(f.read("/f2-repo/bokem.txt", 0, 20).map_err(|e| fail("cteni", e))?, b"original".to_vec(), "original nezmeneny");
    let r = f.aside_apply("/f2-pokus", Some(&["/bokem.txt"])).map_err(|e| fail("aside_apply", e))?;
    check!(r.conflicts.is_empty(), "bez konfliktu");
    check_eq!(f.read("/f2-repo/bokem.txt", 0, 20).map_err(|e| fail("cteni", e))?, b"ZMENENO".to_vec(), "prevzato");
    f.aside_discard("/f2-pokus").map_err(|e| fail("aside_discard", e))?;
    check_eq!(f.stat("/f2-pokus").err(), Some(Error::NotFound), "klon zahozen");
    f.delete_dataset("/f2-repo").map_err(|e| fail("delete_dataset", e))?;
    Ok(())
}

fn xattrs(_: &Startup) -> TestResult {
    let f = fs()?;
    let _ = f.create("/f2-attr.txt", kind::FILE);
    f.set_xattr("/f2-attr.txt", "user.barva", b"modra").map_err(|e| fail("set_xattr", e))?;
    check_eq!(f.get_xattr("/f2-attr.txt", "user.barva").map_err(|e| fail("get_xattr", e))?, b"modra".to_vec(), "hodnota");
    check!(f.list_xattrs("/f2-attr.txt").map_err(|e| fail("list", e))?.iter().any(|n| n == "user.barva"), "ve vypisu");
    let when = libos::time::WallTime { secs: 4_000_000_000, nanos: 0 };
    f.set_expires("/f2-attr.txt", Some(when)).map_err(|e| fail("set_expires", e))?;
    check_eq!(f.expires("/f2-attr.txt").map_err(|e| fail("expires", e))?, Some(when), "datum expirace");
    f.set_expires("/f2-attr.txt", None).map_err(|e| fail("zrusit expiraci", e))?;
    f.remove_xattr("/f2-attr.txt", "user.barva").map_err(|e| fail("remove_xattr", e))?;
    Ok(())
}

fn check_clean(_: &Startup) -> TestResult {
    let problems = fs()?.check(true).map_err(|e| fail("kontrola", e))?;
    check!(problems.is_empty(), alloc::format!("problemy: {problems:?}").as_str());
    Ok(())
}
