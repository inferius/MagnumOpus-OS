//! Souborovy system pres sluzbu fsd (datovy disk pripravi xtask).

use alloc::vec::Vec;

use fsproto::perm::{Acl, EVERYONE, Grant, Mount, Profile, grant, mount_flags, rights};
use fsproto::v2::{access, lock, open_flags, share};
use fsproto::{FsClient, kind};
use libos::{Error, Startup};

use super::{Test, TestResult, devices, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("fsd bezi nad datovym diskem", mounted),
    ("soubor zapsany na hostiteli (xtask)", host_file),
    ("zapis a cteni vic zprav (100 KiB)", roundtrip),
    ("jmena bez ohledu na velikost pismen", case_insensitive),
    ("okamzite velikosti adresaru", dir_sizes),
    ("prejmenovani a mazani", rename_remove),
    ("chybove kody", errors),
    ("snapshot: stara verze a smazani", snapshots),
    ("fsync prezije vypadek proudu", fsync_power_loss),
    ("sekvencni zapis = jeden souvisly usek", contiguous),
    ("velky adresar: vypis po strankach", big_directory),
    ("v2: dve relace, ctenar po atomickem nahrazeni", v2_two_sessions),
    ("v2: podminene ulozeni a konflikt", v2_conditional_save),
    ("v2: docasne jmeno nikdy nesmaze cizi soubor", v2_temp_collision),
    ("v2: konec relace uklidi handly, zamky mezi relacemi", v2_session_cleanup_and_locks),
    ("v2: 200 soucasnych relaci a proud (smycka pres port)", v2_many_sessions),
    ("opravneni: identita relace, pripojene cesty, GRANT", perm_sessions),
    ("opravneni: zakaz spousteni a sdileni mezi uzivateli", perm_noexec_and_sharing),
    ("kontrola konzistence a scrub", check_clean),
];

fn fs() -> Result<&'static FsClient, alloc::string::String> {
    devices()
        .and_then(|d| d.fs.as_ref())
        .ok_or_else(|| "sluzba fsd nebezi".into())
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u32).wrapping_mul(2_654_435_761).to_le_bytes()[1] ^ seed)
        .collect()
}

fn mounted(_: &Startup) -> TestResult {
    let info = fs()?.info().map_err(|e| fail("info", e))?;
    check_eq!(info.label.as_str(), "data", "jmeno svazku");
    check_eq!(info.block_size, 4096, "velikost bloku");
    check!(info.free_blocks > 0, "zadne volne misto");
    Ok(())
}

fn host_file(_: &Startup) -> TestResult {
    let f = fs()?;
    let data = f
        .read("/vitejte.txt", 0, 1000)
        .map_err(|e| fail("cteni", e))?;
    check_eq!(&data[..], b"Ahoj z hostitele!\n", "obsah /vitejte.txt");
    let st = f.stat("/dokumenty/navod.md").map_err(|e| fail("stat", e))?;
    check_eq!(st.kind, kind::FILE, "druh");
    Ok(())
}

fn roundtrip(_: &Startup) -> TestResult {
    let f = fs()?;
    let data = pattern(100 * 1024, 5);
    f.create("/test-roundtrip", kind::FILE)
        .map_err(|e| fail("create", e))?;
    f.write("/test-roundtrip", 0, &data)
        .map_err(|e| fail("zapis", e))?;
    let back = f
        .read("/test-roundtrip", 0, data.len() + 10)
        .map_err(|e| fail("cteni", e))?;
    check!(back == data, "prectena data nesouhlasi");
    f.commit().map_err(|e| fail("commit", e))?;
    let part = f
        .read("/test-roundtrip", 50_000, 100)
        .map_err(|e| fail("cteni po commitu", e))?;
    check!(
        part[..] == data[50_000..50_100],
        "data po commitu nesouhlasi"
    );
    Ok(())
}

