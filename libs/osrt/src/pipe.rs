//! Roury (`io::pipe`, `Stdio::piped`): dvojice kanalu s protokolem proudu
//! bajtu (stejny jako stdin/stdout, viz [`crate::stdio`]).

use alloc::vec::Vec;
use core::time::Duration;

use abi::Error;
use libos::Channel;
use libos::sync::Mutex;

use crate::Result;

/// Jeden konec roury (cteni i zapis jdou na kterykoli konec; roura
/// z [`pipe`] ma cteci a zapisovaci konec podle poradi).
pub struct Pipe {
    ch: Channel,
    /// Zbytek posledni prijate zpravy, ktery se neveselo ctenari.
    rest: Mutex<(Vec<u8>, usize)>,
}

/// Nova roura: (cteci konec, zapisovaci konec).
pub fn pipe() -> Result<(Pipe, Pipe)> {
    let (a, b) = Channel::create()?;
    Ok((Pipe::from_channel(a), Pipe::from_channel(b)))
}

impl Pipe {
    pub fn from_channel(ch: Channel) -> Self {
        Self { ch, rest: Mutex::new((Vec::new(), 0)) }
    }

    pub fn into_channel(self) -> Channel {
        self.ch
    }

    /// Precte az `buf.len()` bajtu; 0 = druha strana roury zavrela.
    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        read_stream(&self.ch, &self.rest, buf)
    }

    /// Jako [`Pipe::read`] bez cekani (`O_NONBLOCK`): prazdna roura =
    /// `ShouldWait` (C `EAGAIN`).
    pub fn try_read(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut rest = self.rest.lock();
        while rest.1 >= rest.0.len() {
            match self.ch.try_read() {
                Ok(msg) => *rest = (msg.data, 0),
                Err(Error::PeerClosed) => return Ok(0),
                Err(e) => return Err(e),
            }
        }
        let (data, at) = &mut *rest;
        let n = buf.len().min(data.len() - *at);
        buf[..n].copy_from_slice(&data[*at..*at + n]);
        *at += n;
        Ok(n)
    }

    /// Zapise cely `buf` (po zpravach nejvys `CHANNEL_MAX_DATA`).
    pub fn write(&self, buf: &[u8]) -> Result<usize> {
        write_stream(&self.ch, buf)?;
        Ok(buf.len())
    }

    /// Handle kanalu (cekani v `poll`).
    pub fn handle(&self) -> &libos::Handle {
        self.ch.handle()
    }

    /// Ma konec prectena, ale nevydana data (zbytek zpravy)?
    pub fn has_buffered(&self) -> bool {
        let r = self.rest.lock();
        r.1 < r.0.len()
    }

    /// Duplikat konce (ctou nebo pisou oba; zbytek zpravy se nesdili).
    pub fn try_clone(&self) -> Result<Pipe> {
        Ok(Pipe::from_channel(Channel::from_handle(self.ch.handle().duplicate(None)?)))
    }
}

/// Cteni proudu z kanalu se zbytkem posledni zpravy.
pub(crate) fn read_stream(ch: &Channel, rest: &Mutex<(Vec<u8>, usize)>, buf: &mut [u8]) -> Result<usize> {
    if buf.is_empty() {
        return Ok(0);
    }
    let mut rest = rest.lock();
    // Prazdna zprava neni konec proudu (ten je jen zavreni kanalu).
    while rest.1 >= rest.0.len() {
        match ch.read(None) {
            Ok(msg) => *rest = (msg.data, 0),
            Err(Error::PeerClosed) => return Ok(0),
            Err(e) => return Err(e),
        }
    }
    let (data, at) = &mut *rest;
    let n = buf.len().min(data.len() - *at);
    buf[..n].copy_from_slice(&data[*at..*at + n]);
    *at += n;
    Ok(n)
}

/// Zapis proudu do kanalu. Plna fronta prijemce = kratke cekani a znovu
/// (kanal nema signal "lze zapisovat"); zavreny kanal = `PeerClosed`.
pub(crate) fn write_stream(ch: &Channel, buf: &[u8]) -> Result<()> {
    for chunk in buf.chunks(abi::syscall::CHANNEL_MAX_DATA) {
        loop {
            match ch.write(chunk, Vec::new()) {
                Ok(()) => break,
                Err(Error::ShouldWait) => libos::time::sleep(Duration::from_millis(1)),
                Err(e) => return Err(e),
            }
        }
    }
    Ok(())
}
