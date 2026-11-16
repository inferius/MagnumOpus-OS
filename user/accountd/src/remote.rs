//! Vzdaleny pristup (SSH, `docs/cs/ssh.md`): kanal `ssh` (jen sluzba
//! sshd) a vlastni operace uctu nad klici SSH.
//!
//! Podpis prihlaseni overuje accountd (`accounts::remote`), sshd jen
//! preda data. Po uspechu accountd pozada init o vzdalenou relaci
//! (`ToInit::OpenRemote`); init vrati listek, kterym si ji sshd vyzvedne.
//! Kazdy pokus jde do logu (logd) bez obsahu: kdo, odkud, vysledek.

use alloc::vec;
use alloc::vec::Vec;

use accountproto::ssh::{SshAuthReply, SshKey, TICKET, encode_keys};
use accountproto::{Request, ToInit, reply_err, reply_ok};
use accounts::remote::RemoteAuth;
use libos::{Channel, Error, Result, println};

use crate::service::{Service, Session, now};

/// Adresa do logu: jen tisknutelne ASCII, nejvys 64 znaku (prichazi od
/// sshd, ale nema co rozbit log).
fn clean(s: &str) -> alloc::string::String {
    s.chars().filter(|c| c.is_ascii_graphic()).take(64).collect()
}

impl Service {
    /// Kanal ssh.
    pub fn ssh(&mut self, req: Request) -> Vec<u8> {
        let now = now();
        match req {
            Request::SshProbe { user, addr, key } => {
                let ok = self.eng.ssh_probe(&self.remote, &user, &addr, &key, now);
                reply_ok(&[ok as u8])
            }
            Request::SshAuth { user, addr, key, data, sig } => {
                let (u, a) = (clean(&user), clean(&addr));
                let fp = ssh::keys::fingerprint(&key);
                let r = match self.eng.ssh_auth(&mut self.remote, &user, &addr, &key, &data, &sig, now) {
                    RemoteAuth::Ok { uid } => match self.open_remote(uid, &a) {
                        Ok((session, ticket)) => {
                            println!("accountd: ssh: {u} ({uid}) z {a} prihlasen klicem {fp}, relace {session}");
                            SshAuthReply::Ok { uid, session, ticket }
                        }
                        Err(Error::Locked) => {
                            println!("accountd: ssh: {u} ({uid}) z {a}: domov je zamceny (nejdriv prihlaseni u pocitace)");
                            SshAuthReply::Locked
                        }
                        Err(e) => return reply_err(e),
                    },
                    RemoteAuth::Denied { retry_after_ms } => {
                        println!("accountd: ssh: odmitnuto {u} z {a} (klic {fp}), dalsi pokus za {} s", retry_after_ms / 1000);
                        SshAuthReply::Denied { retry_after_ms }
                    }
                };
                reply_ok(&r.encode())
            }
            _ => reply_err(Error::AccessDenied),
        }
    }

    /// Vzdalena relace u initu. Sifrovany domov klic SSH neodemkne:
    /// projde jen, kdyz uz je odemceny mistni relaci tehoz uctu.
    fn open_remote(&mut self, uid: u64, peer: &str) -> Result<(u64, Vec<u8>)> {
        let a = self.eng.account(uid).ok_or(Error::NotFound)?;
        let (groups, home, encrypted) = (a.groups.clone(), a.home.clone(), a.crypt.is_some());
        if encrypted && !self.sessions.iter().any(|s| s.uid == uid) {
            return Err(Error::Locked);
        }
        let (ours, theirs) = Channel::create()?;
        let m = ToInit::OpenRemote { uid, groups, home, peer: alloc::string::String::from(peer) };
        let body = self.to_init(&m, vec![theirs.into_handle()])?;
        let mut r = wire::Reader::new(&body);
        let id = r.u64()?;
        let ticket = r.take(TICKET)?.to_vec();
        self.sessions.push(Session { id, uid, chan: ours, locked: false, closed: false });
        Ok((id, ticket))
    }

    /// Vlastni operace nad klici SSH a vzdalenym pristupem (kanal self).
    pub fn own_ssh(&mut self, uid: u64, req: Request) -> Result<Vec<u8>> {
        let now = now();
        match req {
            Request::SshKeyAdd { password, line } => {
                let (id, fp) = self.eng.ssh_key_add(uid, &password, &line, now)?;
                println!("accountd: ucet {uid}: pridan klic SSH {fp}");
                let mut w = wire::Writer::new();
                w.u32(id).str(&fp);
                Ok(w.finish())
            }
            Request::SshKeyList => {
                let list: Vec<SshKey> = self
                    .eng
                    .ssh_keys(uid)?
                    .into_iter()
                    .map(|k| SshKey { id: k.id, algo: k.algo, fingerprint: k.fingerprint, comment: k.comment, added: k.added })
                    .collect();
                Ok(encode_keys(&list))
            }
            Request::SshKeyRemove { id } => {
                self.eng.ssh_key_remove(uid, id)?;
                println!("accountd: ucet {uid}: odebran klic SSH {id}");
                Ok(Vec::new())
            }
            Request::SetRemote { password, on } => {
                self.eng.set_remote(uid, &password, on, now)?;
                println!("accountd: ucet {uid}: vzdaleny pristup {}", if on { "zapnut" } else { "vypnut" });
                Ok(Vec::new())
            }
            _ => Err(Error::AccessDenied),
        }
    }
}
