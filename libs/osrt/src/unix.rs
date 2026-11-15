//! Sockety `AF_UNIX` (`SOCK_STREAM`) pro tabulku fd (T1, docs/cs/cstd.md,
//! kapitola Sockety AF_UNIX).
//!
//! Spojeni = dvojice kanalu. Kazdy `send` = zprava kanalu s ramcem:
//!
//! ```text
//! 0  priznaky u8   (EOF = druha strana zavrela zapis, `shutdown`)
//! 1  pocet fd u8   (`SCM_RIGHTS`)
//! 2  fd[]: druh u8, stav u32, delka dat u32, data (`abi::params::fd_kind`)
//!    data zpravy (zbytek)
//! ```
//!
//! Handly fd jsou ve zprave v poradi polozek s handlem (jako dedeni fd,
//! [`crate::inherit`]). Prijemce dostane fd prvnim `recv`, ktery zpravu
//! nacne, a `recv` nikdy neprekroci hranici zpravy s fd (jako Linux).
//!
//! Jmeno (`bind`, `connect` na cestu) je uzel socketu ve fsd
//! (`fsproto::v2::op::SOCKET_BIND`): posluchac preda fsd konec kanalu,
//! fsd do nej pri `connect` zapise prihlaseni klienta a konec spojeni.
//! Jmeno abstraktniho prostoru (Linux `\0jmeno`) neni.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;

use fsproto::v2::SocketPeer;
use libos::sync::Mutex;
use libos::{Channel, Handle};

use crate::fd::Description;
use crate::{Error, Result};

/// Priznaky ramce.
mod frame {
    /// Druha strana zavrela zapis (`shutdown(SHUT_WR)`).
    pub const EOF: u8 = 1;
}

/// Nejvic fd v jedne zprave (Linux `SCM_MAX_FD` je 253).
pub const MAX_FDS: usize = 32;

/// Prijata zprava, ze ktere se jeste cte.
#[derive(Default)]
struct Pending {
    data: Vec<u8>,
    at: usize,
    /// Fd zpravy, ktere jeste nikdo nevzal.
    fds: Vec<Arc<Description>>,
}

struct Conn {
    ch: Channel,
    rest: Mutex<Pending>,
    /// Prijat EOF od druhe strany (dalsi `recv` = 0).
    eof: core::sync::atomic::AtomicBool,
    peer: SocketPeer,
    /// Jmeno druhe strany (`getpeername`), u `connect` cesta.
    peer_path: Option<String>,
}

enum State {
    New,
    /// Po `bind`: spojeni prichazeji do `rx` (i pred `listen`).
    Bound { rx: Channel },
    Listening { rx: Channel },
    Connected(Conn),
}

/// Socket `AF_UNIX` v tabulce fd.
pub struct UnixSocket {
    state: Mutex<State>,
    /// Vlastni jmeno (`bind`, u prijateho spojeni jmeno posluchace).
    path: Mutex<Option<String>>,
}

/// Prijata data a fd z `recv_msg`.
pub struct Received {
    pub len: usize,
    pub fds: Vec<Arc<Description>>,
    /// Fd se nevesly (C `MSG_CTRUNC`); zavrou se.
    pub truncated: bool,
}

fn self_peer() -> SocketPeer {
    let user = crate::fs::user_id();
    SocketPeer { user, group: crate::fs::groups().first().copied().unwrap_or(user), pid: crate::process::self_id() }
}

impl UnixSocket {
    pub fn new() -> Self {
        Self { state: Mutex::new(State::New), path: Mutex::new(None) }
    }

    fn connected(ch: Channel, peer: SocketPeer, path: Option<String>, peer_path: Option<String>) -> Self {
        let conn = Conn { ch, rest: Mutex::new(Pending::default()), eof: Default::default(), peer, peer_path };
        Self { state: Mutex::new(State::Connected(conn)), path: Mutex::new(path) }
    }

    /// `socketpair`: dva spojene konce.
    pub fn pair() -> Result<(Self, Self)> {
        let (a, b) = Channel::create()?;
        Ok((Self::connected(a, self_peer(), None, None), Self::connected(b, self_peer(), None, None)))
    }

    /// `bind` na cestu (absolutni po `env::resolve`).
    pub fn bind(&self, path: &[u8]) -> Result<()> {
        let mut st = self.state.lock();
        if !matches!(*st, State::New) {
            return Err(Error::InvalidArgs);
        }
        let path = crate::env::resolve(path)?;
        let (rx, tx) = Channel::create()?;
        crate::fs::session()?.socket_bind(&path, tx)?;
        *st = State::Bound { rx };
        *self.path.lock() = Some(path);
        Ok(())
    }

