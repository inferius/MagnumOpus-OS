//! Procesy pro `std::process::Command` pres spousteci sluzbu launchd
//! (`libs/launchproto`, docs/cs/rust-std.md, kapitola Procesy).
//!
//! Potomek dostane jen to, co mu proces preda: podrizeny job vlastniho
//! jobu, novou relaci FS (soubory se dedi), relaci site jen na vyslovne
//! pani (`network`), stdio podle `Stdio`. launchd navic uplatni manifest
//! potomka (limity, zuzeni site).

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use abi::Error;
use launchproto::{Launch, Role, op, parse_reply};
use libos::sync::Mutex;
use libos::{Channel, Handle, Process};

use crate::pipe::Pipe;
use crate::start::Init;
use crate::stdio::{Inherit, Stream};

/// Chyba s popisem (napr. ktery svazek pripojit).
pub type Fail = (Error, String);

fn fail(e: Error) -> Fail {
    (e, String::new())
}

static LAUNCH: Init<Mutex<Channel>> = Init::new();
static NEXT_ID: AtomicU32 = AtomicU32::new(1);

/// # Safety
///
/// Jen pri startu (viz `start::Init`).
pub(crate) unsafe fn init(channel: Option<Channel>) {
    if let Some(c) = channel {
        // SAFETY: predano volajicim.
        unsafe { LAUNCH.set(Mutex::new(c)) };
    }
}

/// Jak predat proud potomkovi.
pub enum StdioSpec {
    /// Stejny proud jako rodic.
    Inherit,
    /// Zahodit (vystup) nebo prazdny vstup.
    Null,
    /// Nova roura; rodic dostane svuj konec.
    Pipe,
    /// Konkretni kanal proudu (napr. konec roury z `io::pipe`).
    Channel(Channel),
}

/// Popis spusteni.
pub struct Spawn {
    pub program: String,
    /// Argumenty vcetne nulteho (jmeno programu).
    pub args: Vec<Vec<u8>>,
    /// Kompletni prostredi potomka `JMENO=HODNOTA`.
    pub env: Vec<Vec<u8>>,
    pub cwd: Vec<u8>,
    pub stdin: StdioSpec,
    pub stdout: StdioSpec,
    pub stderr: StdioSpec,
    /// Predat potomkovi relaci site (`CommandExt::network`).
    pub network: bool,
}

/// Spusteny potomek.
pub struct Child {
    process: Process,
    /// Kanal k potomkovi (bootstrap rodice); drzi se, dokud potomek zije.
    _control: Channel,
}

/// Konce rour rodice.
#[derive(Default)]
pub struct Pipes {
    pub stdin: Option<Pipe>,
    pub stdout: Option<Pipe>,
    pub stderr: Option<Pipe>,
}

/// Ma proces kanal ke launchd?
pub fn available() -> bool {
    LAUNCH.get().is_some()
}

/// Novy kanal ke launchd (pro proces, ktery ho preda dal sam).
pub fn launch_channel() -> Result<Channel, Error> {
    let ch = LAUNCH.get().ok_or(Error::NotSupported)?.lock();
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let (data, handles) = call(&ch, &launchproto::simple(op::CONNECT, id, None), Vec::new()).map_err(|(e, _)| e)?;
    let _ = data;
    handles.into_iter().next().map(Channel::from_handle).ok_or(Error::Corrupted)
}

fn call(ch: &Channel, req: &[u8], handles: Vec<Handle>) -> Result<(Vec<u8>, Vec<Handle>), Fail> {
    ch.write(req, handles).map_err(fail)?;
    let msg = ch.read(None).map_err(|e| match e {
        Error::PeerClosed => (Error::ServiceUnavailable, String::from("spousteci sluzba launchd nebezi")),
        e => fail(e),
    })?;
    let (_, r) = parse_reply(&msg.data).map_err(fail)?;
    r.map(|data| (data, msg.handles))
}

/// Kanal proudu pro potomka a pripadny konec roury pro rodice.
fn stdio_for(spec: StdioSpec, s: Stream, flags: &mut u32) -> Result<(Option<Channel>, Option<Pipe>), Fail> {
    let null = match s {
        Stream::Out => abi::params::flags::STDOUT_NULL,
        Stream::Err => abi::params::flags::STDERR_NULL,
        Stream::In => 0,
    };
    Ok(match spec {
        StdioSpec::Inherit => match crate::stdio::inherit(s).map_err(fail)? {
            Inherit::Channel(c) => (Some(c), None),
            Inherit::Null => {
                *flags |= null;
                (None, None)
            }
            Inherit::None => (None, None),
        },
        StdioSpec::Null => {
            *flags |= null;
            (None, None)
        }
        StdioSpec::Pipe => {
            let (mine, theirs) = Channel::create().map_err(fail)?;
            (Some(theirs), Some(Pipe::from_channel(mine)))
        }
        StdioSpec::Channel(c) => (Some(c), None),
    })
}

/// Spusti program pres launchd.
pub fn spawn(s: Spawn) -> Result<(Child, Pipes), Fail> {
    spawn_with_fds(s, Vec::new())
}

