//! Operace identity relaci a opravneni (protokol v2, op 46-53 a 94).
//!
//! Ridici relace (hlavni kanal ze startovni zpravy, ma ho init) je
//! system: vidi cely FS, zaklada relace uzivatelu (`CONNECT_AS`) a meni
//! jejich pripojene cesty (`GRANT`, `REVOKE`). Ostatni relace maji identitu
//! od toho, kdo je zalozil, a vidi jen sve pripojene cesty.

use alloc::string::String;
use alloc::vec::Vec;

use fsproto::perm::{Acl, AclView, Mount, Permissions, get_opt_acl, op, put_opt_acl};
use fsproto::{Reader, Writer};
use libos::{Channel, Error, Result};

use crate::serve::Service;
use crate::serve_v2::{Reply, Session, SessionOp};

/// Nejvic skupin uzivatele a pripojenych cest relace.
const MAX_GROUPS: usize = 64;
const MAX_MOUNTS: usize = 64;

fn to_fs_acl(a: Acl) -> fs::Acl {
    fs::Acl {
        flags: a.flags,
        grants: a.grants.into_iter().map(|g| fs::Grant { who: g.who, rights: g.rights, flags: g.flags }).collect(),
    }
}

fn from_fs_acl(a: fs::Acl) -> Acl {
    Acl {
        flags: a.flags,
        grants: a
            .grants
            .into_iter()
            .map(|g| fsproto::perm::Grant { who: g.who, rights: g.rights, flags: g.flags })
            .collect(),
    }
}

fn get_mount(r: &mut Reader) -> Result<Mount> {
    let m = Mount::get(r)?;
    fsproto::perm::check_path(&m.name)?;
    fsproto::perm::check_path(&m.target)?;
    Ok(m)
}

impl Service {
    pub(crate) fn perm_op(&mut self, sess: &mut Session, opcode: u32, r: &mut Reader) -> Result<Reply> {
        let mut w = Writer::new();
        match opcode {
            op::CONNECT_AS | op::GRANT | op::REVOKE | op::SET_OWNER if !sess.control => {
                return Err(Error::AccessDenied);
            }
            op::CONNECT_AS => {
                let user = r.u64()?;
                if user == fs::SYSTEM {
                    // Systemovou relaci ma jen ridici kanal.
                    return Err(Error::InvalidArgs);
                }
                let n = r.u32()? as usize;
                if n > MAX_GROUPS {
                    return Err(Error::InvalidArgs);
                }
                let groups = (0..n).map(|_| r.u64()).collect::<Result<Vec<_>>>()?;
                let n = r.u32()? as usize;
                if n > MAX_MOUNTS {
                    return Err(Error::InvalidArgs);
                }
                let mounts = (0..n).map(|_| get_mount(r)).collect::<Result<Vec<_>>>()?;
                let (ours, theirs) = Channel::create()?;
                self.session_counter += 1;
                let id = self.session_counter;
                let s = Session::new(ours, id, Some(fs::Principal::user(user, &groups)), mounts);
                w.u64(id);
                return Ok(Reply {
                    body: w,
                    handles: alloc::vec![theirs.into_handle()],
                    new_session: Some(s),
                    session_ops: Vec::new(),
                    watch: None,
                });
            }
            op::GRANT => {
                let id = r.u64()?;
                let m = get_mount(r)?;
                let mut reply = Reply::body(w);
                reply.session_ops.push(SessionOp::Grant(id, m));
                return Ok(reply);
            }
            op::REVOKE => {
                let id = r.u64()?;
                let name: String = r.str()?;
                let mut reply = Reply::body(w);
                reply.session_ops.push(SessionOp::Revoke(id, name));
                return Ok(reply);
            }
            _ => {}
        }
        let fs = self.fs.as_mut().ok_or(Error::IoError)?;
        match opcode {
            op::PERMISSIONS => {
                let path = sess.map(&r.str()?, false)?;
                let p = fs.permissions(&path)?;
                w.u64(p.owner).u8(p.rights);
                put_opt_acl(&mut w, p.acl.map(from_fs_acl).as_ref());
            }
            op::ACL_VIEW => {
                let path = sess.map(&r.str()?, false)?;
                let v = fs.acl_view(&path)?;
                let inherited = v
                    .inherited
                    .into_iter()
                    .map(|g| fsproto::perm::InheritedGrant { who: g.who, rights: g.rights, from: sess.unmap(&g.from) })
                    .collect();
                let perms = Permissions { owner: v.perms.owner, rights: v.perms.rights, acl: v.perms.acl.map(from_fs_acl) };
                AclView { perms, inherited }.put(&mut w);
            }
            op::SET_ACL => {
                let path = sess.map(&r.str()?, true)?;
                let acl = get_opt_acl(r)?.map(to_fs_acl);
                fs.set_acl(&path, acl)?;
            }
            op::SET_OWNER => {
                let path = sess.map(&r.str()?, true)?;
                fs.set_owner(&path, r.u64()?)?;
            }
            op::SET_NOEXEC => {
                let path = sess.map(&r.str()?, true)?;
                fs.set_noexec(&path, r.u8()? != 0)?;
            }
            op::NOEXEC => {
                let path = sess.map(&r.str()?, false)?;
                w.u8(fs.noexec(&path)? as u8);
            }
            _ => return Err(Error::NotSupported),
        }
        Ok(Reply::body(w))
    }
}