fn case_insensitive(_: &Startup) -> TestResult {
    let f = fs()?;
    f.create("/Fotky", kind::DIR)
        .map_err(|e| fail("mkdir", e))?;
    f.create("/fotky/Dovolená.JPG", kind::FILE)
        .map_err(|e| fail("create", e))?;
    f.stat("/FOTKY/DOVOLENÁ.jpg")
        .map_err(|e| fail("stat jinou velikosti pismen", e))?;
    let list = f.list("/fotky").map_err(|e| fail("list", e))?;
    check_eq!(list.len(), 1, "pocet polozek");
    check_eq!(list[0].name.as_str(), "Dovolená.JPG", "zachovane jmeno");
    check_eq!(
        f.create("/FOTKY", kind::DIR).err(),
        Some(Error::AlreadyExists),
        "duplicitni jmeno"
    );
    Ok(())
}

fn dir_sizes(_: &Startup) -> TestResult {
    let f = fs()?;
    f.create("/velikosti", kind::DIR)
        .map_err(|e| fail("mkdir", e))?;
    f.create("/velikosti/a", kind::DIR)
        .map_err(|e| fail("mkdir a", e))?;
    for (i, n) in [1000usize, 5000, 70_000].iter().enumerate() {
        let p = alloc::format!("/velikosti/a/f{i}");
        f.create(&p, kind::FILE).map_err(|e| fail("create", e))?;
        f.write(&p, 0, &pattern(*n, 1))
            .map_err(|e| fail("zapis", e))?;
    }
    let st = f.stat("/velikosti").map_err(|e| fail("stat", e))?;
    check_eq!(
        (st.tree_bytes, st.tree_files, st.tree_dirs),
        (76_000, 3, 1),
        "souhrn podstromu"
    );
    // Masky dostupnosti po drate: adresar vse zname, soubor bez souhrnu,
    // prazdna slozka zname nuly (ne skryte).
    use fsproto::avail;
    check_eq!((st.known, st.hidden), (avail::ALL, 0), "masky adresare");
    let fst = f.stat("/velikosti/a/f0").map_err(|e| fail("stat souboru", e))?;
    check_eq!((fst.known, fst.hidden), (avail::ALL & !avail::TREE, 0), "masky souboru");
    f.create("/velikosti/prazdna", kind::DIR).map_err(|e| fail("mkdir prazdna", e))?;
    let list = f.list_stat("/velikosti").map_err(|e| fail("list_stat", e))?;
    let e = list
        .iter()
        .find(|e| e.name == "prazdna")
        .ok_or_else(|| fail("prazdna ve vypisu", Error::NotFound))?;
    // Polozka vypisu nese jen bity svych poli.
    let entry_bits = avail::SIZE | avail::MTIME | avail::RIGHTS | avail::TREE;
    check_eq!(
        (e.tree_files, e.tree_dirs, e.known, e.hidden),
        (0, 0, entry_bits, 0),
        "masky prazdne slozky"
    );
    Ok(())
}

fn rename_remove(_: &Startup) -> TestResult {
    let f = fs()?;
    f.create("/presun", kind::FILE)
        .map_err(|e| fail("create", e))?;
    f.write("/presun", 0, b"obsah")
        .map_err(|e| fail("zapis", e))?;
    f.create("/cil", kind::DIR).map_err(|e| fail("mkdir", e))?;
    f.rename("/presun", "/cil/presunuty")
        .map_err(|e| fail("rename", e))?;
    check_eq!(
        f.stat("/presun").err(),
        Some(Error::NotFound),
        "puvodni cesta"
    );
    check_eq!(
        &f.read("/cil/presunuty", 0, 10)
            .map_err(|e| fail("cteni", e))?[..],
        b"obsah",
        "obsah"
    );
    check_eq!(
        f.remove("/cil").err(),
        Some(Error::NotEmpty),
        "mazani neprazdneho adresare"
    );
    f.remove("/cil/presunuty")
        .map_err(|e| fail("remove soubor", e))?;
    f.remove("/cil").map_err(|e| fail("remove adresar", e))?;
    Ok(())
}