/// Jako [`spawn`] se zdedenymi fd ([`crate::inherit::export`], C
/// `posix_spawn`); nejvic `abi::params::MAX_FDS`.
pub fn spawn_with_fds(s: Spawn, fds: Vec<crate::inherit::Export>) -> Result<(Child, Pipes), Fail> {
    if fds.len() > abi::params::MAX_FDS {
        return Err((Error::NoResources, String::from("prilis mnoho zdedenych fd")));
    }
    let launch = LAUNCH.get().ok_or((Error::NotSupported, String::from("proces nema kanal ke spousteci sluzbe")))?;
    let mut flags = 0;
    let (stdin, p_in) = stdio_for(s.stdin, Stream::In, &mut flags)?;
    let (stdout, p_out) = stdio_for(s.stdout, Stream::Out, &mut flags)?;
    let (stderr, p_err) = stdio_for(s.stderr, Stream::Err, &mut flags)?;
    let mut fd_entries = Vec::with_capacity(fds.len());
    let mut fd_handles = Vec::new();
    for e in fds {
        fd_entries.push(e.entry);
        fd_handles.extend(e.handle);
    }
    let params = libos::params::Params { cwd: s.cwd, args: s.args, env: s.env, flags, fds: fd_entries }.encode().map_err(fail)?;

    let mut roles = Vec::new();
    let mut handles = Vec::new();
    let job = libos::job::current().map_err(fail)?;
    roles.push(Role::Job);
    handles.push(job.duplicate(None).map_err(fail)?.into_handle());
    // Soubory se dedi: nova relace se stejnou identitou a pripojenymi
    // cestami (vlastni handly, nezavisla na relaci rodice).
    if let Some(fs) = crate::fs::new_session() {
        roles.push(Role::Fs);
        handles.push(fs.into_handle());
    }
    if s.network {
        let net = crate::net::session_channel().ok_or((Error::AccessDenied, String::from("proces nema relaci site")))?;
        roles.push(Role::Net);
        handles.push(net.into_handle());
    }
    for (role, ch) in [(Role::Stdin, stdin), (Role::Stdout, stdout), (Role::Stderr, stderr)] {
        if let Some(ch) = ch {
            roles.push(role);
            handles.push(ch.into_handle());
        }
    }
    for h in fd_handles {
        roles.push(Role::Fd);
        handles.push(h);
    }
    let req = Launch { program: s.program, flags: 0, roles, params }.encode(NEXT_ID.fetch_add(1, Ordering::Relaxed)).map_err(fail)?;
    let (_, mut hs) = call(&launch.lock(), &req, handles)?;
    if hs.len() != 2 {
        return Err(fail(Error::Corrupted));
    }
    let control = Channel::from_handle(hs.pop().unwrap());
    let process = Process::from_handle(hs.pop().unwrap());
    Ok((Child { process, _control: control }, Pipes { stdin: p_in, stdout: p_out, stderr: p_err }))
}

impl Child {
    /// Pocka na konec; vraci navratovy kod.
    pub fn wait(&self) -> Result<i64, Error> {
        self.process.wait_exit(None)
    }

    /// Kod, kdyz uz skoncil (`None` = bezi).
    pub fn try_wait(&self) -> Result<Option<i64>, Error> {
        match libos::sys::process_exit_code(self.process.handle().raw()) {
            Ok(c) => Ok(Some(c)),
            Err(Error::BadState) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn kill(&self) -> Result<(), Error> {
        libos::sys::process_kill(self.process.handle().raw())
    }

    /// Cislo pro `Child::id` (handle procesu v tomto procesu).
    pub fn id(&self) -> u32 {
        self.process.handle().raw()
    }
}

/// Pocka, az skonci aspon jeden z potomku (C `waitpid(-1)`), nejdele do
/// `deadline` (ns od startu, `None` = navzdy); vraci index skonceneho.
/// Nejvys 64 potomku najednou (`OBJECT_WAIT_MANY`).
pub fn wait_any(children: &[&Child], deadline: Option<u64>) -> Result<usize, Error> {
    let items: Vec<(&libos::Handle, libos::Signals)> =
        children.iter().map(|c| (c.process.handle(), libos::Signals::TERMINATED)).collect();
    libos::wait_many(&items, deadline).map(|(i, _)| i)
}

/// Identifikator vlastniho procesu pro `std::process::id`: nahodne cislo
/// zvolene pri startu (jadro zatim cislo procesu procesu nesdeluje).
pub fn self_id() -> u32 {
    static ID: AtomicU32 = AtomicU32::new(0);
    let v = ID.load(Ordering::Relaxed);
    if v != 0 {
        return v;
    }
    let mut b = [0u8; 4];
    crate::fill_random(&mut b);
    // Rozsah jako drive cstd `getpid`: kladne, s mistem pro cisla potomku
    // (`pid_t` potomku cisluje cstd od `getpid() + 1`).
    let v = (u32::from_le_bytes(b) % 0x3fff_0000) + 2;
    match ID.compare_exchange(0, v, Ordering::Relaxed, Ordering::Relaxed) {
        Ok(_) => v,
        Err(old) => old,
    }
}