    pub fn listen(&self) -> Result<()> {
        let mut st = self.state.lock();
        *st = match core::mem::replace(&mut *st, State::New) {
            State::Bound { rx } | State::Listening { rx } => State::Listening { rx },
            // Linux: listen bez bind = automaticke abstraktni jmeno (neni).
            other => {
                *st = other;
                return Err(Error::InvalidArgs);
            }
        };
        Ok(())
    }

    pub fn is_listening(&self) -> bool {
        matches!(*self.state.lock(), State::Listening { .. })
    }

    /// `connect` na cestu. Plna fronta posluchace: neblokujici =
    /// `ShouldWait`, jinak kratke cekani a znovu.
    pub fn connect(&self, path: &[u8], nonblock: bool) -> Result<()> {
        let mut st = self.state.lock();
        match *st {
            State::New => {}
            State::Connected(_) => return Err(Error::AlreadyExists),
            _ => return Err(Error::InvalidArgs),
        }
        let path = crate::env::resolve(path)?;
        let pid = crate::process::self_id();
        loop {
            let (mine, theirs) = Channel::create()?;
            match crate::fs::session()?.socket_connect(&path, pid, theirs) {
                Ok(()) => {
                    // Prihlaseni posluchace fsd nedava: stejny uzivatel
                    // jako relace, ktera uzel vytvorila, overit nejde.
                    *st = State::Connected(Conn {
                        ch: mine,
                        rest: Mutex::new(Pending::default()),
                        eof: Default::default(),
                        peer: SocketPeer::default(),
                        peer_path: Some(path),
                    });
                    return Ok(());
                }
                Err(Error::ShouldWait) if !nonblock => libos::time::sleep(Duration::from_millis(1)),
                Err(e) => return Err(e),
            }
        }
    }

    /// `accept`: dalsi spojeni (neblokujici bez spojeni = `ShouldWait`).
    pub fn accept(&self, nonblock: bool) -> Result<UnixSocket> {
        let msg = {
            let st = self.state.lock();
            let State::Listening { rx } = &*st else { return Err(Error::InvalidArgs) };
            if nonblock { rx.try_read()? } else { rx.read(None)? }
        };
        let peer = SocketPeer::decode(&msg.data)?;
        let h = msg.handles.into_iter().next().ok_or(Error::Corrupted)?;
        Ok(Self::connected(Channel::from_handle(h), peer, self.path.lock().clone(), None))
    }

    /// Prihlaseni druhe strany (`SO_PEERCRED`).
    pub fn peer(&self) -> Result<SocketPeer> {
        match &*self.state.lock() {
            State::Connected(c) => Ok(c.peer),
            _ => Err(Error::BadState),
        }
    }

    /// Vlastni jmeno a jmeno druhe strany (`getsockname`/`getpeername`).
    pub fn local_path(&self) -> Option<String> {
        self.path.lock().clone()
    }

    pub fn peer_path(&self) -> Result<Option<String>> {
        match &*self.state.lock() {
            State::Connected(c) => Ok(c.peer_path.clone()),
            _ => Err(Error::BadState),
        }
    }

    /// Odesle `data` a fd (`sendmsg` s `SCM_RIGHTS`). Zavrena druha strana =
    /// `PeerClosed` (C `EPIPE`); plna fronta neblokujici = `ShouldWait`.
    pub fn send(&self, data: &[u8], fds: &[Arc<Description>], nonblock: bool) -> Result<usize> {
        if fds.len() > MAX_FDS {
            return Err(Error::InvalidArgs);
        }
        let st = self.state.lock();
        let State::Connected(c) = &*st else { return Err(Error::BadState) };
        // Fd jen v prvnim kusu; dalsi kusy (data nad limit zpravy) bez nich.
        let mut head = Vec::new();
        let mut handles = Vec::new();
        head.push(0);
        head.push(fds.len() as u8);
        for d in fds {
            let e = crate::inherit::export(0, d)?;
            head.push(e.entry.kind);
            head.extend_from_slice(&e.entry.status.to_le_bytes());
            head.extend_from_slice(&(e.entry.data.len() as u32).to_le_bytes());
            head.extend_from_slice(&e.entry.data);
            handles.extend(e.handle);
        }
        let room = abi::syscall::CHANNEL_MAX_DATA.saturating_sub(head.len()).max(1);
        let mut sent = 0;
        let mut first = true;
        loop {
            let chunk = &data[sent..data.len().min(sent + room)];
            let mut msg = if first { head.clone() } else { alloc::vec![0, 0] };
            msg.extend_from_slice(chunk);
            let hs = if first { core::mem::take(&mut handles) } else { Vec::new() };
            match write_retry(&c.ch, &msg, hs, nonblock && first) {
                Ok(()) => {}
                // Cast uz odesla: vratit kolik (jako kratky zapis).
                Err(Error::ShouldWait) if !first => return Ok(sent),
                Err(e) => return Err(e),
            }
            first = false;
            sent += chunk.len();
            if sent >= data.len() {
                return Ok(sent);
            }
        }
    }

