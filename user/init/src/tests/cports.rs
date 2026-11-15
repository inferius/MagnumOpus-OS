//! Porty v C prelozene proti sysrootu C (`target/c-sysroot`, libc.a,
//! docs/cs/cstd.md): zlib (vlastni test `example`, `minigzip`) a sqlite
//! (shell a dva procesy nad jednou databazi se zamky fcntl ve fsd).

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use fsproto::FsClient;
use libos::Startup;
use libos::abi::params::tag;
use libos::spawn::StartHandle;

use super::std_app::{run_std_app, start_std_app};
use super::{Test, TestResult, devices, fail};
use crate::check;
use libos::Error;

pub const TESTS: &[Test] = &[
    ("zlib: vlastni test example, minigzip a zpet", zlib),
    ("sqlite: shell sqlite3 nad souborem", sqlite_shell),
    ("sqlite: dva procesy nad jednou databazi (zamky fcntl ve fsd)", sqlite_two),
];

pub(super) fn fs() -> Result<&'static FsClient, String> {
    devices().and_then(|d| d.fs.as_ref()).ok_or_else(|| String::from("sluzba fsd nebezi"))
}

/// Cisty adresar testu pod `/std-test`.
pub(super) fn fresh_dir(fs: &FsClient, dir: &str, files: &[&str]) -> TestResult {
    for d in ["/std-test", dir] {
        match fs.create(d, fsproto::kind::DIR) {
            Ok(()) | Err(Error::AlreadyExists) => {}
            Err(e) => return Err(fail(d, e)),
        }
    }
    for f in files {
        let _ = fs.remove(&format!("{dir}/{f}"));
    }
    Ok(())
}

pub(super) fn session(fs: &FsClient) -> Result<Vec<StartHandle>, String> {
    let own = fs.connect().map_err(|e| fail("relace fsd", e))?;
    Ok(vec![StartHandle::tagged(tag::FS, own.into_channel().into_handle())])
}

fn zlib(s: &Startup) -> TestResult {
    let fs = fs()?;
    let dir = "/std-test/zlib";
    fresh_dir(fs, dir, &["foo.gz", "data.txt", "data.txt.gz"])?;
    // Vlastni test zlib (komprese v pameti, gz soubor, inflateSync, slovnik).
    let run = run_std_app(s, "zlib-example", &["/std-test/zlib/foo.gz"], &[], session(fs)?, Duration::from_secs(60))?;
    check!(run.code == 0, format!("example: kod {}; {}", run.code, run.stderr.trim()));
    for want in ["zlib version 1.3.1", "uncompress(): hello, hello!", "gzgets() after gzseek:  hello!", "after inflateSync(): hello, hello!", "inflate with dictionary: hello, hello!"] {
        check!(run.stdout.contains(want), format!("example: chybi '{want}' v {}", run.stdout));
    }
    // minigzip: komprese souboru a zpet, obsah se nezmeni.
    let data: Vec<u8> = (0..20_000u32).flat_map(|i| format!("radek {i} {}\n", i % 97).into_bytes()).collect();
    let path = format!("{dir}/data.txt");
    fs.create(&path, fsproto::kind::FILE).map_err(|e| fail("data.txt", e))?;
    fs.write(&path, 0, &data).map_err(|e| fail("zapis data.txt", e))?;
    let r = run_std_app(s, "minigzip", &[&path], &[], session(fs)?, Duration::from_secs(60))?;
    check!(r.code == 0, format!("minigzip: kod {}; {}", r.code, r.stderr.trim()));
    let gz = fs.read(&format!("{path}.gz"), 0, 1 << 20).map_err(|e| fail("data.txt.gz", e))?;
    check!(gz.len() > 10 && gz.len() < data.len() / 4 && gz[..2] == [0x1f, 0x8b], format!("gzip {} B", gz.len()));
    let r = run_std_app(s, "minigzip", &["-d", &format!("{path}.gz")], &[], session(fs)?, Duration::from_secs(60))?;
    check!(r.code == 0, format!("minigzip -d: kod {}; {}", r.code, r.stderr.trim()));
    let back = fs.read(&path, 0, 1 << 20).map_err(|e| fail("data.txt zpet", e))?;
    check!(back == data, "obsah po kompresi a dekompresi");
    Ok(())
}

fn sqlite_shell(s: &Startup) -> TestResult {
    let fs = fs()?;
    let dir = "/std-test/sql";
    fresh_dir(fs, dir, &["shell.db", "shell.db-journal"])?;
    let sql = "CREATE TABLE t(a INTEGER, b TEXT); \
               INSERT INTO t VALUES(2,'dva'),(1,'jedna'),(3,'tri'); \
               SELECT b FROM t ORDER BY a; SELECT sum(a), printf('%.2f', avg(a)) FROM t;";
    let run = run_std_app(s, "sqlite3", &["/std-test/sql/shell.db", sql], &[], session(fs)?, Duration::from_secs(60))?;
    check!(run.code == 0, format!("sqlite3: kod {}; {}", run.code, run.stderr.trim()));
    check!(run.stdout.contains("jedna\ndva\ntri\n6|2.00\n"), format!("vystup: {}", run.stdout));
    // Druhy beh nad stejnym souborem cte ulozena data.
    let run = run_std_app(s, "sqlite3", &["/std-test/sql/shell.db", "SELECT count(*) FROM t;"], &[], session(fs)?, Duration::from_secs(60))?;
    check!(run.stdout.trim() == "3", format!("trvalost: {}", run.stdout));
    Ok(())
}

fn sqlite_two(s: &Startup) -> TestResult {
    let fs = fs()?;
    let dir = "/std-test/sql2";
    fresh_dir(fs, dir, &["two.db", "two.db-journal", "a-ready", "a-locked", "b-busy", "a-done"])?;
    let db = "/std-test/sql2/two.db";
    let a = start_std_app(s, "sqltest", &["a", db, dir], &[], session(fs)?)?;
    let b = start_std_app(s, "sqltest", &["b", db, dir], &[], session(fs)?)?;
    let rb = b.finish(Duration::from_secs(90))?;
    let ra = a.finish(Duration::from_secs(90))?;
    ra.expect("sqltest", &["sqlite-a"])?;
    rb.expect("sqltest", &["sqlite-b"])
}

pub const DYN_TESTS: &[Test] = &[
    ("cdyn: program PIE v C nad libc.so (vlakna, TLS, dlopen pluginu)", cdyn),
    ("cdyn-hello: program jen z clang-dyn.cfg sysrootu C", cdyn_hello),
];

fn cdyn_hello(s: &Startup) -> TestResult {
    let run = run_std_app(s, "cdyn-hello", &["x"], &[], Vec::new(), Duration::from_secs(30))?;
    run.expect("cdyn-hello", &["hello 2 cdyn-hello"])
}

/// Program PIE v C nad `libc.so` (zavadec ld.so, knihovny z libd).
fn cdyn(s: &Startup) -> TestResult {
    let fs = fs()?;
    fresh_dir(fs, "/std-test/cdyn", &["cdyn.txt"])?;
    let run = run_std_app(s, "cdyn", &["test"], &["CDYN_DIR=/std-test/cdyn"], session(fs)?, Duration::from_secs(60))?;
    run.expect("cdyn", &["args", "libc", "mem-ld", "dlopen", "plugin", "dlsym-chyba", "vlakna-tls", "soubory", "dlclose"])
}
