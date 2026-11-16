//! Knihovny z disku (DL-4c, `docs/cs/dynamicke-linkovani.md`): program
//! `dltest-origin` a jeho knihovny se zkopiruji z initfs (`dl4c/`) na disk
//! do `/dl4c/{bin,lib}`; zavadec je nahraje relaci FS procesu pres
//! `DT_RUNPATH` s `$ORIGIN` a obsah da libd (`OpenImage`).

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use fsproto::FsClient;
use launchproto::{Launch, Role};
use libos::initfs::Initfs;
use libos::spawn::{Options, Origin};
use libos::job::Job;
use libos::{Channel, Error, Process, Startup, time};

use super::{Test, TestResult, devices, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("dynamicke linkovani: knihovny z disku ($ORIGIN, dlopen cesty) relaci FS procesu", disk_libraries),
    ("dynamicke linkovani: knihovny z disku sdili sablony, initfs bez relace je nenajde", disk_shares_and_needs_session),
    ("dynamicke linkovani: launchd preda zavadeci relaci FS programu ze souboru", via_launchd),
    ("dynamicke linkovani: launchd da JIT jen s [app] jit pod jobem s GRANT", launchd_jit_gate),
    ("uklid: pracovni soubory sady", uklid),
];

const PROGRAM: &str = "/dl4c/bin/dltest-origin";
/// Manifest s `jit = true` pro test brany JIT v launchd.
const JIT_MANIFEST: &[u8] = b"[app]\nid = \"org.magnumopus.dltest-origin\"\njit = true\n";
const FILES: &[&str] = &["bin/dltest-origin", "lib/libdlorigin.so.1", "lib/libdlorigin2.so.1", "lib/libdlorigin3.so.1"];

fn fs() -> Result<&'static FsClient, String> {
    devices().and_then(|d| d.fs.as_ref()).ok_or_else(|| String::from("sluzba fsd nebezi"))
}

fn deadline() -> Option<u64> {
    Some(time::deadline_after(Duration::from_secs(20)))
}

/// Zkopiruje program a knihovny z initfs na disk (`/dl4c`).
fn install(s: &Startup, fs: &FsClient) -> TestResult {
    let initfs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    for d in ["/dl4c", "/dl4c/bin", "/dl4c/lib"] {
        match fs.create(d, fsproto::kind::DIR) {
            Ok(()) | Err(Error::AlreadyExists) => {}
            Err(e) => return Err(fail(d, e)),
        }
    }
    for f in FILES {
        let data = initfs.find(&format!("dl4c/{f}")).ok_or_else(|| format!("dl4c/{f} neni v initfs"))?;
        let path = format!("/dl4c/{f}");
        let _ = fs.remove(&path);
        fs.create(&path, fsproto::kind::FILE).map_err(|e| fail(&path, e))?;
        fs.write(&path, 0, data).map_err(|e| fail(&path, e))?;
    }
    Ok(())
}

/// Spusti program z disku primo (`spawn_from`) s relaci FS pro zavadec.
fn run_from_disk(fs: &FsClient) -> Result<i64, String> {
    let data = fs.read(PROGRAM, 0, 1 << 22).map_err(|e| fail(PROGRAM, e))?;
    let own = fs.connect().map_err(|e| fail("relace FS", e))?;
    let origin = Origin { fs: own.into_channel().into_handle(), path: String::from(PROGRAM) };
    let job = libos::job::current().map_err(|e| fail("job", e))?;
    let (p, _, _ch) = libos::spawn::spawn_from(&job, "dltest-origin", &data, Vec::new(), Options::default(), Some(origin))
        .map_err(|e| fail("spusteni", e))?;
    p.wait_exit(deadline()).map_err(|e| fail("konec programu", e))
}

fn disk_libraries(s: &Startup) -> TestResult {
    let fs = fs()?;
    install(s, fs)?;
    let code = run_from_disk(fs)?;
    check_eq!(
        code,
        0,
        "kontroly programu (1 knihovna z $ORIGIN, 2/4 dlopen cesty, 8 tataz cesta jinak, 16 SONAME, 32 hole jmeno z disku nesmi)"
    );
    Ok(())
}

fn disk_shares_and_needs_session(s: &Startup) -> TestResult {
    let fs = fs()?;
    install(s, fs)?;
    check_eq!(run_from_disk(fs)?, 0, "prvni beh");
    let a = libos::libd::stats().map_err(|e| fail("statistika libd", e))?;
    check_eq!(run_from_disk(fs)?, 0, "druhy beh");
    let b = libos::libd::stats().map_err(|e| fail("statistika libd", e))?;
    check!(a.disk_templates >= 3, format!("sablony z disku: {}", a.disk_templates));
    check_eq!(b.templates, a.templates, "zadna nova sablona pri druhem behu (stejny obsah)");
    // libdlorigin, libdlorigin2, libdlorigin3 (+ libdltest2 z initfs).
    check!(b.hits >= a.hits + 3, format!("zasahy mezipameti {} -> {}", a.hits, b.hits));
    // Tentyz program z initfs bez relace FS: knihovny z disku nenajde
    // (libd podle jmena hleda jen /system/lib) -> kod zavadece.
    let initfs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = initfs.find("dl4c/bin/dltest-origin").ok_or("dl4c/bin/dltest-origin neni v initfs")?;
    let (p, _ch) = libos::spawn("dltest-origin", image, Vec::new()).map_err(|e| fail("spusteni", e))?;
    let code = p.wait_exit(deadline()).map_err(|e| fail("konec programu", e))?;
    check_eq!(code, abi::auxv::EXIT_LOAD_FAILED, "bez relace FS knihovny z disku nejsou");
    Ok(())
}