    /// Prijme az `buf.len()` bajtu a fd zpravy (nejvic `max_fds`, zbytek
    /// se zavre). 0 = konec (druha strana zavrela nebo `shutdown`).
    pub fn recv(&self, buf: &mut [u8], max_fds: usize, nonblock: bool) -> Result<Received> {
        let st = self.state.lock();
        let State::Connected(c) = &*st else { return Err(Error::BadState) };
        let mut p = c.rest.lock();
        loop {
            if p.at < p.data.len() || !p.fds.is_empty() {
                break;
            }
            if c.eof.load(core::sync::atomic::Ordering::Acquire) {
                return Ok(Received { len: 0, fds: Vec::new(), truncated: false });
            }
            let msg = match if nonblock { c.ch.try_read() } else { c.ch.read(None) } {
                Ok(m) => m,
                Err(Error::PeerClosed) => return Ok(Received { len: 0, fds: Vec::new(), truncated: false }),
                Err(e) => return Err(e),
            };
            let (flags, data, fds) = parse(&msg.data, msg.handles)?;
            if flags & frame::EOF != 0 {
                c.eof.store(true, core::sync::atomic::Ordering::Release);
            }
            *p = Pending { data, at: 0, fds };
        }
        let n = buf.len().min(p.data.len() - p.at);
        buf[..n].copy_from_slice(&p.data[p.at..p.at + n]);
        p.at += n;
        let mut fds = core::mem::take(&mut p.fds);
        let truncated = fds.len() > max_fds;
        fds.truncate(max_fds);
        Ok(Received { len: n, fds, truncated })
    }