fn errors(_: &Startup) -> TestResult {
    let f = fs()?;
    check_eq!(f.stat("/neexistuje").err(), Some(Error::NotFound), "stat");
    check_eq!(
        f.read("/", 0, 1).err(),
        Some(Error::IsADirectory),
        "cteni adresare"
    );
    check_eq!(
        f.list("/vitejte.txt").err(),
        Some(Error::NotADirectory),
        "list souboru"
    );
    check_eq!(
        f.create("relativni", kind::FILE).err(),
        Some(Error::InvalidArgs),
        "relativni cesta"
    );
    check_eq!(
        f.create("/a/../b", kind::FILE).err(),
        // `..` a `.` sluzba odmita (z pripojene cesty relace nejde utect).
        Some(Error::InvalidArgs),
        "cesta s .."
    );
    Ok(())
}

fn snapshots(_: &Startup) -> TestResult {
    let f = fs()?;
    f.create("/verze.txt", kind::FILE)
        .map_err(|e| fail("create", e))?;
    f.write("/verze.txt", 0, b"prvni verze")
        .map_err(|e| fail("zapis", e))?;
    f.snapshot("pred-zmenou").map_err(|e| fail("snapshot", e))?;
    f.truncate("/verze.txt", 0)
        .map_err(|e| fail("truncate", e))?;
    f.write("/verze.txt", 0, b"druha")
        .map_err(|e| fail("zapis 2", e))?;
    let old = f
        .snapshot_read("pred-zmenou", "/verze.txt", 0, 100)
        .map_err(|e| fail("cteni snapshotu", e))?;
    check_eq!(&old[..], b"prvni verze", "obsah ve snapshotu");
    check_eq!(
        &f.read("/verze.txt", 0, 100).map_err(|e| fail("cteni", e))?[..],
        b"druha",
        "zivy obsah"
    );
    let list = f.snapshots().map_err(|e| fail("seznam", e))?;
    check!(
        list.iter().any(|s| s.name == "pred-zmenou"),
        "snapshot chybi v seznamu"
    );
    f.delete_snapshot("PRED-ZMENOU")
        .map_err(|e| fail("smazani", e))?;
    check_eq!(
        f.snapshot_stat("pred-zmenou", "/").err(),
        Some(Error::NotFound),
        "smazany snapshot"
    );
    Ok(())
}

fn fsync_power_loss(_: &Startup) -> TestResult {
    let f = fs()?;
    f.commit().map_err(|e| fail("commit", e))?;
    f.create("/trvaly", kind::FILE)
        .map_err(|e| fail("create", e))?;
    let data = pattern(200_000, 9);
    f.write("/trvaly", 0, &data).map_err(|e| fail("zapis", e))?;
    f.fsync().map_err(|e| fail("fsync", e))?;
    f.create("/neulozeny", kind::FILE)
        .map_err(|e| fail("create 2", e))?;
    let replayed = f
        .simulate_power_loss()
        .map_err(|e| fail("simulace vypadku", e))?;
    check!(replayed >= 2, "z intent logu se nic neprehralo");
    let back = f
        .read("/trvaly", 0, data.len())
        .map_err(|e| fail("cteni po vypadku", e))?;
    check!(back == data, "data po vypadku nesouhlasi");
    check_eq!(
        f.stat("/neulozeny").err(),
        Some(Error::NotFound),
        "zmena bez fsync"
    );
    Ok(())
}

fn contiguous(_: &Startup) -> TestResult {
    let f = fs()?;
    f.create("/video.bin", kind::FILE)
        .map_err(|e| fail("create", e))?;
    let chunk = pattern(64 * 1024, 3);
    for i in 0..64u64 {
        f.write("/video.bin", i * chunk.len() as u64, &chunk)
            .map_err(|e| fail("zapis", e))?;
    }
    f.commit().map_err(|e| fail("commit", e))?;
    check_eq!(
        f.fragments("/video.bin")
            .map_err(|e| fail("fragmenty", e))?,
        1,
        "pocet useku"
    );
    Ok(())
}

