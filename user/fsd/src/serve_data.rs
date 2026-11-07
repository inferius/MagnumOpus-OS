//! Operace dat (protokol v2, op 90-97, faze 3 FS): okamzite kopie,
//! predalokace, nastaveni dat datasetu a scrub.
//!
//! Cesty jsou cesty relace (`Session::map`); prava kontroluje FS podle
//! identity. Scrub je stav celeho svazku: ovladat ho smi relace s pravem
//! `MANAGE` na koreni svazku, provede se jako system.

use fsproto::data::{DataSettings, ScrubStatus, compress, dedup, op};
use fsproto::{Reader, Writer};
use libos::{Error, Result};

use crate::serve::Service;
use crate::serve_v2::{Reply, Session};

fn to_fs(s: &DataSettings) -> Result<fs::DataSettings> {
    let compress = match s.compress {
        compress::NONE => None,
        compress::LZ4 => Some(fs::Compress::Lz4),
        compress::ZSTD => Some(fs::Compress::Zstd),
        _ => return Err(Error::InvalidArgs),
    };
    let dedup = match s.dedup {
        dedup::OFF => fs::DedupMode::Off,
        dedup::BACKGROUND => fs::DedupMode::Background,
        dedup::INLINE => fs::DedupMode::Inline,
        _ => return Err(Error::InvalidArgs),
    };
    Ok(fs::DataSettings {
        compress,
        copies: s.copies,
        dedup,
    })
}

fn from_fs(s: &fs::DataSettings) -> DataSettings {
    DataSettings {
        compress: match s.compress {
            None => compress::NONE,
            Some(fs::Compress::Lz4) => compress::LZ4,
            Some(fs::Compress::Zstd) => compress::ZSTD,
        },
        copies: s.copies,
        dedup: s.dedup as u8,
    }
}

impl Service {
    /// Operace dat; `None` = neni z teto skupiny.
    pub fn handle_data(&mut self, sess: &mut Session, opcode: u32, r: &mut Reader) -> Option<Result<Reply>> {
        if !(op::CLONE..=op::SCRUB_STATUS).contains(&opcode) {
            return None;
        }
        Some(self.data(sess, opcode, r))
    }

    fn data(&mut self, sess: &mut Session, opcode: u32, r: &mut Reader) -> Result<Reply> {
        let mut w = Writer::new();
        let now = crate::serve::fs_now();
        let fs = self.fs.as_mut().ok_or(Error::IoError)?;
        fs.set_time(now);
        match opcode {
            op::CLONE => {
                let src = sess.map(&r.str()?, false)?;
                let dst = sess.map(&r.str()?, true)?;
                w.u64(fs.clone_path(&src, &dst)?);
            }
            op::CLONE_RANGE => {
                let src = sess.map(&r.str()?, false)?;
                let src_off = r.u64()?;
                let dst = sess.map(&r.str()?, true)?;
                let (dst_off, len) = (r.u64()?, r.u64()?);
                fs.clone_range(&src, src_off, &dst, dst_off, len)?;
            }
            op::FALLOCATE => {
                let path = sess.map(&r.str()?, true)?;
                let (offset, len, keep) = (r.u64()?, r.u64()?, r.u8()? != 0);
                fs.fallocate(&path, offset, len, keep)?;
            }
            op::DATA_SETTINGS => {
                let ds = sess.map(&r.str()?, false)?;
                from_fs(&fs.data_settings_of(&ds)?).put(&mut w);
            }
            op::SET_DATA_SETTINGS => {
                let ds = sess.map(&r.str()?, true)?;
                let s = to_fs(&DataSettings::get(r)?)?;
                fs.set_data_settings(&ds, &s)?;
            }
            op::SCRUB_START | op::SCRUB_STOP | op::SCRUB_STATUS => {
                // Pravo MANAGE na koreni svazku (FS ho spocita podle identity).
                if opcode != op::SCRUB_STATUS {
                    let root = sess.map("/", true)?;
                    if fs.stat(&root)?.rights & fs::rights::MANAGE == 0 {
                        return Err(Error::AccessDenied);
                    }
                }
                fs.set_principal(None);
                let res = match opcode {
                    op::SCRUB_START => fs.scrub_start(now).map(|()| None),
                    op::SCRUB_STOP => fs.scrub_stop().map(|()| None),
                    _ => fs.scrub_status().map(Some),
                };
                fs.set_principal(sess.identity.clone());
                if let Some(st) = res? {
                    ScrubStatus {
                        state: st.state.state,
                        started: st.state.started,
                        finished: st.state.finished,
                        scanned: st.state.scanned,
                        bad: st.state.bad,
                        repaired: st.state.repaired,
                        lost: st.state.lost,
                        errors: st.errors,
                    }
                    .put(&mut w);
                }
            }
            _ => return Err(Error::NotSupported),
        }
        Ok(Reply::body(w))
    }
}
