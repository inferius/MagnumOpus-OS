//! Prevzate aplikace z crates.io bez uprav (docs/cs/rust-std.md, Porty):
//! ripgrep hleda ve stromu souboru na nasem FS (vlakna prochazeni,
//! `std::fs`, regex, vypis na stdout).

use alloc::format;
use alloc::vec;
use core::time::Duration;

use libos::abi::params::tag;
use libos::spawn::StartHandle;
use libos::{Error, Startup};

use super::std_app::run_std_app;
use super::net::{HOST, configured, net};
use super::{Test, TestResult, devices, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("ripgrep: hledani ve strome souboru", ripgrep),
    ("ureq: HTTP klient, 4 vlakna soubezne", http),
    ("getrandom: backend RANDOM_GET", getrandom),
    ("uklid: pracovni soubory sady", uklid),
];

const DIR: &str = "/std-test/rg";

fn ripgrep(s: &Startup) -> TestResult {
    let fs = devices().and_then(|d| d.fs.as_ref()).ok_or("sluzba fsd nebezi")?;
    for dir in ["/std-test", DIR, "/std-test/rg/pod", "/std-test/rg/pod/hloub"] {
        match fs.create(dir, fsproto::kind::DIR) {
            Ok(()) | Err(Error::AlreadyExists) => {}
            Err(e) => return Err(fail(dir, e)),
        }
    }
    let files: [(&str, &[u8]); 4] = [
        ("/std-test/rg/a.txt", b"prvni radek\nhledam JEHLU v kupce\ntreti\n"),
        ("/std-test/rg/b.txt", b"tady nic neni\n"),
        ("/std-test/rg/pod/c.rs", b"fn main() {\n    let jehla = 1;\n}\n"),
        ("/std-test/rg/pod/hloub/d.txt", b"jehla jehla\n"),
    ];
    for (path, data) in files {
        let _ = fs.remove(path);
        fs.create(path, fsproto::kind::FILE).map_err(|e| fail(path, e))?;
        fs.write(path, 0, data).map_err(|e| fail(path, e))?;
    }
    let run = |args: &[&str]| {
        let own = fs.connect().map_err(|e| fail("relace fsd", e))?;
        run_std_app(
            s,
            "rg",
            args,
            &[],
            vec![StartHandle::tagged(tag::FS, own.into_channel().into_handle())],
            Duration::from_secs(120),
        )
    };
    let v = run(&["--version"])?;
    check_eq!(v.code, 0, format!("rg --version: {}", v.stderr));
    check!(v.stdout.starts_with("ripgrep 15.2.0"), format!("verze: {}", v.stdout));

    // Bez rozliseni velikosti, cisla radku, serazene podle cesty.
    let r = run(&["-i", "-n", "--sort", "path", "jehl", DIR])?;
    check_eq!(r.code, 0, format!("rg: {}", r.stderr));
    let want = "/std-test/rg/a.txt:2:hledam JEHLU v kupce\n\
                /std-test/rg/pod/c.rs:2:    let jehla = 1;\n\
                /std-test/rg/pod/hloub/d.txt:1:jehla jehla\n";
    check_eq!(r.stdout.as_str(), want, "vystup rg");

    // Pocty shod a filtr typu souboru.
    let c = run(&["-c", "--type", "rust", "jehla", DIR])?;
    check_eq!(
        c.stdout.as_str(),
        "/std-test/rg/pod/c.rs:1\n",
        format!("rg -c --type rust (kod {}, stderr {:?})", c.code, c.stderr)
    );
    // Nic nenalezeno = kod 1 (jako na jinych systemech).
    let none = run(&["neexistujici-slovo", DIR])?;
    check_eq!(none.code, 1, "kod pri zadne shode");
    Ok(())
}

/// Port testovaciho serveru HTTP (`http` v `net-test`, `xtask/src/webhelper.rs`).
fn http_port(s: &Startup) -> Result<Option<u16>, alloc::string::String> {
    let fs = libos::initfs::Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let Some(cfg) = fs.find("net-test") else {
        libos::println!("[preskoceno] std porty: chybi pomocne servery (jen `cargo xtask test`)");
        return Ok(None);
    };
    let text = core::str::from_utf8(cfg).map_err(|_| alloc::string::String::from("net-test neni UTF-8"))?;
    Ok(text
        .lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == "http")
        .and_then(|(_, v)| v.trim().parse().ok()))
}

/// Aplikace nad knihovnou ureq (crates.io, bez uprav): TCP jen na server
/// HTTP hostitele.
fn http(s: &Startup) -> TestResult {
    use netproto::{Policy, Rule, Target, proto, rights};
    let Some(port) = http_port(s)? else {
        return Ok(());
    };
    let root = net()?;
    configured(root)?;
    let policy = Policy::only(vec![Rule::port(proto::TCP, Target::Net { addr: HOST, prefix: 32 }, port)]);
    let session = root.restrict(rights::CLIENT, &policy).map_err(|e| fail("restrict", e))?;
    let base = format!("BASE=http://{HOST}:{port}");
    let run = run_std_app(
        s,
        "std-http",
        &[],
        &[base.as_str()],
        vec![StartHandle::tagged(tag::NET, session.into_channel().map_err(|e| format!("predani site: {e:?}"))?.into_handle())],
        Duration::from_secs(120),
    )?;
    run.expect("std-http", &["hello", "echo", "parallel"])
}

/// Backend `getrandom` pro MagnumOpus (patch v `third_party/ports`).
fn getrandom(s: &Startup) -> TestResult {
    let run = run_std_app(s, "getrandom-probe", &[], &[], vec![], Duration::from_secs(60))?;
    run.expect("getrandom-probe", &["fill", "fill_big", "u64"])
}

/// Posledni test sady: smaze pracovni slozka programu std (`state.rs` jinak sadu
/// oznaci za selhanou; bezi i po chybe drivejsich testu).
fn uklid(_: &Startup) -> TestResult {
    super::restore::remove_paths(&["/std-test"])
}
