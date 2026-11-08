//! Testy diagnostiky: trvaly log (logd), balicek pro vyvojare, prava
//! syscallu `LOG_READ` a `CRASH_RECORD`. Zvlastni beh "zaznam padu prezije
//! restart" je [`scenario`] (initfs `crash-test`, `cargo xtask test`).

use alloc::string::String;
use alloc::vec;
use core::time::Duration;

use libos::{Error, Rights, Startup, println, time};

use super::{TestResult, fail};
use crate::check;

pub const TESTS: &[(&str, fn(&Startup) -> TestResult)] = &[
    ("log jadra jde cist jen s pravem DIAG", log_read_right),
    ("logd zapisuje log bootu na disk", log_file),
    ("report sestavi balicek s nahledem", report),
];

/// Znacky zvlastniho behu (musi sedet s `xtask/src/crashtest.rs`).
pub const MARKER_PANIC: &str = "init: test padu: vyvolavam paniku jadra";
pub const MARKER_OK: &str = "init: test padu: zaznam prezil restart";

fn log_read_right(s: &Startup) -> TestResult {
    let root = s.root_resource.as_ref().ok_or("chybi korenovy Resource")?;
    let mut buf = vec![0u8; 4096];
    let (_start, n, _lost) = root.log_read(0, &mut buf).map_err(|e| fail("LOG_READ", e))?;
    check!(n > 0, "log jadra je prazdny");
    // Prostredek bez DIAG: ACCESS_DENIED.
    let weak = root
        .handle()
        .duplicate(Some(Rights::BASIC | Rights::MANAGE))
        .map(libos::hw::Resource::from_handle)
        .map_err(|e| fail("duplikace", e))?;
    check!(weak.log_read(0, &mut buf).err() == Some(Error::AccessDenied), "LOG_READ bez DIAG");
    check!(
        weak.crash_record(abi::syscall::crash_op::CLEAR, &mut []).err() == Some(Error::AccessDenied),
        "CRASH_RECORD bez DIAG"
    );
    // Mimo testovaci zaznam jadro paniku nevyvola - jen v qemu-test; tady
    // by test ukoncil beh, proto se nevola.
    check!(
        root.crash_record(abi::syscall::crash_op::CAPACITY, &mut []).map_err(|e| fail("CAPACITY", e))? > 0,
        "oblast zaznamu padu chybi (loader ji nevyhradil)"
    );
    Ok(())
}

fn fs() -> Result<&'static fsproto::FsClient, String> {
    crate::system::devices().and_then(|d| d.fs.as_ref()).ok_or_else(|| String::from("FS neni"))
}

fn log_file(_s: &Startup) -> TestResult {
    let fs = fs()?;
    let list = fs.list(crate::logd::LOG_DIR).map_err(|e| fail("vypis /system/log", e))?;
    let name = list
        .iter()
        .map(|e| e.name.as_str())
        .filter(|n| n.starts_with("boot-") && n.ends_with(".log"))
        .max()
        .ok_or("chybi boot-*.log")?;
    let path = alloc::format!("{}/{name}", crate::logd::LOG_DIR);
    let before = fs.stat(&path).map_err(|e| fail("stat logu", e))?.size;
    println!("init: test diagnostiky: znacka do logu");
    // logd zapisuje kazdou sekundu.
    let deadline = time::deadline_after(Duration::from_secs(10));
    loop {
        let now = fs.stat(&path).map_err(|e| fail("stat logu", e))?.size;
        if now > before {
            return Ok(());
        }
        check!(time::now() < deadline, "log bootu neroste");
        time::sleep(Duration::from_millis(200));
    }
}

fn report(_s: &Startup) -> TestResult {
    let (path, preview) = crate::logd::report().map_err(|e| fail("report", e))?;
    println!("init: balicek {path}:\n{preview}");
    check!(preview.contains("log tohoto bootu"), "nahled bez logu bootu");
    let st = fs()?.stat(&path).map_err(|e| fail("stat balicku", e))?;
    check!(st.size > 1000, "balicek je skoro prazdny");
    Ok(())
}

/// Je to zvlastni beh testu zaznamu padu (`crash-test` v initfs)?
pub fn crash_mode(startup: &Startup) -> bool {
    libos::initfs::Initfs::open(startup).ok().and_then(|f| f.find("crash-test").map(|_| ())).is_some()
}

/// Zvlastni beh: 1. start vyvola paniku jadra (zaznam do pstore, restart),
/// 2. start najde zaznam ulozeny logd v `/system/pady`, overi ho a vytvori
/// balicek. Vraci kod initu (0 = uspech, jadro ukonci QEMU).
pub fn scenario(startup: &Startup) -> i64 {
    match scenario_inner(startup) {
        Ok(()) => {
            println!("{MARKER_OK}");
            0
        }
        Err(e) => {
            println!("init: test padu selhal: {e}");
            1
        }
    }
}

fn scenario_inner(startup: &Startup) -> TestResult {
    let fs = fs()?;
    let crash = fs
        .list(crate::logd::CRASH_DIR)
        .map_err(|e| fail("vypis /system/pady", e))?
        .into_iter()
        .map(|e| e.name)
        .find(|n| n.contains("-jadro-") && n.ends_with(".crash"));
    let Some(name) = crash else {
        // Prvni start: panika (jadro zapise zaznam a restartuje).
        let root = startup.root_resource.as_ref().ok_or("chybi korenovy Resource")?;
        // Test overuje zaznam padu, ne obnovu FS po vypadku: nejdriv vse
        // zapsat (commit), aby panika neprerusila rozpracovany zapis.
        fs.commit().map_err(|e| fail("commit pred panikou", e))?;
        time::sleep(Duration::from_secs(2));
        println!("{MARKER_PANIC}");
        let r = root.crash_record(abi::syscall::crash_op::TEST_PANIC, &mut []);
        return Err(alloc::format!("TEST_PANIC se vratil: {r:?}"));
    };
    let path = alloc::format!("{}/{name}", crate::logd::CRASH_DIR);
    let size = fs.stat(&path).map_err(|e| fail("stat zaznamu", e))?.size as usize;
    let data = fs.read(&path, 0, size.min(60_000)).map_err(|e| fail("cteni zaznamu", e))?;
    let text = String::from_utf8_lossy(&data);
    check!(text.contains("duvod:   KERNEL_PANIC"), "zaznam bez duvodu KERNEL_PANIC");
    check!(text.contains("TEST_PANIC"), "zaznam bez hlaseni paniky");
    check!(text.contains("pad vyvolany testem"), "zaznam bez priznaku testu");
    let root = startup.root_resource.as_ref().ok_or("chybi korenovy Resource")?;
    let left = root.crash_record(abi::syscall::crash_op::READ, &mut vec![0u8; 70_000]).map_err(|e| fail("READ", e))?;
    check!(left == 0, "zaznam v jadre zustal po ulozeni");
    let (rep, preview) = crate::logd::report().map_err(|e| fail("report", e))?;
    println!("init: balicek {rep}:\n{preview}");
    check!(preview.contains(&name), "balicek bez zaznamu padu");
    Ok(())
}
