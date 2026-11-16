//! Beh C++ (krok 7a-1, docs/cs/cpp.md): program `cpptest` (user/cpptest)
//! staticky s `libc++.a` a jako PIE nad `/system/lib/libc++.so` -
//! kontejnery, vyjimky (i pres ramec C), RTTI, zarovnany `new`, vlakna
//! a `thread_local`, `chrono`, `random_device`, `std::filesystem` nad fsd,
//! proudy (`iostream`, `fstream`), locale a siroke znaky, `format`, `regex`.
//! Port HarfBuzz (C++ bez vyjimek, volany z C): `hbtest` tvaruje text
//! pismem Inter z initfs.

use alloc::format;
use core::time::Duration;

use libos::Startup;
use libos::initfs::Initfs;

use super::cports::{fresh_dir, fs, session};
use super::std_app::run_std_app;
use super::{Test, TestResult, fail};
use crate::check;

pub const TESTS: &[Test] = &[
    ("cpptest: libc++ staticky (vyjimky, RTTI, vlakna, filesystem)", cpptest_static),
    ("cpptest-dyn: libc++.so pres zavadec", cpptest_dyn),
    ("hbtest: HarfBuzz tvaruje text (diakritika, kerning)", hbtest),
    ("uklid: pracovni soubory sady", uklid),
];

const PARTS: &[&str] = &[
    "staticke-konstruktory",
    "kontejnery",
    "vyjimky",
    "vyjimka-pres-ramec-c",
    "rtti",
    "pamet",
    "vlakna",
    "cas-nahoda",
    "filesystem",
    "iostream",
    "proudy",
    "locale-siroke-znaky",
    "format-regex",
];

fn run(s: &Startup, name: &str) -> TestResult {
    let fs = fs()?;
    let dir = format!("/std-test/{name}");
    fresh_dir(fs, &dir, &[])?;
    let run = run_std_app(s, name, &[&dir], &[], session(fs)?, Duration::from_secs(60))?;
    run.expect("cpptest", PARTS)?;
    // Destruktor globalniho objektu (__cxa_atexit) az po main.
    check!(
        run.stdout.ends_with("cpptest: destruktor globalniho objektu\n"),
        format!("{name}: destruktor statickeho objektu po main: {}", run.stdout)
    );
    Ok(())
}

fn cpptest_static(s: &Startup) -> TestResult {
    run(s, "cpptest")
}

fn cpptest_dyn(s: &Startup) -> TestResult {
    run(s, "cpptest-dyn")
}

fn hbtest(s: &Startup) -> TestResult {
    let fs = fs()?;
    let dir = "/std-test/hbtest";
    fresh_dir(fs, dir, &["pismo.ttf"])?;
    let initfs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let data = initfs.find("hb-test.ttf").ok_or("hb-test.ttf neni v initfs")?;
    let path = format!("{dir}/pismo.ttf");
    fs.create(&path, fsproto::kind::FILE).map_err(|e| fail("pismo: vytvoreni", e))?;
    fs.write(&path, 0, data).map_err(|e| fail("pismo: zapis", e))?;
    let run = run_std_app(s, "hbtest", &[&path], &[], session(fs)?, Duration::from_secs(60))?;
    run.expect("hbtest", &["verze", "tvarovani", "kerning"])
}

/// Posledni test sady: smaze pracovni slozka programu C++ (`state.rs` jinak sadu
/// oznaci za selhanou; bezi i po chybe drivejsich testu).
fn uklid(_: &Startup) -> TestResult {
    super::restore::remove_paths(&["/std-test"])
}
