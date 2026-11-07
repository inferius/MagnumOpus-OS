//! Nastaveni casu objektu (napr. kopie se zachovanim casu, ovladac Windows
//! `SetBasicInfo`, rozbaleni archivu).
//!
//! Nastavit jde cas zmeny obsahu, vytvoreni a posledniho pristupu. Cas
//! zmeny metadat (`ctime`) nastavit nejde: je to zaznam o tom, ze se
//! objekt menil, a nastavi se na aktualni cas (jako v POSIX).

use abi::Result;

use super::handles::{Handle, access};
use super::{Fs, zil};
use crate::device::BlockDevice;
use crate::layout::Timestamp;

/// Casy k nastaveni; `None` = nechat beze zmeny.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SetTimes {
    pub mtime: Option<Timestamp>,
    pub btime: Option<Timestamp>,
    /// `Timestamp::UNIX_EPOCH` = nesleduje se.
    pub atime: Option<Timestamp>,
}

impl SetTimes {
    pub fn is_empty(&self) -> bool {
        self.mtime.is_none() && self.btime.is_none() && self.atime.is_none()
    }
}

impl<D: BlockDevice> Fs<D> {
    /// Nastavi casy objektu na ceste (soubor, adresar i odkaz).
    pub fn set_times(&mut self, path: &str, t: SetTimes) -> Result<()> {
        self.guarded(|fs| {
            let oid = fs.require(path, crate::perm::rights::WRITE)?;
            fs.s.check_writable()?;
            if t.is_empty() {
                return Ok(());
            }
            fs.ensure_room(4)?;
            fs.set_times_oid(oid, t)
        })
    }

    /// Nastavi casy objektu handlu (handle musi mit pravo zapisu).
    pub fn set_times_h(&mut self, h: Handle, t: SetTimes) -> Result<()> {
        let (o, _) = self.handle_object(h, access::WRITE)?;
        self.guarded(|fs| {
            fs.s.check_writable()?;
            if t.is_empty() {
                return Ok(());
            }
            fs.ensure_room(4)?;
            fs.set_times_oid(o.oid, t)
        })
    }

    /// Objekt `oid` aktivniho datasetu (i pri prehrani intent logu).
    pub(super) fn set_times_oid(&mut self, oid: u64, t: SetTimes) -> Result<()> {
        let mut inode = self.live_inode(oid)?;
        if let Some(m) = t.mtime {
            inode.mtime = m;
        }
        if let Some(b) = t.btime {
            inode.btime = b;
        }
        if let Some(a) = t.atime {
            inode.atime = a;
        }
        inode.ctime = self.now;
        self.put_inode(oid, &inode)?;
        self.zil.op(zil::Op::SetTimes { oid, times: t, time: self.now });
        Ok(())
    }
}
