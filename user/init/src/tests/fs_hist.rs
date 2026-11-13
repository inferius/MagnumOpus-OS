//! Historie FS pres sluzbu fsd (faze 2): zurnal zmen a cekani na zmeny,
//! kos, commity, vetve a merge, "zkusit zmeny bokem", atributy; dotazy
//! shellu (vypis s metadaty, rozdil verzi, dataset cesty).

use fsproto::hist::jop;
use fsproto::perm::{Acl, EVERYONE, Grant, Mount, Profile, grant, rights};
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
    ("vypis s metadaty (LIST_STAT_PAGE)", list_stat),
    ("rozdil verzi po pohledu (DIFF) a dataset cesty", diff_dataset_of),
    ("vypis s metadaty a dataset cesty s omezenou relaci", restricted_session),
    ("dataset na existujici jmeno: svazek zustane zapisovatelny", dataset_again),
    ("seznam datasetu (DATASETS): prava a puvod klonu", super::fs_datasets::datasets),
    ("kontrola konzistence po historii", check_clean),
];

/// Druhy start sluzby uctu: dataset uz existuje (`ALREADY_EXISTS`) a dalsi
/// zapisy musi projit. Driv chyba svazek prepnula do stavu chyby a od
/// druheho startu systemu (VMware) vracel kazdy zapis `IO_ERROR`; testy
/// s novym datovym diskem pri kazdem behu na to neprisly.
fn dataset_again(_: &Startup) -> TestResult {
    let f = fs()?;
    match f.create_dataset("/f2-znovu") {
        Ok(_) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(fail("create_dataset", e)),
    }
    check_eq!(f.create_dataset("/f2-znovu").err(), Some(Error::AlreadyExists), "druhe zalozeni");
    let _ = f.create("/f2-znovu/po.txt", kind::FILE);
    f.write("/f2-znovu/po.txt", 0, b"po kolizi").map_err(|e| fail("zapis po kolizi", e))?;
    f.commit().map_err(|e| fail("commit po kolizi", e))?;
    Ok(())
}

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

/// `list_stat` vrati stejne polozky jako `list` a metadata jako `stat`.
fn list_stat(_: &Startup) -> TestResult {
    let f = fs()?;
    let _ = f.create("/f2-ls", kind::DIR);
    let _ = f.create("/f2-ls/a.txt", kind::FILE);
    f.write("/f2-ls/a.txt", 0, b"12345").map_err(|e| fail("zapis", e))?;
    let _ = f.create("/f2-ls/B.txt", kind::FILE);
    let _ = f.create("/f2-ls/sub", kind::DIR);
    let list = f.list("/f2-ls").map_err(|e| fail("list", e))?;
    let ls = f.list_stat("/f2-ls").map_err(|e| fail("list_stat", e))?;
    check_eq!(ls.len(), list.len(), "pocet polozek");
    for (e, s) in list.iter().zip(ls.iter()) {
        check_eq!((&s.name, s.kind, s.oid), (&e.name, e.kind, e.oid), "polozka jako list");
        let st = f.stat(&alloc::format!("/f2-ls/{}", e.name)).map_err(|e| fail("stat", e))?;
        check_eq!((s.size, s.mtime), (st.size, st.mtime), "metadata jako stat");
    }
    check!(ls.iter().any(|s| s.name == "a.txt" && s.size == 5 && s.kind == kind::FILE), "velikost a.txt");
    check!(ls.iter().any(|s| s.name == "sub" && s.kind == kind::DIR), "podslozka");
    check_eq!(f.list_stat("/f2-ls/a.txt").err(), Some(Error::NotADirectory), "vypis souboru");
    Ok(())
}