fn check_clean(_: &Startup) -> TestResult {
    let problems = fs()?.check(true).map_err(|e| fail("kontrola", e))?;
    check!(
        problems.is_empty(),
        alloc::format!("problemy: {problems:?}").as_str()
    );
    Ok(())
}

/// Adresar, jehoz vypis se do jedne zpravy kanalu nevejde (2000 jmen po
/// ~100 znacich = ~220 KiB): klient ho nacte po strankach a sluzba nespadne.
fn big_directory(_: &Startup) -> TestResult {
    let f = fs()?;
    f.create("/velky-adresar", kind::DIR).map_err(|e| fail("create dir", e))?;
    let long = "x".repeat(90);
    for i in 0..2000 {
        let p = alloc::format!("/velky-adresar/{long}-{i:05}");
        f.create(&p, kind::FILE).map_err(|e| fail("create", e))?;
    }
    let list = f.list("/velky-adresar").map_err(|e| fail("list", e))?;
    check_eq!(list.len(), 2000, "pocet polozek");
    check!(
        list.windows(2).all(|w| w[0].name < w[1].name),
        "polozky serazene a bez opakovani"
    );
    // Sluzba po velkem vypisu dal odpovida.
    f.info().map_err(|e| fail("info po vypisu", e))?;
    Ok(())
}

/// A otevre report, B ho v jine relaci atomicky nahradi; A cte dal puvodni
/// obsah, nove otevreni dostane novy.
fn v2_two_sessions(_: &Startup) -> TestResult {
    let f = fs()?;
    check_eq!(f.hello().map_err(|e| fail("hello", e))?, fsproto::v2::VERSION, "verze protokolu");
    let a = f.connect().map_err(|e| fail("connect A", e))?;
    let b = f.connect().map_err(|e| fail("connect B", e))?;
    a.save_if("/v2-report.txt", None, b"puvodni").map_err(|e| fail("prvni ulozeni", e))?;
    let ha = a.open("/v2-report.txt", access::READ, share::ALL, 0).map_err(|e| fail("open A", e))?;
    let rev = ha.revision;
    b.save_if("/v2-report.txt", Some(rev), b"nova verze").map_err(|e| fail("nahrazeni B", e))?;
    let old = a.read_h(ha.handle, 0, 100).map_err(|e| fail("cteni A", e))?;
    check_eq!(&old[..], b"puvodni", "A cte puvodni objekt");
    let hb = b.open("/v2-report.txt", access::READ, share::ALL, 0).map_err(|e| fail("open B", e))?;
    let new = b.read_h(hb.handle, 0, 100).map_err(|e| fail("cteni B", e))?;
    check_eq!(&new[..], b"nova verze", "nove otevreni = novy objekt");
    // Handle z cizi relace neplati.
    check_eq!(a.read_h(hb.handle, 0, 1).err(), Some(Error::BadHandle), "cizi handle");
    a.close(ha.handle).map_err(|e| fail("close A", e))?;
    b.close(hb.handle).map_err(|e| fail("close B", e))?;
    Ok(())
}

