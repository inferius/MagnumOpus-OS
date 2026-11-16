//! Vymenna media (docs/cs/media.md): flash disky USB s FAT32 a NTFS
//! (xtask `mediadisks.rs`, `removable=on`) se pri startu pripoji do
//! `/media/<jmenovka>`, uzivatelska relace je cte a na FAT zapisuje, NTFS je
//! jen pro cteni, bezpecne odpojeni media zmizi. Pak flash disk zasunuty
//! a vytazeny za behu (QMP). Co test zapsal na FAT, overi xtask po behu na
//! hostiteli (`mediadisks::verify_fat`, znacka [`GUEST_MARK`]).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use fsproto::FsClient;
use fsproto::perm::Profile;
use fsproto::v2::MediaEntry;
use libos::{Error, Startup, println, time};

use super::{Test, TestResult, devices, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[("flash disky FAT32 a NTFS, zapis, odpojeni, zasunuti za behu", media)];

/// Musi sedet s `xtask/src/mediadisks.rs`.
const FAT: &str = "MOTEST";
const NTFS: &str = "mylabel";
const HOT: &str = "HOTPLUG";
const FAT_FILE: &str = "Přílišžluťoučký kůň.txt";
const FAT_TEXT: &str = "ahoj z hostitele\n";
const GUEST_DIR: &str = "Zapsáno v systému";
const GUEST_FILE: &str = "z-testu.txt";
const GUEST_TEXT: &str = "zapis z MagnumOpus\n";
const GUEST_MARK: &str = "media: zapsano a odpojeno";
/// Znacky pro QMP (`xtask/src/qmp.rs`, modul `media`).
const PLUG: &str = "init: test media: zasun flashku";
const UNPLUG: &str = "init: test media: vytahni flashku";

/// Testovaci uzivatel (relace se standardnim profilem = s `/media`).
const USER: u64 = 1000;
const WAIT: Duration = Duration::from_secs(30);

/// Pocka, az `want(seznam)` plati. Zpravy devmgr (medium pripojene za
/// behu) zpracuje jako supervisor, ktery v testovacim behu nebezi.
fn wait_list(fs: &FsClient, what: &str, want: impl Fn(&[MediaEntry]) -> bool) -> Result<Vec<MediaEntry>, String> {
    let until = time::deadline_after(WAIT);
    loop {
        if let Some(d) = devices() {
            while let Ok(m) = d.control.try_read() {
                crate::system::devmgr_message(d, m);
            }
        }
        let list = fs.media_list().map_err(|e| fail("seznam medii", e))?;
        if want(&list) {
            return Ok(list);
        }
        if time::now() > until {
            let names: Vec<&str> = list.iter().map(|m| m.name.as_str()).collect();
            return Err(format!("{what}: media {names:?} do {} s", WAIT.as_secs()));
        }
        time::sleep(Duration::from_millis(100));
    }
}

fn media(_: &Startup) -> TestResult {
    let sys = devices().and_then(|d| d.fs.as_ref()).ok_or("sluzba fsd nebezi")?;
    let list = wait_list(sys, "media pri startu", |l| l.iter().any(|m| m.name == FAT) && l.iter().any(|m| m.name == NTFS))?;
    let fat = list.iter().find(|m| m.name == FAT).ok_or("FAT")?;
    let ntfs = list.iter().find(|m| m.name == NTFS).ok_or("NTFS")?;
    check_eq!((fat.fs.as_str(), fat.read_only), ("fat", false), "FAT na vymennem USB zapisovatelny");
    check!(fat.total_bytes > 50 << 20 && fat.free_bytes > 0, "velikost FAT");
    check_eq!((ntfs.fs.as_str(), ntfs.read_only), ("ntfs", true), "NTFS jen pro cteni");

    // Uzivatelska relace (Soubory, aplikace) vidi /media.
    let (_, user) = sys.connect_as(USER, &[], &Profile::Standard.user_mounts()).map_err(|e| fail("relace uzivatele", e))?;
    let names: Vec<String> = user.list("/media").map_err(|e| fail("vypis /media", e))?.into_iter().map(|e| e.name).collect();
    check!(names.iter().any(|n| n == FAT) && names.iter().any(|n| n == NTFS), format!("/media: {names:?}").as_str());
    let root = format!("/media/{FAT}");
    let host = user.read(&format!("{root}/{FAT_FILE}"), 0, 4096).map_err(|e| fail("cteni souboru z hostitele", e))?;
    check_eq!(host, FAT_TEXT.as_bytes().to_vec(), "dlouhe ceske jmeno a obsah");
    check_eq!(user.stat(&format!("{root}/{FAT_FILE}")).map(|s| s.owner), Ok(USER), "vlastnik = uzivatel relace");

    // Zapis: slozka, docasny soubor, prejmenovani (jako ulozeni editoru).
    user.create(&format!("{root}/{GUEST_DIR}"), fsproto::kind::DIR).map_err(|e| fail("slozka na FAT", e))?;
    let tmp = format!("{root}/{GUEST_DIR}/.z-testu.tmp");
    user.create(&tmp, fsproto::kind::FILE).map_err(|e| fail("soubor na FAT", e))?;
    user.write(&tmp, 0, GUEST_TEXT.as_bytes()).map_err(|e| fail("zapis na FAT", e))?;
    let dst = format!("{root}/{GUEST_DIR}/{GUEST_FILE}");
    user.rename(&tmp, &dst).map_err(|e| fail("prejmenovani na FAT", e))?;
    check_eq!(user.read(&dst, 0, 100).map_err(|e| fail("cteni zpet", e))?, GUEST_TEXT.as_bytes().to_vec(), "obsah zpet");
    check_eq!(user.stat(&tmp).err(), Some(Error::NotFound), "docasny soubor prejmenovan");
    // Prejmenovani mimo medium = kopie (Soubory), ne prejmenovani.
    check_eq!(user.rename(&dst, "/home/x.txt").err(), Some(Error::NotSupported), "rename mezi mediem a domovem");

    // NTFS: cteni, zapis odmitnut.
    let nroot = format!("/media/{NTFS}");
    check_eq!(user.read(&format!("{nroot}/file-with-12345"), 0, 100).map_err(|e| fail("cteni NTFS", e))?, b"12345".to_vec(), "NTFS cteni");
    check_eq!(user.list(&format!("{nroot}/many_subdirs")).map(|l| l.len()), Ok(512), "NTFS slozka s indexem");
    check_eq!(user.write(&format!("{nroot}/file-with-12345"), 0, b"x").err(), Some(Error::ReadOnly), "NTFS zapis odmitnut");

    // Bezpecne odpojeni (Odpojit v Souborech = uzivatelska relace).
    user.media_remove(FAT).map_err(|e| fail("odpojeni FAT", e))?;
    check!(!sys.media_list().map_err(|e| fail("seznam po odpojeni", e))?.iter().any(|m| m.name == FAT), "FAT po odpojeni zmizel");
    check_eq!(user.stat(&dst).err(), Some(Error::NotFound), "cesta odpojeneho media");
    println!("init: {GUEST_MARK}");

    // Zasunuti a vytazeni za behu (QMP device_add/device_del).
    println!("{PLUG}");
    wait_list(sys, "flash disk zasunuty za behu", |l| l.iter().any(|m| m.name == HOT))?;
    check_eq!(user.read(&format!("/media/{HOT}/hot.txt"), 0, 100).map_err(|e| fail("cteni media za behu", e))?, b"za behu\n".to_vec(), "obsah media za behu");
    println!("{UNPLUG}");
    wait_list(sys, "vytazeny flash disk", |l| !l.iter().any(|m| m.name == HOT))?;
    check_eq!(user.stat(&format!("/media/{HOT}/hot.txt")).err(), Some(Error::NotFound), "vytazene medium");
    Ok(())
}