/// Rozdil verzi proti pohledu (`VIEW`) a mezi dvema pohledy v malem
/// datasetu; `dataset_of` uvnitr datasetu a v koreni.
fn diff_dataset_of(_: &Startup) -> TestResult {
    let f = fs()?;
    match f.create_dataset("/f2-diff") {
        Ok(_) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(fail("create_dataset", e)),
    }
    let _ = f.create("/f2-diff/stary.txt", kind::FILE);
    f.write("/f2-diff/stary.txt", 0, b"puvodni").map_err(|e| fail("zapis", e))?;
    let v1 = f.view("/f2-diff").map_err(|e| fail("view", e))?;
    f.create("/f2-diff/novy.txt", kind::FILE).map_err(|e| fail("create", e))?;
    f.write("/f2-diff/novy.txt", 0, b"novy").map_err(|e| fail("zapis", e))?;
    let d = f.diff("/f2-diff", &v1.snapshot, None).map_err(|e| fail("diff do zive verze", e))?;
    check!(d.iter().any(|c| c.path == "/novy.txt" && c.kind == 1 && !c.is_dir), alloc::format!("pridany soubor: {d:?}").as_str());
    check!(!d.iter().any(|c| c.path == "/stary.txt"), "nezmeneny soubor v rozdilu");
    let v2 = f.view("/f2-diff").map_err(|e| fail("view 2", e))?;
    let d2 = f.diff("/f2-diff", &v1.snapshot, Some(&v2.snapshot)).map_err(|e| fail("diff mezi pohledy", e))?;
    check_eq!(d2, d, "rozdil do pohledu = rozdil do zive verze");
    check_eq!(f.diff("/f2-diff", "neni-takovy", None).err(), Some(Error::NotFound), "neznamy snapshot");
    // Cteni z pohledu datasetu (SNAP_*_OF; cesty uvnitr datasetu).
    let st = f.snapshot_stat_of("/f2-diff", &v1.snapshot, "/stary.txt").map_err(|e| fail("snapshot_stat_of", e))?;
    check_eq!(st.size, 7, "velikost ve snapshotu");
    check_eq!(f.snapshot_stat_of("/f2-diff", &v1.snapshot, "/novy.txt").err(), Some(Error::NotFound), "novy soubor v pohledu neni");
    let names: alloc::vec::Vec<_> = f
        .snapshot_list_of("/f2-diff", &v1.snapshot, "/")
        .map_err(|e| fail("snapshot_list_of", e))?
        .into_iter()
        .map(|e| e.name)
        .collect();
    check!(names.iter().any(|n| n == "stary.txt") && !names.iter().any(|n| n == "novy.txt"), "vypis pohledu");
    check_eq!(
        f.snapshot_read_file_of("/f2-diff", &v1.snapshot, "/stary.txt").map_err(|e| fail("snapshot_read_file_of", e))?,
        b"puvodni".to_vec(),
        "obsah ve snapshotu"
    );
    check_eq!(
        f.snapshot_read_of("/f2-diff", &v2.snapshot, "/novy.txt", 1, 2).map_err(|e| fail("snapshot_read_of", e))?,
        b"ov".to_vec(),
        "cast souboru ve snapshotu"
    );
    f.release_view("/f2-diff", &v2.snapshot).map_err(|e| fail("release_view", e))?;
    f.release_view("/f2-diff", &v1.snapshot).map_err(|e| fail("release_view", e))?;
    // Dataset cesty.
    let _ = f.create("/f2-diff/sub", kind::DIR);
    let _ = f.create("/f2-diff/sub/y.txt", kind::FILE);
    check_eq!(f.dataset_of("/f2-diff/sub/y.txt").map_err(|e| fail("dataset_of", e))?, alloc::string::String::from("/f2-diff"), "soubor v datasetu");
    check_eq!(f.dataset_of("/f2-diff").map_err(|e| fail("dataset_of", e))?, alloc::string::String::from("/f2-diff"), "koren datasetu");
    check_eq!(f.dataset_of("/f2-ls/a.txt").map_err(|e| fail("dataset_of", e))?, alloc::string::String::from("/"), "soubor v koreni");
    check_eq!(f.dataset_of("/").map_err(|e| fail("dataset_of", e))?, alloc::string::String::from("/"), "koren");
    check_eq!(f.dataset_of("/f2-diff/neni").err(), Some(Error::NotFound), "neexistujici cesta");
    f.delete_dataset("/f2-diff").map_err(|e| fail("delete_dataset", e))?;
    Ok(())
}

