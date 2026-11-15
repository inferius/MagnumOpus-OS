//! Windows vrstva (W1, `docs/cs/windows-vrstva.md`): Wine na datovem disku
//! v `/apps/org.winehq.wine` (beh s `--wine`). Program se spousti pres
//! `launchd` jako aplikace ze souboru (zavadec dostane relaci FS, knihovny
//! z disku DL-4c).

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use fsproto::FsClient;
use launchproto::{Launch, Role};
use libos::{Channel, Error, Process, Startup, time};

use super::{Test, TestResult, devices, fail};
use crate::check;

pub const TESTS: &[Test] = &[("wine: zavadec z disku nahraje ntdll.so (wine --version)", wine_version)];

/// Adresar aplikace Wine (`xtask/src/datadisk.rs`, `WINE_APP`).
const APP: &str = "/apps/org.winehq.wine";

fn fs() -> Result<&'static FsClient, String> {
    devices().and_then(|d| d.fs.as_ref()).ok_or_else(|| String::from("sluzba fsd nebezi"))
}

/// Vysledek programu spusteneho pres launchd.
struct Run {
    code: i64,
    stdout: String,
    stderr: String,
}

/// Spusti program `path` ze souboru pres launchd (job, relace FS, stdout
/// a stderr jako kanaly) a pocka na konec; supervisor se pritom ridi (libd
/// po restartu obnovuje relace pres listky).
fn launch_path(s: &Startup, path: &str, args: &[&str], env: &[&str], limit: Duration) -> Result<Run, String> {
    let fs = fs()?;
    let launch = crate::launch::client().map_err(|e| fail("kanal k launchd", e))?;
    let params = libos::params::Params {
        cwd: b"/".to_vec(),
        args: core::iter::once(path).chain(args.iter().copied()).map(|a| a.as_bytes().to_vec()).collect(),
        env: env.iter().map(|e| e.as_bytes().to_vec()).collect(),
        flags: 0,
        fds: Vec::new(),
    }
    .encode()
    .map_err(|e| fail("blok parametru", e))?;
    let req = Launch { program: String::from(path), flags: 0, roles: vec![Role::Job, Role::Fs, Role::Stdout, Role::Stderr], params }
        .encode(1)
        .map_err(|e| fail("pozadavek", e))?;
    let job = libos::job::current().map_err(|e| fail("job", e))?;
    let own = fs.connect().map_err(|e| fail("relace FS", e))?;
    let (out, out_child) = Channel::create().map_err(|e| fail("kanal stdout", e))?;
    let (err, err_child) = Channel::create().map_err(|e| fail("kanal stderr", e))?;
    launch
        .write(
            &req,
            vec![
                job.duplicate(None).map_err(|e| fail("job", e))?.into_handle(),
                own.into_channel().into_handle(),
                out_child.into_handle(),
                err_child.into_handle(),
            ],
        )
        .map_err(|e| fail("LAUNCH", e))?;
    let end = time::deadline_after(limit);
    let step = || crate::supervisor::SUPERVISOR.lock().step(s, Some(time::deadline_after(Duration::from_millis(2))));
    let m = loop {
        if let Ok(m) = launch.try_read() {
            break m;
        }
        if time::now() > end {
            return Err(String::from("launchd neodpovedel"));
        }
        step();
    };
    let (_, r) = launchproto::parse_reply(&m.data).map_err(|e| fail("odpoved launchd", e))?;
    if let Err((e, msg)) = r {
        return Err(format!("launchd: {e}: {msg}"));
    }
    let mut hs = m.handles;
    let _control = Channel::from_handle(hs.pop().ok_or("chybi kanal")?);
    let process = Process::from_handle(hs.pop().ok_or("chybi proces")?);
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let code = loop {
        while let Ok(m) = out.try_read() {
            stdout.extend_from_slice(&m.data);
        }
        while let Ok(m) = err.try_read() {
            stderr.extend_from_slice(&m.data);
        }
        match process.wait_exit(Some(time::deadline_after(Duration::from_millis(2)))) {
            Ok(c) => break c,
            Err(Error::TimedOut) if time::now() <= end => {
                step();
            }
            Err(e) => {
                let _ = process.kill();
                return Err(format!("{e}; stdout: {}; stderr: {}", String::from_utf8_lossy(&stdout), String::from_utf8_lossy(&stderr)));
            }
        }
    };
    // Zbytek vystupu po konci procesu.
    for (ch, buf) in [(&out, &mut stdout), (&err, &mut stderr)] {
        while let Ok(m) = ch.read(Some(time::deadline_after(Duration::from_millis(200)))) {
            buf.extend_from_slice(&m.data);
        }
    }
    let run = Run { code, stdout: String::from_utf8_lossy(&stdout).into_owned(), stderr: String::from_utf8_lossy(&stderr).into_owned() };
    for line in run.stdout.lines().chain(run.stderr.lines()) {
        libos::println!("init: wine: {line}");
    }
    Ok(run)
}

fn wine_version(s: &Startup) -> TestResult {
    let fs = fs()?;
    let loader = format!("{APP}/lib/wine/x86_64-unix/wine");
    if fs.stat(&loader).is_err() {
        return Err(format!("{loader} neni na disku (beh testu s --wine, nejdriv `cargo xtask wine`)"));
    }
    let run = launch_path(s, &loader, &["--version"], &[], Duration::from_secs(60))?;
    check!(run.code == 0 && run.stdout.trim() == "wine-11.0", format!("kod {}; stdout: {}; stderr: {}", run.code, run.stdout.trim(), run.stderr.trim()));
    Ok(())
}
