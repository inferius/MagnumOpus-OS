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
/// stdout po `dup2`), roury a soubory (proud z fsd, `OPEN_STREAM`, od
/// aktualni pozice; potomek ma vlastni pozici). Sockety a adresare
/// `ENOTSUP`.
fn spec(io: ChildIo, target: Stream) -> R<StdioSpec> {
    let fd = match io {
        ChildIo::Null | ChildIo::Closed => return Ok(StdioSpec::Null),
        ChildIo::Open(_) => return Err(errno::EINVAL),
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
        Object::Null => Ok(StdioSpec::Null),
        Object::File(f) => match f.open_stream(target != Stream::In) {
            Ok(ch) => Ok(StdioSpec::Channel(ch)),
            // Handle bez prava pro smer proudu (soubor jen pro cteni na
            // stdout): jako zapis do takoveho fd.
            Err(rt::Error::AccessDenied) => Err(errno::EBADF),
            Err(err) => Err(e(err)),
        },
        _ => Err(errno::ENOTSUP),
    }
}

/// Zdedene fd nad 2 (`osrt::inherit`). Druh, ktery zdedit nejde, se
/// u zdedeneho fd preskoci; u fd z akce souboru je to `ENOTSUP`.
fn inherited(fds: &[(crate::c_int, ChildIo, bool)]) -> R<Vec<rt::inherit::Export>> {
    let mut out = Vec::new();
    for &(child, io, explicit) in fds {
        let d = match io {
            ChildIo::Parent(fd) => rt::fd::get(fd).map_err(|_| errno::EBADF)?,
            // addopen(n, "/dev/null") nad 2.
            ChildIo::Null => rt::fd::Description::new(Object::Null, crate::sys::status::RDWR),
            _ => return Err(errno::ENOTSUP),
        };
        match rt::inherit::export(child, &d) {
            Ok(x) => out.push(x),
            Err(rt::Error::NotSupported) if !explicit => {}
            Err(err) => return Err(e(err)),
        }
    }
    if out.len() > abi::params::MAX_FDS {
        return Err(errno::EMFILE);
    }
    Ok(out)
}

/// Spusti potomka. Relaci site dostane, ma-li ji rodic (POSIX: potomek
/// dedi prostredi rodice); co smi, urci manifest potomka v launchd.
pub fn spawn_child(r: SpawnReq) -> R<Child> {
    // Predani socketu AF_UNIX s kanalem ve fronte muze narazit na BAD_STATE
    // (druha strana mezitim poslala kanal): export se zopakuje
    // (`osrt::unix`, vyctena fronta jde s nim).
    let mut tries = 0;
    loop {
        let fds = inherited(&r.fds)?;
        let s = Spawn {
            program: r.program.clone(),
            args: r.args.clone(),
            env: r.env.clone(),
            cwd: r.cwd.clone(),
            stdin: spec(r.io[0], Stream::In)?,
            stdout: spec(r.io[1], Stream::Out)?,
            stderr: spec(r.io[2], Stream::Err)?,
            network: rt::net::available(),
        };
        match rt::process::spawn_with_fds(s, fds) {
            Ok((c, _)) => return Ok(Child(c)),
            Err((rt::Error::BadState, _)) if !r.fds.is_empty() && tries < 16 => tries += 1,
            Err((err, _)) => {
                return Err(match err {
                    // Proces bez kanalu ke launchd potomky spoustet nemuze.
                    rt::Error::NotSupported => errno::ENOSYS,
                    err => e(err),
                });
            }
        }
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
