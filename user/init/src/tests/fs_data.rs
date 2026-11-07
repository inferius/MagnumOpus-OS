//! Data FS pres sluzbu fsd (faze 3): okamzite kopie, predalokace,
//! nastaveni dat (komprese, kopie) a scrub.

use alloc::vec::Vec;

use fsproto::data::{DataSettings, SCRUB_RUNNING, compress, dedup};
use fsproto::{FsClient, kind};
use libos::Startup;

use super::{Test, TestResult, devices, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("okamzita kopie souboru a rozsahu", clone),
    ("komprese a druha kopie dat v datasetu", settings),
    ("predalokace: nuly a zapis", fallocate),
    ("scrub na povel", scrub),
    ("kontrola konzistence po datech", check_clean),
];

fn fs() -> Result<&'static FsClient, alloc::string::String> {
    devices()
        .and_then(|d| d.fs.as_ref())
        .ok_or_else(|| "sluzba fsd nebezi".into())
}

/// Nekomprimovatelna data (xorshift).
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

fn clone(_: &Startup) -> TestResult {
    let f = fs()?;
    let data = noise(200_000, 3);
    f.create("/f3-a.bin", kind::FILE).map_err(|e| fail("create", e))?;
    f.write("/f3-a.bin", 0, &data).map_err(|e| fail("zapis", e))?;
    let n = f.clone_path("/f3-a.bin", "/f3-b.bin").map_err(|e| fail("clone", e))?;
    check_eq!(n, data.len() as u64, "sdilene bajty");
    check_eq!(f.read("/f3-b.bin", 0, data.len()).map_err(|e| fail("cteni kopie", e))?, data, "obsah kopie");
    // Zmena kopie original nemeni.
    f.write("/f3-b.bin", 0, b"jine").map_err(|e| fail("zapis do kopie", e))?;
    check_eq!(f.read("/f3-a.bin", 0, 4).map_err(|e| fail("cteni", e))?, data[..4].to_vec(), "original");
    f.clone_range("/f3-a.bin", 8192, "/f3-b.bin", 0, 4096).map_err(|e| fail("clone_range", e))?;
    check_eq!(f.read("/f3-b.bin", 0, 4096).map_err(|e| fail("cteni", e))?, data[8192..12288].to_vec(), "rozsah");
    check!(f.clone_range("/f3-a.bin", 1, "/f3-b.bin", 0, 4096).is_err(), "nezarovnany posun odmitnut");
    f.remove("/f3-a.bin").map_err(|e| fail("remove", e))?;
    f.remove("/f3-b.bin").map_err(|e| fail("remove", e))?;
    Ok(())
}

fn settings(_: &Startup) -> TestResult {
    let f = fs()?;
    let _ = f.delete_dataset("/f3-ds");
    f.create_dataset("/f3-ds").map_err(|e| fail("dataset", e))?;
    let s = f.data_settings("/f3-ds").map_err(|e| fail("nastaveni", e))?;
    check_eq!((s.compress, s.copies, s.dedup), (compress::NONE, 1, dedup::BACKGROUND), "vychozi nastaveni");
    let want = DataSettings { compress: compress::LZ4, copies: 2, dedup: dedup::OFF };
    f.set_data_settings("/f3-ds", &want).map_err(|e| fail("zmena nastaveni", e))?;
    check_eq!(f.data_settings("/f3-ds").map_err(|e| fail("nastaveni", e))?, want, "ulozene nastaveni");
    let bad = DataSettings { compress: 9, copies: 1, dedup: dedup::OFF };
    check!(f.set_data_settings("/f3-ds", &bad).is_err(), "neznama komprese odmitnuta");
    let text: Vec<u8> = b"komprimovatelny radek textu; ".iter().copied().cycle().take(300_000).collect();
    f.create("/f3-ds/t.txt", kind::FILE).map_err(|e| fail("create", e))?;
    f.write("/f3-ds/t.txt", 0, &text).map_err(|e| fail("zapis", e))?;
    f.commit().map_err(|e| fail("commit", e))?;
    check_eq!(f.read("/f3-ds/t.txt", 0, text.len()).map_err(|e| fail("cteni", e))?, text, "obsah");
    f.delete_dataset("/f3-ds").map_err(|e| fail("smazani datasetu", e))?;
    Ok(())
}

fn fallocate(_: &Startup) -> TestResult {
    let f = fs()?;
    f.create("/f3-db", kind::FILE).map_err(|e| fail("create", e))?;
    f.fallocate("/f3-db", 0, 1 << 20, false).map_err(|e| fail("fallocate", e))?;
    check_eq!(f.stat("/f3-db").map_err(|e| fail("stat", e))?.size, 1 << 20, "velikost");
    let z = f.read("/f3-db", 500_000, 4096).map_err(|e| fail("cteni", e))?;
    check!(z.iter().all(|&b| b == 0), "predalokovane bloky ctou nuly");
    f.write("/f3-db", 65536, b"zaznam").map_err(|e| fail("zapis", e))?;
    f.fsync().map_err(|e| fail("fsync", e))?;
    check_eq!(f.read("/f3-db", 65536, 6).map_err(|e| fail("cteni", e))?, b"zaznam".to_vec(), "zapis");
    f.remove("/f3-db").map_err(|e| fail("remove", e))?;
    Ok(())
}

fn scrub(_: &Startup) -> TestResult {
    let f = fs()?;
    f.commit().map_err(|e| fail("commit", e))?;
    f.scrub_start().map_err(|e| fail("scrub_start", e))?;
    // Scrub bezi na pozadi mezi pozadavky; cekat, az dobehne.
    let mut st = f.scrub_status().map_err(|e| fail("scrub_status", e))?;
    for _ in 0..600 {
        if st.state != SCRUB_RUNNING {
            break;
        }
        libos::time::sleep(core::time::Duration::from_millis(50));
        st = f.scrub_status().map_err(|e| fail("scrub_status", e))?;
    }
    check!(st.state != SCRUB_RUNNING, "scrub nedobehl");
    check!(st.scanned > 0, "scrub nic neprecetl");
    check_eq!((st.bad, st.lost), (0, 0), "chyby na cistem svazku");
    Ok(())
}

fn check_clean(_: &Startup) -> TestResult {
    let problems = fs()?.check(true).map_err(|e| fail("kontrola", e))?;
    check!(problems.is_empty(), alloc::format!("problemy: {problems:?}").as_str());
    Ok(())
}