/// Podminene ulozeni zaklada docasny soubor `.~jmeno.N.tmp` vyhradne:
/// existujici soubor se stejnym jmenem (cizi, zbytek po padu) zustane
/// netknuty a fsd vezme dalsi cislo.
fn v2_temp_collision(_: &Startup) -> TestResult {
    let f = fs()?;
    let ed = f.connect().map_err(|e| fail("connect", e))?;
    f.create("/v2-kolize", kind::DIR).map_err(|e| fail("slozka", e))?;
    // Cislo pristiho docasneho souboru: zacit nahrazeni a podivat se.
    let h = ed.replace_begin("/v2-kolize/x.txt", None).map_err(|e| fail("replace_begin", e))?;
    let list = f.list("/v2-kolize").map_err(|e| fail("vypis", e))?;
    let n: u64 = list
        .iter()
        .find_map(|e| e.name.strip_prefix(".~x.txt.").and_then(|r| r.strip_suffix(".tmp")).and_then(|d| d.parse().ok()))
        .ok_or("docasny soubor nahrazeni neni ve vypisu")?;
    ed.replace_abort(h).map_err(|e| fail("replace_abort", e))?;
    // Cizi soubory presne na dalsich cislech.
    for k in n + 1..=n + 4 {
        let p = alloc::format!("/v2-kolize/.~x.txt.{k}.tmp");
        f.create(&p, kind::FILE).map_err(|e| fail("cizi soubor", e))?;
        f.write(&p, 0, alloc::format!("cizi {k}").as_bytes()).map_err(|e| fail("zapis ciziho", e))?;
    }
    ed.save_if("/v2-kolize/x.txt", None, b"novy").map_err(|e| fail("ulozeni pres kolizi", e))?;
    check_eq!(&f.read("/v2-kolize/x.txt", 0, 10).map_err(|e| fail("cteni", e))?[..], b"novy", "ulozeno");
    for k in n + 1..=n + 4 {
        let p = alloc::format!("/v2-kolize/.~x.txt.{k}.tmp");
        let data = f.read(&p, 0, 20).map_err(|e| fail("cteni ciziho", e))?;
        check_eq!(data, alloc::format!("cizi {k}").into_bytes(), "cizi soubor netknuty");
        f.remove(&p).map_err(|e| fail("uklid", e))?;
    }
    f.remove("/v2-kolize/x.txt").map_err(|e| fail("uklid", e))?;
    f.remove("/v2-kolize").map_err(|e| fail("uklid", e))?;
    Ok(())
}

/// Editor si pamatuje revizi; mezitim soubor zmeni nekdo jiny - ulozeni
/// editoru vrati CONFLICT a nic se nezmeni.
fn v2_conditional_save(_: &Startup) -> TestResult {
    let f = fs()?;
    let ed = f.connect().map_err(|e| fail("connect", e))?;
    ed.save_if("/v2-config.toml", None, b"a = 1").map_err(|e| fail("vytvoreni", e))?;
    let h = ed.open("/v2-config.toml", access::READ, share::ALL, 0).map_err(|e| fail("open", e))?;
    ed.close(h.handle).map_err(|e| fail("close", e))?;
    // Agent zapise pres cestu (v1).
    f.write("/v2-config.toml", 0, b"a = 2").map_err(|e| fail("zapis agenta", e))?;
    check_eq!(ed.save_if("/v2-config.toml", Some(h.revision), b"a = 3").err(), Some(Error::Conflict), "konflikt");
    let data = f.read("/v2-config.toml", 0, 10).map_err(|e| fail("cteni", e))?;
    check_eq!(&data[..], b"a = 2", "obsah po konfliktu");
    // Zadny docasny soubor nezustal.
    let list = f.list("/").map_err(|e| fail("vypis", e))?;
    check!(!list.iter().any(|e| e.name.starts_with(".~")), "docasny soubor zustal");
    // Zapis po kusech: pocet prijatych bajtu.
    let w = ed.open("/v2-velky", access::READ | access::WRITE, share::ALL, open_flags::CREATE).map_err(|e| fail("open velky", e))?;
    let data = pattern(100 * 1024, 9);
    check_eq!(ed.write_h(w.handle, 0, &data).map_err(|(_, e)| fail("write_h", e))?, data.len(), "prijato vse");
    let back = ed.read_h(w.handle, 0, data.len()).map_err(|e| fail("read_h", e))?;
    check!(back == data, "obsah po zapisu po kusech");
    ed.close(w.handle).map_err(|e| fail("close velky", e))?;
    Ok(())
}

