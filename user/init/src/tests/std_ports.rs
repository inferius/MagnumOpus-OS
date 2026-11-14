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
use super::{Test, TestResult, devices, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[("ripgrep: hledani ve strome souboru", ripgrep)];

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
