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

pub const TESTS: &[Test] = &[
    ("wine: zavadec z disku nahraje ntdll.so (wine --version)", wine_version),
    ("wine: novy prefix a konzolovy cmd /c echo (pomaly, --release)", wine_cmd),
    ("wine: vlastni konzolovy program (printf, argv, prostredi, soubory, vlakno, kod konce)", wine_program),
    ("wine: hostname", wine_hostname),
    ("wine: dalsi proces, pojmenovane udalosti, Read/WriteProcessMemory (W2)", wine_processes),
];

/// Testovaci programy Windows na disku (`xtask/src/datadisk.rs`).
const TESTS_APP: &str = "/apps/org.magnumopus.wintest";
/// Prostredi pro programy Windows v testech (prefix z `wine_cmd`, jinak se
/// zalozi novy - pomale).
const WINE_ENV: &[&str] = &["HOME=/home/wine", "WINEPREFIX=/home/wine/.wine"];

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
    // Bez stdin: Wine posila serveru fd 0-2, prazdny stdin jde predat jako
    // null fd (osrt::inherit).
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
    let step = || crate::supervisor::SUPERVISOR.lock().step(s, Some(time::deadline_after(Duration::from_millis(50))));
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

/// Novy prefix (wineboot: services, rundll32 setupapi nad wine.inf,
/// registr, C:\windows\system32) a konzolovy `cmd.exe`. Pomaly test:
/// Wine na wineboot ceka nejvys 5 minut, v QEMU TCG to stihne jen sestaveni
/// release (`--release`, ~70 s); debug je asi 20x pomalejsi (cteni z fsd).
fn wine_cmd(s: &Startup) -> TestResult {
    let fs = fs()?;
    let loader = format!("{APP}/lib/wine/x86_64-unix/wine");
    for d in ["/home", "/home/wine"] {
        match fs.create(d, fsproto::kind::DIR) {
            Ok(()) | Err(Error::AlreadyExists) => {}
            Err(e) => return Err(fail(d, e)),
        }
    }
    let t0 = time::now();
    let run = launch_path(s, &loader, &["cmd", "/c", "echo", "ahoj"], WINE_ENV, Duration::from_secs(600))?;
    libos::println!("init: wine: novy prefix a cmd /c echo za {} ms", (time::now() - t0) / 1_000_000);
    check!(
        run.code == 0 && run.stdout.lines().any(|l| l.trim() == "ahoj"),
        format!("kod {}; stdout: {}; stderr: {}", run.code, run.stdout.trim(), run.stderr.trim())
    );
    // Prefix: disky jako symbolicke odkazy (fs-symlink), registr, system32.
    for p in ["/home/wine/.wine/system.reg", "/home/wine/.wine/dosdevices/c:/windows/system32/kernel32.dll"] {
        fs.stat(p).map_err(|e| fail(p, e))?;
    }
    Ok(())
}

fn wine_program(s: &Startup) -> TestResult {
    let loader = format!("{APP}/lib/wine/x86_64-unix/wine");
    let exe = format!("{TESTS_APP}/wintest.exe");
    fs()?.stat(&exe).map_err(|e| fail(&exe, e))?;
    let mut env = WINE_ENV.to_vec();
    env.push("WINTEST_VAR=hodnota");
    let run = launch_path(s, &loader, &[&exe, "alfa", "beta gama"], &env, Duration::from_secs(600))?;
    check!(
        run.code == 42 && run.stdout.contains("wintest: start") && run.stdout.contains("wintest: chyby 0"),
        format!("kod {} (42 = ok, 100 + bity chyb viz user/wintest); stdout: {}; stderr: {}", run.code, run.stdout.trim(), run.stderr.trim())
    );
    Ok(())
}

fn wine_hostname(s: &Startup) -> TestResult {
    let loader = format!("{APP}/lib/wine/x86_64-unix/wine");
    let run = launch_path(s, &loader, &["hostname"], WINE_ENV, Duration::from_secs(600))?;
    let name = run.stdout.trim();
    check!(
        run.code == 0 && !name.is_empty() && !name.contains(' '),
        format!("kod {}; stdout: {}; stderr: {}", run.code, name, run.stderr.trim())
    );
    Ok(())
}

fn wine_processes(s: &Startup) -> TestResult {
    let loader = format!("{APP}/lib/wine/x86_64-unix/wine");
    let exe = format!("{TESTS_APP}/wintest.exe");
    let run = launch_path(s, &loader, &[&exe, "procesy"], WINE_ENV, Duration::from_secs(600))?;
    check!(
        run.code == 42 && run.stdout.contains("wintest procesy: chyby 0"),
        format!("kod {} (42 = ok, 100 + bity viz user/wintest); stdout: {}; stderr: {}", run.code, run.stdout.trim(), run.stderr.trim())
    );
    Ok(())
}