/// Konec relace zavre jeji handly (sirotek zmizi); zamek jedne relace
/// brani druhe.
fn v2_session_cleanup_and_locks(_: &Startup) -> TestResult {
    let f = fs()?;
    {
        let c = f.connect().map_err(|e| fail("connect C", e))?;
        let h = c
            .open("/v2-sirotek", access::READ | access::WRITE, share::ALL, open_flags::CREATE)
            .map_err(|e| fail("open", e))?;
        c.write_h(h.handle, 0, &[1u8; 50_000]).map_err(|(_, e)| fail("zapis", e))?;
        f.remove("/v2-sirotek").map_err(|e| fail("smazani otevreneho", e))?;
        let still = c.read_h(h.handle, 0, 4).map_err(|e| fail("cteni sirotka", e))?;
        check_eq!(&still[..], &[1u8; 4][..], "sirotek citelny");
        // Konec bloku: relace C zavre kanal.
    }
    let a = f.connect().map_err(|e| fail("connect A", e))?;
    let b = f.connect().map_err(|e| fail("connect B", e))?;
    let rw = access::READ | access::WRITE;
    let ha = a.open("/v2-db", rw, share::ALL, open_flags::CREATE).map_err(|e| fail("open A", e))?;
    let hb = b.open("/v2-db", rw, share::ALL, 0).map_err(|e| fail("open B", e))?;
    a.lock(ha.handle, 0, 100, lock::EXCLUSIVE | lock::MANDATORY).map_err(|e| fail("lock", e))?;
    check_eq!(b.write_h(hb.handle, 10, b"x").err().map(|(_, e)| e), Some(Error::Locked), "povinny zamek");
    a.unlock(ha.handle, 0, 100).map_err(|e| fail("unlock", e))?;
    b.write_h(hb.handle, 10, b"x").map_err(|(_, e)| fail("zapis po odemceni", e))?;
    drop(a);
    drop(b);
    // Kontrola: sirotek po konci relace zmizel, svazek je cisty.
    let problems = f.check(false).map_err(|e| fail("kontrola", e))?;
    check!(problems.is_empty(), alloc::format!("problemy: {problems:?}").as_str());
    Ok(())
}

const ALICE: u64 = 1000;
const BOB: u64 = 1001;

/// Domov /perm/alice patri Alici; / a /perm jde projit.
/// Pocet relaci uz neomezuje limit `OBJECT_WAIT_MANY` (64): fsd ceka pres
/// port. 200 relaci, kazda s vlastnim handlem; proud za nimi; po zavreni
/// pulky ostatni dal funguji.
fn v2_many_sessions(_: &Startup) -> TestResult {
    const N: usize = 200;
    let f = fs()?;
    f.save_if("/v2-mnoho.txt", None, b"mnoho relaci").map_err(|e| fail("ulozeni", e))?;
    let mut sessions = Vec::new();
    for i in 0..N {
        let c = f.connect().map_err(|e| fail(&alloc::format!("connect {i}"), e))?;
        let h = c.open("/v2-mnoho.txt", access::READ, share::ALL, 0).map_err(|e| fail(&alloc::format!("open {i}"), e))?;
        sessions.push((c, h.handle));
    }
    // Vsechny relace odpovidaji (i ty nad starym limitem), v obracenem poradi.
    for (i, (c, h)) in sessions.iter().enumerate().rev() {
        let d = c.read_h(*h, 0, 5).map_err(|e| fail(&alloc::format!("cteni {i}"), e))?;
        check_eq!(&d[..], b"mnoho", "obsah v relaci");
    }
    // Proud se vejde i pri 200 relacich (drive sdileny limit relaci a proudu).
    let (last, lh) = &sessions[N - 1];
    let chan = last.open_stream(*lh, fsproto::v2::stream_mode::READ, 0).map_err(|e| fail("proud", e))?;
    let mut got = Vec::new();
    loop {
        match libos::wait_one(chan.handle(), libos::Signals::READABLE | libos::Signals::PEER_CLOSED, None) {
            Ok(_) => {}
            Err(e) => return Err(fail("cekani proudu", e)),
        }
        match chan.try_read() {
            Ok(m) => got.extend_from_slice(&m.data),
            Err(Error::ShouldWait) => continue,
            Err(_) => break,
        }
    }
    check_eq!(&got[..], b"mnoho relaci", "obsah proudu");
    // Zavrit kazdou druhou relaci (bez CLOSE: konec kanalu); ostatni dal funguji.
    let mut keep = Vec::new();
    for (i, s) in sessions.into_iter().enumerate() {
        if i % 2 == 1 {
            keep.push(s);
        }
    }
    for (c, h) in &keep {
        let d = c.read_h(*h, 6, 6).map_err(|e| fail("cteni po zavreni pulky", e))?;
        check_eq!(&d[..], b"relaci", "obsah po zavreni pulky");
    }
    for (c, h) in &keep {
        c.close(*h).map_err(|e| fail("close", e))?;
    }
    drop(keep);
    f.remove("/v2-mnoho.txt").map_err(|e| fail("smazani", e))?;
    Ok(())
}

