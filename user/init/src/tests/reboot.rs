//! Opakovany start na stejnem datovem disku (`cargo xtask test --only
//! opakovany-start`, xtask/src/boottest.rs): xtask nastartuje system
//! nekolikrat po sobe nad jednim diskem a tahle sada pri kazdem startu
//! overi, ze jde zapisovat a commitovat i na disk z minuleho startu.
//!
//! Chyby typu "prvni start vs. dalsi" bezne testy nechytnou: kazdy beh ma
//! novy datovy disk. Priklad: sluzba uctu zaklada svuj dataset pri kazdem
//! startu; od druheho startu prisla chyba `ALREADY_EXISTS` az po zmene stavu
//! a svazek presel do stavu chyby (kazdy zapis `IO_ERROR`).
//!
//! Na disku: `/opakovany-start.txt` = pocet startu (xtask ho po poslednim
//! startu precte na hostiteli), dataset `/opakovany-ds` se souborem za kazdy
//! start.

use alloc::format;

use fsproto::{FsClient, kind};
use libos::{Error, Startup};

use super::{Test, TestResult, devices, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[("zapis a commit na disku z minuleho startu", write_after_reboot)];

const COUNTER: &str = "/opakovany-start.txt";
const DATASET: &str = "/opakovany-ds";

fn fs() -> Result<&'static FsClient, alloc::string::String> {
    devices().and_then(|d| d.fs.as_ref()).ok_or_else(|| "sluzba fsd nebezi".into())
}

/// Pocet startu ulozeny na disku (0 = novy disk).
fn previous(f: &FsClient) -> Result<u64, alloc::string::String> {
    match f.read(COUNTER, 0, 32) {
        Ok(b) => core::str::from_utf8(&b)
            .ok()
            .and_then(|t| t.trim().parse().ok())
            .ok_or_else(|| "pocitadlo startu je poskozene".into()),
        Err(Error::NotFound) => Ok(0),
        Err(e) => Err(fail("cteni pocitadla", e)),
    }
}

fn write_after_reboot(_: &Startup) -> TestResult {
    let f = fs()?;
    let n = previous(f)?;
    let this = n + 1;
    libos::println!("init: opakovany start: start c. {this} na tomto disku");
    // Zalozeni datasetu jako sluzba uctu: od druheho startu uz existuje.
    let r = f.create_dataset(DATASET);
    if n == 0 {
        r.map_err(|e| fail("prvni zalozeni datasetu", e))?;
    } else {
        check_eq!(r.err(), Some(Error::AlreadyExists), "dataset z minuleho startu");
        let prev = format!("{DATASET}/start-{n}.txt");
        let data = f.read(&prev, 0, 64).map_err(|e| fail("soubor z minuleho startu", e))?;
        check!(data == format!("start {n}").into_bytes(), "obsah souboru z minuleho startu");
    }
    // Zapis a commit po kolizi musi projit (svazek nesmi byt ve stavu chyby).
    let path = format!("{DATASET}/start-{this}.txt");
    let _ = f.create(&path, kind::FILE);
    f.write(&path, 0, format!("start {this}").as_bytes()).map_err(|e| fail("zapis", e))?;
    let _ = f.create(COUNTER, kind::FILE);
    f.truncate(COUNTER, 0).map_err(|e| fail("zkraceni pocitadla", e))?;
    f.write(COUNTER, 0, format!("{this}\n").as_bytes()).map_err(|e| fail("zapis pocitadla", e))?;
    f.commit().map_err(|e| fail("commit", e))?;
    check_eq!(previous(f)?, this, "pocitadlo po zapisu");
    Ok(())
}
