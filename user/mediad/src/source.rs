//! Zdroj bajtu relace: VMO od aplikace nebo kanal protokolu
//! `mediaproto::source`. Dekoder se pta pres svuj kanal zdroje, `mediad`
//! odpovida odsud - dekoder nikdy nedostane handle VMO ani kanal aplikace.

use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use libos::{Channel, Error, Result, Vmo};
use mediaproto::source::{self, SourceRequest};

/// Jak dlouho cekat na odpoved zdroje aplikace.
const APP_TIMEOUT: Duration = Duration::from_secs(5);

pub enum Source {
    Vmo { vmo: Vmo, size: u64 },
    App(Channel),
}

impl Source {
    /// Zjisti velikost (u kanalu se zepta aplikace).
    pub fn size(&self) -> Result<u64> {
        match self {
            Source::Vmo { size, .. } => Ok(*size),
            Source::App(ch) => {
                let d = self.ask(ch, SourceRequest::Size)?;
                if d.len() != 8 {
                    return Err(Error::Corrupted);
                }
                Ok(u64::from_le_bytes(d.try_into().map_err(|_| Error::Corrupted)?))
            }
        }
    }

    fn ask(&self, ch: &Channel, req: SourceRequest) -> Result<Vec<u8>> {
        ch.write(&req.encode(), Vec::new())?;
        let m = ch.read(Some(libos::time::deadline_after(APP_TIMEOUT)))?;
        let d = source::decode_reply(&m.data)?;
        if let SourceRequest::ReadAt { len, .. } = req {
            if d.len() > len as usize {
                return Err(Error::Corrupted);
            }
        }
        Ok(d.to_vec())
    }

    /// Prvnich az `n` bajtu zdroje (volba dekoderu podle magie).
    pub fn peek(&self, n: u32) -> Result<Vec<u8>> {
        match self {
            Source::Vmo { vmo, size } => {
                let (_, len) = source::clamp(0, n, *size);
                let mut buf = vec![0u8; len];
                if len > 0 {
                    vmo.read(0, &mut buf)?;
                }
                Ok(buf)
            }
            Source::App(ch) => self.ask(ch, SourceRequest::ReadAt { offset: 0, len: n }),
        }
    }

    /// Odpovi na pozadavek dekoderu.
    pub fn serve(&self, msg: &[u8]) -> Vec<u8> {
        let req = match SourceRequest::decode(msg) {
            Ok(r) => r,
            Err(e) => return source::encode_reply(Err(e)),
        };
        let r = match (self, req) {
            (Source::Vmo { vmo, size }, SourceRequest::ReadAt { offset, len }) => {
                let (off, n) = source::clamp(offset, len, *size);
                let mut buf = vec![0u8; n];
                if n == 0 { Ok(buf) } else { vmo.read(off, &mut buf).map(|_| buf) }
            }
            (Source::Vmo { size, .. }, SourceRequest::Size) => Ok(size.to_le_bytes().to_vec()),
            (Source::App(ch), r) => self.ask(ch, r),
        };
        match r {
            Ok(d) => source::encode_reply(Ok(&d)),
            Err(e) => source::encode_reply(Err(e)),
        }
    }
}
