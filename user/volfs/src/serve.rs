//! Obsluha jedne zpravy `volproto` nad svazkem (bez kanalu: testy na
//! hostiteli volaji primo [`handle`]).

use volproto::{Error, MAX_IO, Result, Writer, kind, normalize, op, open_flags, parse_request, reply};

use crate::Volume;

/// Vysledek obsluhy: odpoved a jestli proces ma skoncit (`UNMOUNT`).
pub struct Outcome {
    pub reply: Vec<u8>,
    pub stop: bool,
}

/// Obslouzi pozadavek `msg`. Nesmyslna zprava = odpoved s chybou (nebo
/// prazdna odpoved bez cisla pozadavku, kdyz ho nejde precist).
pub fn handle(vol: &mut dyn Volume, msg: &[u8]) -> Outcome {
    let Ok((id, opcode, mut r)) = parse_request(msg) else {
        return Outcome { reply: reply(0, Err(Error::InvalidArgs)), stop: false };
    };
    let mut stop = false;
    let res: Result<Writer> = (|| {
        let mut w = Writer::new();
        match opcode {
            op::INFO => vol.info()?.put(&mut w),
            op::STAT => vol.stat(&normalize(&r.str()?)?)?.put(&mut w),
            op::LIST => {
                let list = vol.list(&normalize(&r.str()?)?)?;
                w.u32(list.len() as u32);
                for (name, st) in &list {
                    w.str(name);
                    st.put(&mut w);
                }
            }
            op::READ => {
                let path = normalize(&r.str()?)?;
                let (off, len) = (r.u64()?, (r.u32()? as usize).min(MAX_IO));
                w.bytes(&vol.read(&path, off, len)?);
            }
            op::WRITE => {
                let path = normalize(&r.str()?)?;
                let off = r.u64()?;
                let data = r.rest();
                if data.len() > MAX_IO {
                    return Err(Error::InvalidArgs);
                }
                w.u32(vol.write(&path, off, data)? as u32);
            }
            op::CREATE => {
                let path = normalize(&r.str()?)?;
                let (k, excl) = (r.u8()?, r.u8()? != 0);
                if k != kind::FILE && k != kind::DIR {
                    return Err(Error::InvalidArgs);
                }
                vol.create(&path, k, excl)?;
            }
            op::REMOVE => vol.remove(&normalize(&r.str()?)?)?,
            op::RENAME => {
                let (from, to) = (normalize(&r.str()?)?, normalize(&r.str()?)?);
                vol.rename(&from, &to, r.u8()? != 0)?;
            }
            op::SET_LEN => {
                let path = normalize(&r.str()?)?;
                vol.set_len(&path, r.u64()?)?;
            }
            op::OPEN => {
                let path = normalize(&r.str()?)?;
                let f = r.u8()?;
                if f & open_flags::CREATE != 0 {
                    let k = if f & open_flags::DIR != 0 { kind::DIR } else { kind::FILE };
                    vol.create(&path, k, f & open_flags::EXCL != 0)?;
                }
                let st = vol.stat(&path)?;
                if f & open_flags::TRUNCATE != 0 && st.kind == kind::FILE && st.size != 0 {
                    vol.set_len(&path, 0)?;
                    vol.stat(&path)?.put(&mut w);
                } else {
                    st.put(&mut w);
                }
            }
            op::SYNC => vol.sync()?,
            op::UNMOUNT => {
                vol.unmount()?;
                stop = true;
            }
            _ => return Err(Error::NotSupported),
        }
        Ok(w)
    })();
    Outcome { reply: reply(id, res), stop }
}
