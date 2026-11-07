//! Proudy bajtu: spolecne rozhrani pro TCP a vrstvy nad nim.
//!
//! [`TcpStream`](crate::TcpStream) implementuje [`Read`] a [`Write`]. TLS
//! (pozdeji) bude obal `TlsStream<S: Read + Write>`, ktery implementuje
//! totez, a klient HTTP bude pracovat s libovolnym `Read + Write` - nad
//! holym TCP i nad TLS stejne. Proto tu nic nezavisi na TCP.

use abi::{Error, Result};

pub trait Read {
    /// Precte nejvys `buf.len()` bajtu. `Ok(0)` = konec proudu (druha strana
    /// ukoncila zapis), pokud `buf` neni prazdny.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;

    /// Precte presne `buf.len()` bajtu; predcasny konec = `PEER_CLOSED`.
    fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<()> {
        while !buf.is_empty() {
            let n = self.read(buf)?;
            if n == 0 {
                return Err(Error::PeerClosed);
            }
            buf = &mut buf[n..];
        }
        Ok(())
    }

    /// Cte do konce proudu.
    fn read_to_end(&mut self, out: &mut alloc::vec::Vec<u8>) -> Result<usize> {
        let start = out.len();
        let mut chunk = [0u8; 4096];
        loop {
            let n = self.read(&mut chunk)?;
            if n == 0 {
                return Ok(out.len() - start);
            }
            out.extend_from_slice(&chunk[..n]);
        }
    }
}

pub trait Write {
    /// Zapise cast nebo vse z `buf`, vraci kolik.
    fn write(&mut self, buf: &[u8]) -> Result<usize>;

    /// Zapise cely `buf`.
    fn write_all(&mut self, mut buf: &[u8]) -> Result<()> {
        while !buf.is_empty() {
            let n = self.write(buf)?;
            if n == 0 {
                return Err(Error::PeerClosed);
            }
            buf = &buf[n..];
        }
        Ok(())
    }

    /// Odesle data drzena ve vrstve (TLS zaznam, buffer HTTP). TCP nedrzi
    /// nic - data predana sluzbe uz odchazeji.
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}
