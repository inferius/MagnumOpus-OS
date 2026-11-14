//! Spousteci sluzba `launchd` (viz `user/launchd`, `libs/launchproto`,
//! `docs/cs/rust-std.md`): init ji spusti s obrazem initfs, drzi jeji
//! ridici kanal a razi klientske kanaly pro aplikace (`std::process`).

use alloc::vec::Vec;
use core::time::Duration;

use libos::abi::bootstrap::kind;
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Process, Result, Startup, time};

pub struct Launch {
    pub process: Process,
    ctl: Channel,
}

/// Bezici launchd.
pub static LAUNCH: spin::Mutex<Option<Launch>> = spin::Mutex::new(None);

/// Spusti launchd (z initfs).
pub fn start(startup: &Startup) -> Result<()> {
    let fs = Initfs::open(startup)?;
    let image = fs.find("launchd").ok_or(Error::NotFound)?;
    let vmo = startup.initfs.as_ref().ok_or(Error::NotFound)?;
    let handles = alloc::vec![StartHandle::new(kind::INITFS, 0, vmo.handle().duplicate(None)?)];
    // Spoustec aplikaci: relace s libd (programy se zavadecem).
    let opts = libos::spawn::Options {
        qos: libos::sched::service_default("launchd"),
        libd: true,
        ..Default::default()
    };
    let (process, ctl) = libos::spawn::spawn_with(&libos::job::current()?, "launchd", image, handles, opts)?;
    *LAUNCH.lock() = Some(Launch { process, ctl });
    Ok(())
}

/// Novy klientsky kanal k launchd (aplikace ho dostane se stitkem
/// `abi::params::tag::LAUNCH`).
pub fn client() -> Result<Channel> {
    let guard = LAUNCH.lock();
    let l = guard.as_ref().ok_or(Error::ServiceUnavailable)?;
    l.ctl.write(b"client", Vec::new())?;
    let m = l.ctl.read(Some(time::deadline_after(Duration::from_secs(5))))?;
    if m.data != b"ok" {
        return Err(Error::Corrupted);
    }
    m.handles.into_iter().next().map(Channel::from_handle).ok_or(Error::Corrupted)
}
