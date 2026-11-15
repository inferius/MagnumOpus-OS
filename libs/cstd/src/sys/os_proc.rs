//! Procesy pro `posix_spawn` a `waitpid` nad `osrt::process` (launchd,
//! stejna cesta jako `std::process::Command`).

use std::os::magnumopus::runtime as rt;

use rt::fd::Object;
use rt::process::{Spawn, StdioSpec};
use rt::stdio::{Inherit, Stream};

use super::{ChildIo, R, SpawnReq};
use crate::errno;

fn e(err: rt::Error) -> crate::c_int {
    errno::from_os(err as i32)
}

/// Spusteny potomek.
pub struct Child(rt::process::Child);

/// Proud potomka z popisu rodice: stdio (i jiny proud, napr. stderr jako
/// stdout po `dup2`) a roury; soubory a sockety blok parametru predat
/// neumi (`ENOTSUP`).
fn spec(io: ChildIo, target: Stream) -> R<StdioSpec> {
    let fd = match io {
        ChildIo::Null | ChildIo::Closed => return Ok(StdioSpec::Null),
        ChildIo::Parent(fd) => fd,
    };
    let d = rt::fd::get(fd).map_err(|_| errno::EBADF)?;
    match &d.object {
        Object::Stdio(s) if *s == target => Ok(StdioSpec::Inherit),
        Object::Stdio(s) => match rt::stdio::inherit(*s).map_err(e)? {
            Inherit::Channel(c) => Ok(StdioSpec::Channel(c)),
            Inherit::Null | Inherit::None => Ok(StdioSpec::Null),
        },
        Object::Pipe(p) => Ok(StdioSpec::Channel(p.try_clone().map_err(e)?.into_channel())),
        _ => Err(errno::ENOTSUP),
    }
}

/// Spusti potomka. Relaci site dostane, ma-li ji rodic (POSIX: potomek
/// dedi prostredi rodice); co smi, urci manifest potomka v launchd.
pub fn spawn_child(r: SpawnReq) -> R<Child> {
    let s = Spawn {
        program: r.program,
        args: r.args,
        env: r.env,
        cwd: r.cwd,
        stdin: spec(r.io[0], Stream::In)?,
        stdout: spec(r.io[1], Stream::Out)?,
        stderr: spec(r.io[2], Stream::Err)?,
        network: rt::net::available(),
    };
    match rt::process::spawn(s) {
        Ok((c, _)) => Ok(Child(c)),
        Err((err, _)) => Err(match err {
            // Proces bez kanalu ke launchd potomky spoustet nemuze.
            rt::Error::NotSupported => errno::ENOSYS,
            err => e(err),
        }),
    }
}

impl Child {
    /// Kod konce, kdyz uz skoncil.
    pub fn try_exit(&self) -> R<Option<i64>> {
        self.0.try_wait().map_err(e)
    }

    pub fn wait_exit(&self) -> R<i64> {
        self.0.wait().map_err(e)
    }

    pub fn kill(&self) -> R<()> {
        self.0.kill().map_err(e)
    }
}

/// Pocka na konec nektereho z potomku nejdele `timeout_ns`; `None` =
/// termin vyprsel.
pub fn wait_children(children: &[&Child], timeout_ns: Option<u64>) -> R<Option<usize>> {
    let inner: Vec<&rt::process::Child> = children.iter().map(|c| &c.0).collect();
    let deadline = timeout_ns.map(|t| super::monotonic_ns().saturating_add(t));
    match rt::process::wait_any(&inner, deadline) {
        Ok(i) => Ok(Some(i)),
        Err(rt::Error::TimedOut) => Ok(None),
        Err(err) => Err(e(err)),
    }
}
