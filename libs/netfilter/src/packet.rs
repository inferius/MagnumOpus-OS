//! Rozbor prijateho ramce Ethernet (jen co filtr potrebuje).

use core::net::Ipv4Addr;

const ETHERTYPE_IPV4: u16 = 0x0800;
const IP_ICMP: u8 = 1;
const IP_TCP: u8 = 6;
const IP_UDP: u8 = 17;
const ICMP_ECHO_REQUEST: u8 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proto {
    Tcp { syn: bool, ack: bool, rst: bool },
    Udp,
    /// ICMP echo request (ping na nas).
    IcmpEcho,
    /// Ostatni ICMP (odpovedi, chyby).
    Icmp,
    /// Dalsi fragment IPv4 (bez hlavicky protokolu).
    Fragment,
    Other,
}

impl Proto {
    /// Bity protokolu v pravidlech (stejne jako `netproto::proto`).
    pub const TCP_BIT: u8 = 1;
    pub const UDP_BIT: u8 = 2;
}

/// Prijaty paket. `src = None` = neni IPv4 (ARP, IPv6, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Packet {
    pub src: Option<Ipv4Addr>,
    pub dst: Option<Ipv4Addr>,
    pub proto: Proto,
    pub sport: u16,
    pub dport: u16,
}

fn be16(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*d.get(at)?, *d.get(at + 1)?]))
}

/// Rozebere ramec Ethernet. Poskozeny paket IPv4 = `Proto::Other` (stack
/// ho zahodi sam).
pub fn parse_frame(frame: &[u8]) -> Packet {
    let none = Packet {
        src: None,
        dst: None,
        proto: Proto::Other,
        sport: 0,
        dport: 0,
    };
    if be16(frame, 12) != Some(ETHERTYPE_IPV4) {
        return none;
    }
    let ip = &frame[14..];
    let Some(&vihl) = ip.first() else {
        return none;
    };
    let ihl = ((vihl & 0x0F) as usize) * 4;
    if vihl >> 4 != 4 || ihl < 20 || ip.len() < ihl {
        return none;
    }
    let src = Ipv4Addr::new(ip[12], ip[13], ip[14], ip[15]);
    let dst = Ipv4Addr::new(ip[16], ip[17], ip[18], ip[19]);
    let mut p = Packet {
        src: Some(src),
        dst: Some(dst),
        proto: Proto::Other,
        sport: 0,
        dport: 0,
    };
    let frag_offset = be16(ip, 6).unwrap_or(0) & 0x1FFF;
    if frag_offset != 0 {
        p.proto = Proto::Fragment;
        return p;
    }
    let l4 = &ip[ihl..];
    match ip[9] {
        IP_TCP => {
            let (Some(sport), Some(dport), Some(&flags)) = (be16(l4, 0), be16(l4, 2), l4.get(13))
            else {
                return p;
            };
            p.sport = sport;
            p.dport = dport;
            p.proto = Proto::Tcp {
                syn: flags & 0x02 != 0,
                ack: flags & 0x10 != 0,
                rst: flags & 0x04 != 0,
            };
        }
        IP_UDP => {
            let (Some(sport), Some(dport)) = (be16(l4, 0), be16(l4, 2)) else {
                return p;
            };
            p.sport = sport;
            p.dport = dport;
            p.proto = Proto::Udp;
        }
        IP_ICMP => {
            p.proto = match l4.first() {
                Some(&ICMP_ECHO_REQUEST) => Proto::IcmpEcho,
                Some(_) => Proto::Icmp,
                None => Proto::Other,
            };
        }
        _ => {}
    }
    p
}