    /// `shutdown(SHUT_WR)`: druha strana docte data a dostane konec.
    pub fn shutdown_write(&self) -> Result<()> {
        let st = self.state.lock();
        let State::Connected(c) = &*st else { return Err(Error::BadState) };
        match write_retry(&c.ch, &[frame::EOF, 0], Vec::new(), false) {
            Ok(()) | Err(Error::PeerClosed) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Ceka se na data (`poll`): handle kanalu spojeni nebo posluchace
    /// a jestli jsou data uz prijata.
    pub fn wait_handle(&self) -> Option<(Handle, bool)> {
        match &*self.state.lock() {
            State::Connected(c) => {
                let p = c.rest.lock();
                let ready = p.at < p.data.len() || !p.fds.is_empty() || c.eof.load(core::sync::atomic::Ordering::Acquire);
                c.ch.handle().duplicate(None).ok().map(|h| (h, ready))
            }
            State::Listening { rx } => rx.handle().duplicate(None).ok().map(|h| (h, false)),
            _ => None,
        }
    }

    /// Handle a stav pro predani jinemu procesu (dedeni, `SCM_RIGHTS`):
    /// (stav u8, handle). Socket bez spojeni ani jmena predat nejde.
    pub(crate) fn export(&self) -> Result<(Vec<u8>, Handle)> {
        let st = self.state.lock();
        let mut data = Vec::new();
        let h = match &*st {
            State::Connected(c) => {
                data.push(STATE_CONNECTED);
                for v in [c.peer.user, c.peer.group] {
                    data.extend_from_slice(&v.to_le_bytes());
                }
                data.extend_from_slice(&c.peer.pid.to_le_bytes());
                put_path(&mut data, c.peer_path.as_deref());
                c.ch.handle().duplicate(None)?
            }
            State::Listening { rx } | State::Bound { rx } => {
                data.push(if matches!(*st, State::Listening { .. }) { STATE_LISTENING } else { STATE_BOUND });
                rx.handle().duplicate(None)?
            }
            State::New => return Err(Error::NotSupported),
        };
        put_path(&mut data, self.path.lock().as_deref());
        Ok((data, h))
    }

    /// Socket z [`UnixSocket::export`] jineho procesu.
    pub(crate) fn import(data: &[u8], h: Handle) -> Result<Self> {
        let (&state, mut rest) = data.split_first().ok_or(Error::Corrupted)?;
        let ch = Channel::from_handle(h);
        let s = match state {
            STATE_CONNECTED => {
                let u = |r: &[u8], at: usize| -> Result<u64> {
                    Ok(u64::from_le_bytes(r.get(at..at + 8).ok_or(Error::Corrupted)?.try_into().map_err(|_| Error::Corrupted)?))
                };
                let user = u(rest, 0)?;
                let group = u(rest, 8)?;
                let pid = u32::from_le_bytes(rest.get(16..20).ok_or(Error::Corrupted)?.try_into().map_err(|_| Error::Corrupted)?);
                rest = &rest[20..];
                let peer_path = take_path(&mut rest)?;
                Self::connected(ch, SocketPeer { user, group, pid }, None, peer_path)
            }
            STATE_LISTENING => Self { state: Mutex::new(State::Listening { rx: ch }), path: Mutex::new(None) },
            STATE_BOUND => Self { state: Mutex::new(State::Bound { rx: ch }), path: Mutex::new(None) },
            _ => return Err(Error::Corrupted),
        };
        *s.path.lock() = take_path(&mut rest)?;
        Ok(s)
    }
}

impl Default for UnixSocket {
    fn default() -> Self {
        Self::new()
    }
}

const STATE_CONNECTED: u8 = 1;
const STATE_LISTENING: u8 = 2;
const STATE_BOUND: u8 = 3;

fn put_path(out: &mut Vec<u8>, p: Option<&str>) {
    let p = p.unwrap_or("");
    out.extend_from_slice(&(p.len() as u32).to_le_bytes());
    out.extend_from_slice(p.as_bytes());
}

fn take_path(r: &mut &[u8]) -> Result<Option<String>> {
    let n = u32::from_le_bytes(r.get(..4).ok_or(Error::Corrupted)?.try_into().map_err(|_| Error::Corrupted)?) as usize;
    let s = r.get(4..4 + n).ok_or(Error::Corrupted)?;
    let out = (n > 0).then(|| String::from_utf8(s.to_vec())).transpose().map_err(|_| Error::Corrupted)?;
    *r = &r[4 + n..];
    Ok(out)
}

/// Zapis zpravy; plna fronta: neblokujici `ShouldWait`, jinak kratke
/// cekani a znovu (kanal nema signal "lze zapisovat").
fn write_retry(ch: &Channel, msg: &[u8], handles: Vec<Handle>, nonblock: bool) -> Result<()> {
    let mut handles = Some(handles);
    loop {
        // Handly jdou pri neuspechu zpet jen pri ShouldWait (kanal je
        // nevzal); proto se pri opakovani posila duplikat.
        let hs = handles.take().unwrap_or_default();
        let dup: Vec<Handle> = hs.iter().filter_map(|h| h.duplicate(None).ok()).collect();
        match ch.write(msg, dup) {
            Ok(()) => return Ok(()),
            Err(Error::ShouldWait) if !nonblock => {
                handles = Some(hs);
                libos::time::sleep(Duration::from_millis(1));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Rozlozi ramec: (priznaky, data, fd).
fn parse(b: &[u8], handles: Vec<Handle>) -> Result<(u8, Vec<u8>, Vec<Arc<Description>>)> {
    let (&flags, b) = b.split_first().ok_or(Error::Corrupted)?;
    let (&n, mut b) = b.split_first().ok_or(Error::Corrupted)?;
    let mut handles = handles.into_iter();
    let mut fds = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let kind = *b.first().ok_or(Error::Corrupted)?;
        let status = u32::from_le_bytes(b.get(1..5).ok_or(Error::Corrupted)?.try_into().map_err(|_| Error::Corrupted)?);
        let len = u32::from_le_bytes(b.get(5..9).ok_or(Error::Corrupted)?.try_into().map_err(|_| Error::Corrupted)?) as usize;
        let data = b.get(9..9 + len).ok_or(Error::Corrupted)?;
        b = &b[9 + len..];
        let h = if abi::params::fd_kind::has_handle(kind) { handles.next() } else { None };
        let e = libos::params::Fd { fd: 0, kind, status, data: data.to_vec() };
        // Fd, ktery v tomto procesu otevrit nejde (soubor mimo relaci),
        // se vynecha (prijemce dostane mene fd, jako Linux pri chybe).
        if let Ok(obj) = crate::inherit::object(&e, h) {
            fds.push(Description::new(obj, status));
        }
    }
    Ok((flags, b.to_vec(), fds))
}
