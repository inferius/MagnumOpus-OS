//! `std::process` v aplikaci se std (docs/cs/rust-std.md, faze 2):
//! `std-proc` spousti sam sebe pres launchd (registr prikazu, initfs,
//! absolutni cesta ve FS), overi roury, stdio, kody, kill a predani site.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use libos::abi::params::tag;
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Error, Startup};
use netproto::{Policy, Rule, Target, proto, rights};

use super::net::{HOST, configured, host, net, ports};
use super::std_app::run_std_app;
use super::{Test, TestResult, devices, fail};
use crate::check;

pub const TESTS: &[Test] = &[("std-proc: Command, roury, stdio, kody, sit, registr", std_proc)];

fn std_proc(s: &Startup) -> TestResult {
    let Some(p) = ports(s, "std-proc")? else {
        return Ok(());
    };
    let fs = devices().and_then(|d| d.fs.as_ref()).ok_or("sluzba fsd nebezi")?;
    // Kopie programu ve FS (spusteni podle absolutni cesty).
    for dir in ["/std-test", "/std-test/bin"] {
        match fs.create(dir, fsproto::kind::DIR) {
            Ok(()) | Err(Error::AlreadyExists) => {}
            Err(e) => return Err(fail(dir, e)),
        }
    }
    let image = Initfs::open(s).map_err(|e| fail("initfs", e))?.find("std-proc").ok_or("std-proc neni v initfs")?;
    let _ = fs.remove("/std-test/bin/std-proc");
    fs.create("/std-test/bin/std-proc", fsproto::kind::FILE).map_err(|e| fail("kopie programu", e))?;
    fs.write("/std-test/bin/std-proc", 0, image).map_err(|e| fail("zapis kopie", e))?;

    let root = net()?;
    configured(root)?;
    let target = Target::Net { addr: HOST, prefix: 32 };
    let session = root
        .restrict(rights::CLIENT, &Policy::only(vec![Rule::port(proto::TCP, target, p.echo)]))
        .map_err(|e| fail("restrict", e))?;
    let launch = crate::launch::client().map_err(|e| fail("kanal ke launchd", e))?;
    let own_fs = fs.connect().map_err(|e| fail("relace fsd", e))?;
    let env = [format!("ECHO={}", host(p.echo))];
    let env: Vec<&str> = env.iter().map(|e| e.as_str()).collect();
    let run = run_std_app(
        s,
        "std-proc",
        &[],
        &env,
        vec![
            StartHandle::tagged(tag::FS, own_fs.into_channel().into_handle()),
            StartHandle::tagged(tag::NET, session.into_channel().map_err(|e| format!("predani site: {e:?}"))?.into_handle()),
            StartHandle::tagged(tag::LAUNCH, launch.into_handle()),
        ],
        Duration::from_secs(180),
    )?;
    run.expect(
        "std-proc",
        &[
            "output", "exit_code", "stdin_pipe", "env_cwd", "null_and_inherit", "kill", "network", "not_found",
            "absolute_path", "registry", "io_pipe",
        ],
    )?;
    // Zdedeny stdout potomka prisel do stdout rodice.
    check!(run.stdout.contains("zdedeny vystup\n"), "zdedeny stdout potomka chybi");
    check!(!run.stdout.contains("zahozeno"), "Stdio::null nezahodil vystup");
    Ok(())
}
