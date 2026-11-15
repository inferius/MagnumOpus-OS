//! Uzly socketu `AF_UNIX` (fsproto v2 `SOCKET_BIND`, `SOCKET_CONNECT`).
//!
//! Uzel je objekt FS druhu `Socket` (jmeno, vlastnik, prava); spojeni FS
//! neuklada. Posluchac pri `bind` preda konec kanalu, do ktereho fsd
//! zapisuje prichozi spojeni: zprava = prihlaseni klienta (uzivatel,
//! skupina, cislo procesu) a handle konce spojeni. Mapa uzel -> posluchac
//! je jen v pameti: konec posluchace (zavreny kanal) nebo restart fsd =
//! `CONNECTION_REFUSED`, uzel zustane (jako Unix; smaze ho `unlink`).

use alloc::collections::BTreeMap;

use fsproto::Reader;
use fsproto::v2::{access as v2access, op, share};
use libos::{Channel, Error, Handle, Result};

use crate::serve::Service;
use crate::serve_v2::{Reply, Session};

/// Posluchaci podle (dataset, oid) uzlu.
#[derive(Default)]
pub struct Listeners {
    map: BTreeMap<(u64, u64), Channel>,
}

impl Listeners {
    /// Zahodi posluchace, kteri skoncili (kanal bez druhe strany).
    fn prune(&mut self) {
        self.map.retain(|_, ch| libos::wait_one(ch.handle(), libos::Signals::PEER_CLOSED, Some(0)).is_err());
    }
}

impl Service {
    /// Operace socketu; `None` = neni jeho. `handles` = handly pozadavku.
    pub fn handle_sock(&mut self, sess: &mut Session, opcode: u32, r: &mut Reader, handles: &mut alloc::vec::Vec<Handle>) -> Option<Result<Reply>> {
        match opcode {
            op::SOCKET_BIND => Some(self.sock_bind(sess, r, handles)),
            op::SOCKET_CONNECT => Some(self.sock_connect(sess, r, handles)),
            op::SET_MODE => Some((|| {
                let path = sess.map(&r.str()?, true)?;
                let (mode, widen) = (r.u16()?, r.u8()? != 0);
                self.fs.as_mut().ok_or(Error::IoError)?.set_mode(&path, mode, widen)?;
                Ok(Reply::body(fsproto::Writer::new()))
            })()),
            op::WHOAMI => {
                let mut w = fsproto::Writer::new();
                match &sess.identity {
                    Some(p) => {
                        w.u64(p.user).u32(p.groups.len() as u32);
                        for g in &p.groups {
                            w.u64(*g);
                        }
                    }
                    None => {
                        w.u64(0).u32(0);
                    }
                }
                Some(Ok(Reply::body(w)))
            }
            _ => None,
        }
    }

    /// (dataset, oid) uzlu socketu na ceste `path` s pristupem `access`.
    fn socket_node(&mut self, path: &str, access: u8) -> Result<(u64, u64)> {
        let fs = self.fs.as_mut().ok_or(Error::IoError)?;
        let h = fs.open(path, access, share::ALL)?;
        let st = fs.stat_h(h);
        let rev = fs.revision_h(h);
        let _ = fs.close(h);
        if st?.kind != fs::Kind::Socket {
            return Err(Error::ConnectionRefused);
        }
        let rev = rev?;
        Ok((rev.dataset, rev.oid))
    }

    /// `SOCKET_BIND`: cesta str + handle kanalu posluchace -> vytvori uzel.
    /// Existujici jmeno = `ALREADY_EXISTS` (C `EADDRINUSE`).
    fn sock_bind(&mut self, sess: &mut Session, r: &mut Reader, handles: &mut alloc::vec::Vec<Handle>) -> Result<Reply> {
        let raw = r.str()?;
        let listener = Channel::from_handle(handles.pop().ok_or(Error::InvalidArgs)?);
        let path = sess.map(&raw, true)?;
        self.fs.as_mut().ok_or(Error::IoError)?.create(&path, fs::Kind::Socket)?;
        let key = self.socket_node(&path, v2access::READ)?;
        self.listeners.prune();
        self.listeners.map.insert(key, listener);
        Ok(Reply::body(fsproto::Writer::new()))
    }

    /// `SOCKET_CONNECT`: cesta str, cislo procesu u32 + handle konce
    /// spojeni. Chce pravo zapisu na uzel. Plna fronta posluchace =
    /// `SHOULD_WAIT` (klient zkusi znovu).
    fn sock_connect(&mut self, sess: &mut Session, r: &mut Reader, handles: &mut alloc::vec::Vec<Handle>) -> Result<Reply> {
        let raw = r.str()?;
        let pid = r.u32()?;
        let end = handles.pop().ok_or(Error::InvalidArgs)?;
        let path = sess.map(&raw, false)?;
        let key = self.socket_node(&path, v2access::WRITE)?;
        let Some(listener) = self.listeners.map.get(&key) else { return Err(Error::ConnectionRefused) };
        let (user, group) = match &sess.identity {
            Some(p) => (p.user, p.groups.first().copied().unwrap_or(p.user)),
            None => (0, 0),
        };
        let mut w = fsproto::Writer::new();
        w.u64(user).u64(group).u32(pid);
        match listener.write(&w.finish(), alloc::vec![end]) {
            Ok(()) => Ok(Reply::body(fsproto::Writer::new())),
            Err(Error::PeerClosed) => {
                self.listeners.map.remove(&key);
                Err(Error::ConnectionRefused)
            }
            Err(e) => Err(e),
        }
    }
}