/// Uzivatel bez prav k vlastnimu adresari.
const CAROL: u64 = 1002;

/// `LIST_STAT_PAGE` nevrati polozku, kterou relace nevidi; `DATASET_OF`
/// pro neviditelnou cestu a pro koren mimo pripojene cesty = `NOT_FOUND`.
fn restricted_session(_: &Startup) -> TestResult {
    let f = fs()?;
    // Koren jde projit (stejne jako v testech opravneni FS).
    let traverse = Acl { flags: 0, grants: alloc::vec![Grant { who: EVERYONE, rights: rights::TRAVERSE, flags: grant::THIS }] };
    f.set_acl("/", Some(&traverse)).map_err(|e| fail("acl /", e))?;
    let _ = f.create("/f2-omez", kind::DIR);
    let dir_acl = Acl {
        flags: 0,
        grants: alloc::vec![Grant { who: CAROL, rights: rights::LIST | rights::TRAVERSE, flags: grant::THIS }],
    };
    f.set_acl("/f2-omez", Some(&dir_acl)).map_err(|e| fail("acl slozky", e))?;
    let _ = f.create("/f2-omez/moje.txt", kind::FILE);
    f.write("/f2-omez/moje.txt", 0, b"carol").map_err(|e| fail("zapis", e))?;
    f.set_owner("/f2-omez/moje.txt", CAROL).map_err(|e| fail("vlastnik", e))?;
    let _ = f.create("/f2-omez/cizi.txt", kind::FILE);
    let (_, carol) = f.connect_as(CAROL, &[], &Profile::Standard.user_mounts()).map_err(|e| fail("connect_as", e))?;
    let list: alloc::vec::Vec<_> = carol.list("/f2-omez").map_err(|e| fail("list", e))?.into_iter().map(|e| e.name).collect();
    let ls = carol.list_stat("/f2-omez").map_err(|e| fail("list_stat", e))?;
    let names: alloc::vec::Vec<_> = ls.iter().map(|e| e.name.clone()).collect();
    check_eq!(names, list, "stejne polozky jako list");
    check_eq!(names, alloc::vec![alloc::string::String::from("moje.txt")], "jen viditelna polozka");
    check_eq!(ls[0].size, 5, "velikost viditelne polozky");
    // Systemova relace vidi obe.
    check_eq!(f.list_stat("/f2-omez").map_err(|e| fail("list_stat systemu", e))?.len(), 2, "system vidi vse");
    check_eq!(carol.dataset_of("/f2-omez/moje.txt").map_err(|e| fail("dataset_of", e))?, alloc::string::String::from("/"), "viditelna cesta");
    check_eq!(carol.dataset_of("/f2-omez/cizi.txt").err(), Some(Error::NotFound), "neviditelna cesta");
    // Relace aplikace jen s /app -> /f2-omez: koren datasetu "/" nevidi.
    let mounts = [Mount::new("/app", "/f2-omez", 0)];
    let (_, app) = f.connect_as(CAROL, &[], &mounts).map_err(|e| fail("relace aplikace", e))?;
    let names: alloc::vec::Vec<_> = app.list_stat("/app").map_err(|e| fail("list_stat aplikace", e))?.into_iter().map(|e| e.name).collect();
    check_eq!(names, alloc::vec![alloc::string::String::from("moje.txt")], "vypis pres pripojenou cestu");
    check_eq!(app.dataset_of("/app/moje.txt").err(), Some(Error::NotFound), "koren mimo pripojene cesty");
    check_eq!(app.dataset_of("/f2-omez/moje.txt").err(), Some(Error::NotFound), "cesta mimo pripojene");
    drop(app);
    drop(carol);
    f.remove("/f2-omez/cizi.txt").map_err(|e| fail("uklid", e))?;
    f.remove("/f2-omez/moje.txt").map_err(|e| fail("uklid", e))?;
    f.remove("/f2-omez").map_err(|e| fail("uklid", e))?;
    Ok(())
}
