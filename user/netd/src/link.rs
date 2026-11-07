//! Zarizeni pro smoltcp nad kanalem k ovladaci sitove karty
//! (`netproto::link`).

use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;
use core::net::Ipv4Addr;

use libos::{Channel, Error};
use netproto::link;
use smoltcp::phy::{self, DeviceCapabilities, Medium};
use smoltcp::time::Instant;

/// Nejvic prijatych ramcu ve fronte sluzby (zbytek se zahodi).
const RX_QUEUE_MAX: usize = 512;
/// Nejvic zapamatovanych chyb ICMP pro TCP.
const ICMP_ERRORS_MAX: usize = 16;

/// Chyba ICMP "cil nedosazitelny" k odeslanemu segmentu TCP: cil, port
/// cile, mistni port a kod ICMP (3 = port nedosazitelny).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IcmpError {
    pub dest: Ipv4Addr,
    pub dest_port: u16,
    pub local_port: u16,
    pub code: u8,
}

/// Rozpozna v ramci ICMP "cil nedosazitelny" k segmentu TCP. smoltcp
/// chyby ICMP u TCP nezpracovava; sluzba podle nich ukonci navazovani
/// spojeni (RFC 1122 4.2.3.9) misto cekani na vyprseni.
fn icmp_tcp_error(f: &[u8]) -> Option<IcmpError> {
    if f.len() < 14 + 20 || f[12..14] != [0x08, 0x00] {
        return None;
    }
    let ip = &f[14..];
    let ihl = (ip[0] & 0x0F) as usize * 4;
    if ip[0] >> 4 != 4 || ihl < 20 || ip[9] != 1 {
        return None;
    }
    let icmp = ip.get(ihl..)?;
    // Typ 3 = cil nedosazitelny; za hlavickou ICMP (8 B) puvodni IP + 8 B.
    if icmp.len() < 8 + 20 + 8 || icmp[0] != 3 {
        return None;
    }
    let inner = &icmp[8..];
    let inner_ihl = (inner[0] & 0x0F) as usize * 4;
    if inner[0] >> 4 != 4 || inner_ihl < 20 || inner[9] != 6 {
        return None;
    }
    let tcp = inner.get(inner_ihl..inner_ihl + 4)?;
    Some(IcmpError {
        dest: Ipv4Addr::new(inner[16], inner[17], inner[18], inner[19]),
        dest_port: u16::from_be_bytes([tcp[2], tcp[3]]),
        local_port: u16::from_be_bytes([tcp[0], tcp[1]]),
        code: icmp[1],
    })
}

pub struct Link {
    ch: Channel,
    /// Prijate zpravy `FRAME` (vcetne bajtu typu).
    rx: VecDeque<Vec<u8>>,
    pub info: link::Info,
    /// Posledni chyby ICMP k segmentum TCP (viz `icmp_tcp_error`).
    pub icmp_errors: VecDeque<IcmpError>,
    pub rx_dropped: u64,
    pub tx_dropped: u64,
}

impl Link {
    pub fn new(ch: Channel, info: link::Info) -> Self {
        Self {
            ch,
            rx: VecDeque::new(),
            info,
            icmp_errors: VecDeque::new(),
            rx_dropped: 0,
            tx_dropped: 0,
        }
    }

    pub fn channel(&self) -> &Channel {
        &self.ch
    }

    /// Precte cekajici zpravy od ovladace; ramec, pro ktery `accept`
    /// vrati `false` (prichozi filtr), se zahodi. `Err(PEER_CLOSED)` =
    /// ovladac skoncil.
    pub fn pump(&mut self, accept: &mut dyn FnMut(&[u8]) -> bool) -> libos::Result<()> {
        // Omezeny pocet za kolo, aby zaplava ramcu nezablokovala klienty.
        for _ in 0..256 {
            let msg = match self.ch.try_read() {
                Ok(m) => m,
                Err(Error::ShouldWait) => return Ok(()),
                Err(e) => return Err(e),
            };
            match msg.data.first() {
                Some(&link::FRAME) if msg.data.len() > 14 => {
                    if !accept(&msg.data[1..]) {
                        continue;
                    }
                    if self.rx.len() >= RX_QUEUE_MAX {
                        self.rx_dropped += 1;
                    } else {
                        // Zprava se uklada cela (typ + ramec); ramec je od
                        // bajtu 1, bez kopirovani.
                        let d = msg.data;
                        if let Some(e) = icmp_tcp_error(&d[1..]) {
                            if self.icmp_errors.len() >= ICMP_ERRORS_MAX {
                                self.icmp_errors.pop_front();
                            }
                            self.icmp_errors.push_back(e);
                        }
                        self.rx.push_back(d);
                    }
                }
                Some(&link::LINK) if msg.data.len() >= 2 => self.info.flags = msg.data[1],
                _ => {}
            }
        }
        Ok(())
    }

    /// Vyjme chybu ICMP ke spojeni `local_port` -> `dest:dest_port`.
    pub fn take_icmp_error(
        &mut self,
        dest: Ipv4Addr,
        dest_port: u16,
        local_port: u16,
    ) -> Option<u8> {
        let i = self.icmp_errors.iter().position(|e| {
            e.dest == dest && e.dest_port == dest_port && e.local_port == local_port
        })?;
        self.icmp_errors.remove(i).map(|e| e.code)
    }

    pub fn has_rx(&self) -> bool {
        !self.rx.is_empty()
    }
}

/// Prijata zprava `FRAME` od ovladace: bajt 0 = typ, ramec od bajtu 1.
pub struct RxToken(Vec<u8>);

pub struct TxToken<'a> {
    link: &'a Channel,
    dropped: &'a mut u64,
}

impl phy::RxToken for RxToken {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        f(&self.0[1..])
    }
}

impl phy::TxToken for TxToken<'_> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buf = vec![0u8; 1 + len];
        buf[0] = link::FRAME;
        let r = f(&mut buf[1..]);
        // Plna fronta ovladace = ramec zahodit (TCP ho posle znovu).
        if self.link.write(&buf, Vec::new()).is_err() {
            *self.dropped += 1;
        }
        r
    }
}

impl phy::Device for Link {
    type RxToken<'a> = RxToken;
    type TxToken<'a> = TxToken<'a>;

    fn receive(&mut self, _timestamp: Instant) -> Option<(RxToken, TxToken<'_>)> {
        let frame = self.rx.pop_front()?;
        Some((
            RxToken(frame),
            TxToken {
                link: &self.ch,
                dropped: &mut self.tx_dropped,
            },
        ))
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<TxToken<'_>> {
        Some(TxToken {
            link: &self.ch,
            dropped: &mut self.tx_dropped,
        })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = self.info.mtu as usize + 14;
        caps
    }
}