fn via_launchd(s: &Startup) -> TestResult {
    let fs = fs()?;
    install(s, fs)?;
    let job = libos::job::current().map_err(|e| fail("job", e))?;
    let code = launch(s, fs, &job)?.map_err(|(e, msg)| format!("launchd: {e}: {msg}"))?;
    check_eq!(code, 0, "program ze souboru pres launchd (bity viz dltest-origin)");
    Ok(())
}

/// launchd je brana JIT: `[app] jit = true` dostane `jit::RUN` jen pod
/// jobem volajiciho s `GRANT` (init ho ma); pod jobem bez `GRANT` chyba,
/// program bez manifestu tam bezi bez JIT.
fn launchd_jit_gate(s: &Startup) -> TestResult {
    let fs = fs()?;
    install(s, fs)?;
    let manifest = format!("{PROGRAM}.manifest.toml");
    let _ = fs.remove(&manifest);
    let plain = Job::create(&libos::job::current().map_err(|e| fail("job", e))?).map_err(|e| fail("job bez JIT", e))?;
    // Bez manifestu: pod jobem bez GRANT normalne (zadny JIT).
    check_eq!(launch(s, fs, &plain)?.map_err(|(e, m)| format!("{e}: {m}"))?, 0, "bez manifestu pod jobem bez GRANT");
    fs.create(&manifest, fsproto::kind::FILE).map_err(|e| fail(&manifest, e))?;
    fs.write(&manifest, 0, JIT_MANIFEST).map_err(|e| fail(&manifest, e))?;
    let denied = launch(s, fs, &plain)?;
    let own = libos::job::current().map_err(|e| fail("job", e))?;
    let granted = launch(s, fs, &own)?;
    let _ = fs.remove(&manifest);
    match denied {
        Err((Error::AccessDenied, msg)) => check!(msg.contains("JIT"), format!("hlaska bez JIT: {msg}")),
        other => return Err(format!("jit = true pod jobem bez GRANT: ocekavana chyba ACCESS_DENIED, je {other:?}")),
    }
    check_eq!(granted.map_err(|(e, m)| format!("{e}: {m}"))?, 0, "jit = true pod jobem s GRANT");
    Ok(())
}

/// LAUNCH programu `PROGRAM` pres launchd s jobem `job` jako jobem
/// volajiciho; vrati kod programu nebo chybu launchd. Supervisor se pri
/// cekani ridi (launchd muze po restartu libd obnovovat relaci pres
/// listek, testovaci beh ho jinak neobsluhuje).
fn launch(s: &Startup, fs: &FsClient, job: &Job) -> Result<Result<i64, (Error, String)>, String> {
    let launch = crate::launch::client().map_err(|e| fail("kanal k launchd", e))?;
    let params = libos::params::Params { cwd: b"/".to_vec(), args: vec![PROGRAM.as_bytes().to_vec()], env: Vec::new(), flags: 0, fds: Vec::new() }
        .encode()
        .map_err(|e| fail("blok parametru", e))?;
    let req = Launch { program: String::from(PROGRAM), flags: 0, roles: vec![Role::Job, Role::Fs], params }
        .encode(1)
        .map_err(|e| fail("pozadavek", e))?;
    let own = fs.connect().map_err(|e| fail("relace FS", e))?;
    launch
        .write(&req, vec![job.duplicate(None).map_err(|e| fail("job", e))?.into_handle(), own.into_channel().into_handle()])
        .map_err(|e| fail("LAUNCH", e))?;
    let end = time::deadline_after(Duration::from_secs(20));
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
    if let Err(e) = r {
        return Ok(Err(e));
    }
    let mut hs = m.handles;
    check_eq!(hs.len(), 2, "proces a kanal v odpovedi");
    let _control = Channel::from_handle(hs.pop().ok_or("chybi kanal")?);
    let process = Process::from_handle(hs.pop().ok_or("chybi proces")?);
    loop {
        match process.wait_exit(Some(time::deadline_after(Duration::from_millis(2)))) {
            Ok(c) => return Ok(Ok(c)),
            Err(Error::TimedOut) if time::now() <= end => {
                step();
            }
            Err(e) => return Err(fail("konec programu", e)),
        }
    }
}

/// Posledni test sady: smaze knihovny DL-4c zkopirovane na disk (`state.rs` jinak sadu
/// oznaci za selhanou; bezi i po chybe drivejsich testu).
fn uklid(_: &Startup) -> TestResult {
    super::restore::remove_paths(&["/dl4c"])
}
