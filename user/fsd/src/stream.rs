//! Soubor jako proud bajtu (fsproto v2 `OPEN_STREAM`): stdin/stdout
//! potomka `posix_spawn` presmerovany do souboru (docs/cs/cstd.md,
//! kapitola Procesy).
//!
//! Proud ma vlastni handle souboru (`reopen` tehoz objektu), takze prezije
//! zavreni relace klienta. Obsluhuje ho hlavni smycka fsd vedle relaci:
//! - **cteni** (`READ`): fsd posila kusy souboru od pozice, dokud se
//!   fronta kanalu nezaplni (`SHOULD_WAIT`), pak to zkusi pri dalsim kole
//!   (kanal nema signal "zapisovatelny": smycka se probudi nejpozdeji za
//!   [`RETRY`]). Na konci souboru kanal zavre = konec proudu u ctenare.
//! - **zapis** (`WRITE`, `APPEND`): kazda zprava kanalu = data k zapisu
//!   (od pozice, resp. na konec souboru). Zavreni kanalu = konec proudu.
//!
//! Restart fsd proudy zahodi (ctenar/zapisovatel uvidi zavreny kanal).

use alloc::vec::Vec;

use fsproto::v2::stream_mode;
use libos::{Channel, Error, Signals};

use crate::blkdev::BlkDev;

/// Nejvic soubeznych proudu.
pub const MAX_STREAMS: usize = 16;
/// Kus cteni poslany jednou zpravou.
const CHUNK: usize = 60 * 1024;
/// Za jak dlouho zkusit znovu poslat do plne fronty (ns).
pub const RETRY: u64 = 2_000_000;

pub struct Stream {
    pub chan: Channel,
    /// Vlastni handle fsd na soubor.
    pub h: u64,
    pub mode: u8,
    pub offset: u64,
    /// Kus cteni, ktery se do plne fronty nevesel.
    pending: Option<Vec<u8>>,
    /// Klic a stav registrace na portu hlavni smycky (jako `Session`).
    pub key: u64,
    pub armed: bool,
}

impl Stream {
    pub fn new(chan: Channel, h: u64, mode: u8, offset: u64) -> Self {
        Stream { chan, h, mode, offset, pending: None, key: 0, armed: false }
    }

    /// Na co ceka smycka (registrace na portu).
    pub fn signals(&self) -> Signals {
        if self.mode == stream_mode::READ { Signals::PEER_CLOSED } else { Signals::READABLE | Signals::PEER_CLOSED }
    }

    /// Ma cteci proud data, ktera cekaji na misto ve fronte?
    pub fn blocked(&self) -> bool {
        self.pending.is_some()
    }
}

/// Jedno kolo proudu. `Ok(true)` = neco zapsal do souboru, `Err` = proud
/// skoncil (zavrit).
pub fn pump(fs: &mut fs::Fs<BlkDev>, s: &mut Stream) -> Result<bool, ()> {
    if s.mode == stream_mode::READ {
        loop {
            let data = match s.pending.take() {
                Some(d) => d,
                None => {
                    let d = fs.read_h(s.h, s.offset, CHUNK).map_err(|_| ())?;
                    if d.is_empty() {
                        return Err(());
                    }
                    s.offset += d.len() as u64;
                    d
                }
            };
            match s.chan.write(&data, Vec::new()) {
                Ok(()) => {}
                Err(Error::ShouldWait) => {
                    s.pending = Some(data);
                    return Ok(false);
                }
                Err(_) => return Err(()),
            }
        }
    }
    let mut wrote = false;
    loop {
        let msg = match s.chan.try_read() {
            Ok(m) => m,
            Err(Error::ShouldWait) => return Ok(wrote),
            Err(_) => return Err(()),
        };
        if msg.data.is_empty() {
            continue;
        }
        let at = if s.mode == stream_mode::APPEND { fs.stat_h(s.h).map_err(|_| ())?.size } else { s.offset };
        fs.write_h(s.h, at, &msg.data).map_err(|_| ())?;
        s.offset = at + msg.data.len() as u64;
        wrote = true;
    }
}
