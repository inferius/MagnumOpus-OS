//! Knihovna C `cstd` (docs/cs/cstd.md): program v C (`ctest`) nad std -
//! argumenty, getenv, formatovani, malloc, soubory pres FILE, errno,
//! stderr a atexit s vyprazdnenim stdout pri konci procesu.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use libos::Startup;
use libos::abi::params::tag;
use libos::spawn::StartHandle;

use super::std_app::run_std_app;
use super::{Test, TestResult, devices, fail};
use crate::check;
use libos::Error;

pub const TESTS: &[Test] = &[
    ("ctest: stdio, soubory, errno, getenv, atexit", ctest),
    ("ctest lock: zamky fcntl a flock mezi dvema procesy, EDEADLK", ctest_lock),
];

fn ctest(s: &Startup) -> TestResult {
    let fs = devices().and_then(|d| d.fs.as_ref()).ok_or("sluzba fsd nebezi")?;
    match fs.create("/std-test", fsproto::kind::DIR) {
        Ok(()) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(fail("adresar /std-test", e)),
    }
    let own = fs.connect().map_err(|e| fail("relace fsd", e))?;
    // Kanal ke launchd: posix_spawn spousti ctest sam sebe (krok posix2).
    let launch = crate::launch::client().map_err(|e| fail("kanal ke launchd", e))?;
    let run = run_std_app(
        s,
        "ctest",
        &["jedna", "dva"],
        &["CTEST_DIR=/std-test", "CTEST_HELLO=svet", "TZ=Europe/Prague"],
        vec![
            StartHandle::tagged(tag::FS, own.into_channel().into_handle()),
            StartHandle::tagged(tag::LAUNCH, launch.into_handle()),
        ],
        Duration::from_secs(120),
    )?;
    run.expect("ctest", &["args", "getenv", "format", "memory", "files", "errno", "posix", "threads", "time", "math", "mmap", "signal", "wchar", "posix2"])?;
    check!(run.stdout.ends_with("ctest: hotovo\n"), "atexit v obracenem poradi a vyprazdneni pri konci");
    check!(run.stderr.contains("chyba na stderr\n"), "stderr");
    check!(run.stderr.contains("ctest perror: Invalid argument\n"), "perror");
    Ok(())
}

pub const NET_TESTS: &[Test] = &[("ctest net: sockety, getaddrinfo, poll, select, epoll, accept", ctest_net)];

/// Sockety knihovny C proti pomocnym serverum hostitele (jako `std sit`):
/// relace jen s porty testu, naslouchani pro zpetne spojeni.
fn ctest_net(s: &Startup) -> TestResult {
    use alloc::format;
    use netproto::{Policy, Rule, Target, proto, rights};

    use super::net::{HOST, configured, host, net, ports};

    let Some(p) = ports(s, "ctest net")? else {
        return Ok(());
    };
    let root = net()?;
    configured(root)?;
    // Prichozi spojeni pusti jen profil home.
    super::firewall::ensure_home()?;
    let target = Target::Net { addr: HOST, prefix: 32 };
    let mut policy = Policy::only(vec![
        Rule::port(proto::TCP, target.clone(), p.echo),
        Rule::port(proto::TCP, target.clone(), p.closed),
        Rule::port(proto::TCP, target.clone(), p.callback),
        Rule::port(proto::UDP, target, p.udp_echo),
    ]);
    // Naslouchat smi jen na portu presmerovanem z hostitele.
    policy.bind = Some(vec![(p.forwarded, p.forwarded)]);
    let session = root
        .restrict(rights::CLIENT | rights::TCP_LISTEN, &policy)
        .map_err(|e| fail("restrict", e))?;
    let env = [
        format!("ECHO={}", host(p.echo)),
        format!("UDP_ECHO={}", host(p.udp_echo)),
        format!("CLOSED={}", host(p.closed)),
        format!("REFUSES={}", u8::from(p.refuses)),
        format!("CALLBACK={}", host(p.callback)),
        format!("FORWARDED={}", p.forwarded),
        String::from("NAME=echo.magnumopus"),
    ];
    let env: Vec<&str> = env.iter().map(|e| e.as_str()).collect();
    let ch = session.into_channel().map_err(|e| format!("predani site: {e:?}"))?;
    let run = run_std_app(
        s,
        "ctest",
        &["net"],
        &env,
        vec![StartHandle::tagged(tag::NET, ch.into_handle())],
        Duration::from_secs(120),
    )?;
    run.expect("ctest", &["tcp", "udp", "dns", "refused", "accept"])
}

/// Dva procesy `ctest lock` nad jednim souborem (zamky drzi fsd).
fn ctest_lock(s: &Startup) -> TestResult {
    use super::std_app::start_std_app;

    let fs = devices().and_then(|d| d.fs.as_ref()).ok_or("sluzba fsd nebezi")?;
    match fs.create("/std-test", fsproto::kind::DIR) {
        Ok(()) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(fail("adresar /std-test", e)),
    }
    let dir = "/std-test/zamky";
    // Cisty adresar (znacky z minuleho behu pryc).
    for name in ["a-ready", "b-ready", "a-got", "lock.db"] {
        let _ = fs.remove(&alloc::format!("{dir}/{name}"));
    }
    match fs.create(dir, fsproto::kind::DIR) {
        Ok(()) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(fail("adresar zamku", e)),
    }
    let session = || -> Result<Vec<StartHandle>, String> {
        let own = fs.connect().map_err(|e| fail("relace fsd", e))?;
        Ok(vec![StartHandle::tagged(tag::FS, own.into_channel().into_handle())])
    };
    let a = start_std_app(s, "ctest", &["lock", "a", dir], &[], session()?)?;
    let b = start_std_app(s, "ctest", &["lock", "b", dir], &[], session()?)?;
    let rb = b.finish(Duration::from_secs(60))?;
    let ra = a.finish(Duration::from_secs(60))?;
    ra.expect("ctest", &["lock-a"])?;
    rb.expect("ctest", &["lock-b"])
}
