//! Knihovna C `cstd` (docs/cs/cstd.md): program v C (`ctest`) nad std -
//! argumenty, getenv, formatovani, malloc, soubory pres FILE, errno,
//! stderr a atexit s vyprazdnenim stdout pri konci procesu.

use alloc::vec;
use core::time::Duration;

use libos::Startup;
use libos::abi::params::tag;
use libos::spawn::StartHandle;

use super::std_app::run_std_app;
use super::{Test, TestResult, devices, fail};
use crate::check;
use libos::Error;

pub const TESTS: &[Test] = &[("ctest: stdio, soubory, errno, getenv, atexit", ctest)];

fn ctest(s: &Startup) -> TestResult {
    let fs = devices().and_then(|d| d.fs.as_ref()).ok_or("sluzba fsd nebezi")?;
    match fs.create("/std-test", fsproto::kind::DIR) {
        Ok(()) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(fail("adresar /std-test", e)),
    }
    let own = fs.connect().map_err(|e| fail("relace fsd", e))?;
    let run = run_std_app(
        s,
        "ctest",
        &["jedna", "dva"],
        &["CTEST_DIR=/std-test", "CTEST_HELLO=svet", "TZ=Europe/Prague"],
        vec![StartHandle::tagged(tag::FS, own.into_channel().into_handle())],
        Duration::from_secs(120),
    )?;
    run.expect("ctest", &["args", "getenv", "format", "memory", "files", "errno", "posix", "threads", "time", "math"])?;
    check!(run.stdout.ends_with("ctest: hotovo\n"), "atexit v obracenem poradi a vyprazdneni pri konci");
    check!(run.stderr.contains("chyba na stderr\n"), "stderr");
    check!(run.stderr.contains("ctest perror: Invalid argument\n"), "perror");
    Ok(())
}
