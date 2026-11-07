//! Kodovani zprav: spolecny zaklad z crate `wire` (cisla little-endian,
//! retezec = delka u16 + UTF-8) a navic adresy IP: rodina u8 (4 nebo 6)
//! + 4 nebo 16 B, `socket_addr` = adresa + port u16.

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use abi::{Error, Result};

pub use wire::{Reader, Writer};

/// Zapis adres IP (rozsireni `Writer`).
pub trait WriteAddr {
    fn ip(&mut self, a: IpAddr) -> &mut Self;
    fn socket_addr(&mut self, a: SocketAddr) -> &mut Self;
}

/// Cteni adres IP (rozsireni `Reader`).
pub trait ReadAddr {
    fn ip(&mut self) -> Result<IpAddr>;
    fn socket_addr(&mut self) -> Result<SocketAddr>;
}

impl WriteAddr for Writer {
    fn ip(&mut self, a: IpAddr) -> &mut Self {
        match a {
            IpAddr::V4(v4) => self.u8(4).bytes(&v4.octets()),
            IpAddr::V6(v6) => self.u8(6).bytes(&v6.octets()),
        }
    }

    fn socket_addr(&mut self, a: SocketAddr) -> &mut Self {
        self.ip(a.ip()).u16(a.port())
    }
}

impl ReadAddr for Reader<'_> {
    fn ip(&mut self) -> Result<IpAddr> {
        match self.u8()? {
            4 => {
                let o: [u8; 4] = self.take(4)?.try_into().unwrap();
                Ok(IpAddr::V4(Ipv4Addr::from(o)))
            }
            6 => {
                let o: [u8; 16] = self.take(16)?.try_into().unwrap();
                Ok(IpAddr::V6(Ipv6Addr::from(o)))
            }
            _ => Err(Error::InvalidArgs),
        }
    }

    fn socket_addr(&mut self) -> Result<SocketAddr> {
        let ip = self.ip()?;
        Ok(SocketAddr::new(ip, self.u16()?))
    }
}