fn perm_setup(f: &FsClient) -> TestResult {
    let traverse = Acl { flags: 0, grants: alloc::vec![Grant { who: EVERYONE, rights: rights::TRAVERSE, flags: grant::THIS }] };
    f.set_acl("/", Some(&traverse)).map_err(|e| fail("acl /", e))?;
    let _ = f.create("/perm", kind::DIR);
    f.set_acl("/perm", Some(&traverse)).map_err(|e| fail("acl /perm", e))?;
    let _ = f.create("/perm/alice", kind::DIR);
    f.set_owner("/perm/alice", ALICE).map_err(|e| fail("vlastnik", e))?;
    Ok(())
}

fn perm_sessions(_: &Startup) -> TestResult {
    let f = fs()?;
    perm_setup(f)?;
    // Relace uzivatele: cely FS, prava podle opravneni.
    let (_, alice) = f.connect_as(ALICE, &[], &Profile::Standard.user_mounts()).map_err(|e| fail("connect_as", e))?;
    let _ = alice.create("/perm/alice/apps", kind::DIR);
    let _ = alice.create("/perm/alice/apps/editor", kind::DIR);
    alice.write("/perm/alice/foto.jpg", 0, b"JPEG").or_else(|_| {
        alice.create("/perm/alice/foto.jpg", kind::FILE)?;
        alice.write("/perm/alice/foto.jpg", 0, b"JPEG")
    }).map_err(|e| fail("foto", e))?;
    let st = alice.stat("/perm/alice/foto.jpg").map_err(|e| fail("stat foto", e))?;
    check_eq!(st.owner, ALICE, "vlastnik noveho souboru");
    check_eq!(st.rights, rights::ALL, "vlastnik ma vse");
    // Cizi soubory (z jinych testu) jsou neviditelne.
    check_eq!(alice.stat("/vitejte.txt").err(), Some(Error::NotFound), "systemovy soubor");
    // Klient si identitu vymyslet nemuze: zakladat relace smi jen ridici.
    check_eq!(alice.connect_as(BOB, &[], &[]).err(), Some(Error::AccessDenied), "CONNECT_AS z relace uzivatele");
    // Relace aplikace (profil Standardni): jen svuj adresar a vybrany soubor.
    let picked = [Mount::new("/vybrane/foto.jpg", "/perm/alice/foto.jpg", mount_flags::READ_ONLY)];
    let mounts = Profile::Standard.app_mounts("/perm/alice", "editor", &picked);
    let (id, app) = f.connect_as(ALICE, &[], &mounts).map_err(|e| fail("relace aplikace", e))?;
    let _ = app.create("/app/poznamky.txt", kind::FILE);
    app.write("/app/poznamky.txt", 0, b"ahoj").map_err(|e| fail("zapis do /app", e))?;
    check_eq!(
        alice.read("/perm/alice/apps/editor/poznamky.txt", 0, 10).map_err(|e| fail("cteni uzivatelem", e))?,
        b"ahoj".to_vec(),
        "/app je adresar aplikace"
    );
    check_eq!(app.read("/vybrane/foto.jpg", 0, 4).map_err(|e| fail("vybrany soubor", e))?, b"JPEG".to_vec(), "vybrany soubor");
    check_eq!(app.write("/vybrane/foto.jpg", 0, b"x").err(), Some(Error::AccessDenied), "vyber jen pro cteni");
    check_eq!(app.stat("/perm/alice").err(), Some(Error::NotFound), "mimo pripojene cesty");
    check_eq!(app.stat("/app/../foto.jpg").err(), Some(Error::InvalidArgs), "cesta s ..");
    // Powerbox: uzivatel vybere slozku pozdeji.
    f.grant(id, &Mount::new("/dokumenty", "/perm/alice", 0)).map_err(|e| fail("grant", e))?;
    app.stat("/dokumenty/foto.jpg").map_err(|e| fail("po grant", e))?;
    f.revoke(id, "/dokumenty").map_err(|e| fail("revoke", e))?;
    check_eq!(app.stat("/dokumenty/foto.jpg").err(), Some(Error::NotFound), "po revoke");
    check_eq!(f.grant(999_999, &Mount::new("/x", "/", 0)).err(), Some(Error::NotFound), "grant neexistujici relaci");
    Ok(())
}

