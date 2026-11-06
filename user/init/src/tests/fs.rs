//! Souborovy system pres sluzbu fsd (datovy disk pripravi xtask).

use alloc::vec::Vec;

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
