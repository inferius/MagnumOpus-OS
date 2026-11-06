//! Souborovy system pres sluzbu fsd (datovy disk pripravi xtask).

use alloc::vec::Vec;

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
    ("v2: konec relace uklidi handly, zamky mezi relacemi", v2_session_cleanup_and_locks),
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
        Some(Error::NotFound),
        "neexistujici rodic"
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
    check_eq!(f.hello().map_err(|e| fail("hello", e))?, 2, "verze protokolu");
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