fn perm_noexec_and_sharing(_: &Startup) -> TestResult {
    let f = fs()?;
    perm_setup(f)?;
    let (_, alice) = f.connect_as(ALICE, &[], &Profile::Standard.user_mounts()).map_err(|e| fail("alice", e))?;
    let (_, bob) = f.connect_as(BOB, &[], &Profile::Standard.user_mounts()).map_err(|e| fail("bob", e))?;
    let _ = alice.create("/perm/alice/sdilene", kind::DIR);
    let _ = alice.create("/perm/alice/sdilene/plan.txt", kind::FILE);
    check_eq!(bob.stat("/perm/alice/sdilene").err(), Some(Error::NotFound), "pred sdilenim");
    let t = Acl { flags: 0, grants: alloc::vec![Grant { who: BOB, rights: rights::TRAVERSE, flags: grant::THIS }] };
    alice.set_acl("/perm/alice", Some(&t)).map_err(|e| fail("acl domova", e))?;
    let share_acl = Acl { flags: 0, grants: alloc::vec![Grant { who: BOB, rights: rights::EDIT, flags: grant::BOTH }] };
    alice.set_acl("/perm/alice/sdilene", Some(&share_acl)).map_err(|e| fail("sdileni", e))?;
    bob.write("/perm/alice/sdilene/plan.txt", 0, b"od Boba").map_err(|e| fail("zapis Boba", e))?;
    let p = bob.permissions("/perm/alice/sdilene").map_err(|e| fail("permissions", e))?;
    check_eq!(p.owner, ALICE, "vlastnik sdilene");
    check_eq!(p.rights, rights::EDIT, "prava Boba");
    check_eq!(bob.set_acl("/perm/alice/sdilene", None).err(), Some(Error::AccessDenied), "Bob nesmi menit prava");
    check_eq!(bob.set_owner("/perm/alice/sdilene", BOB).err(), Some(Error::AccessDenied), "vlastnika meni jen system");
    // Zakaz spousteni (na korenovem datasetu jen docasne).
    let h = f.open("/vitejte.txt", access::EXECUTE, share::ALL, 0).map_err(|e| fail("spusteni", e))?;
    f.close(h.handle).map_err(|e| fail("close", e))?;
    f.set_noexec("/", true).map_err(|e| fail("noexec", e))?;
    check!(f.noexec("/").map_err(|e| fail("noexec?", e))?, "zakaz nastaveny");
    let denied = f.open("/vitejte.txt", access::EXECUTE, share::ALL, 0).err();
    f.set_noexec("/", false).map_err(|e| fail("noexec vypnout", e))?;
    check_eq!(denied, Some(Error::AccessDenied), "spusteni se zakazem");
    Ok(())
}
