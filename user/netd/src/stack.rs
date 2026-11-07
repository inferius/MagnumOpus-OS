//! Stav sitoveho stacku: rozhrani smoltcp, sockety, DHCP, servery DNS,
//! pridelovani portu.
//!
//! smoltcp je tu schovany: zbytek sluzby pracuje s `Stack` a s handly
//! socketu. Vlastni stack (TODO, viz `docs/cs/sit.md`) nahradi tento modul
//! a moduly socketu; protokol `netproto` a ovladac se nezmeni.

use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};

use libos::{Error, Result, println};
use netproto::Info;
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::socket::{dhcpv4, tcp, udp};
use smoltcp::time::{Duration as SmolDuration, Instant};
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpCidr, Ipv4Cidr};

use crate::link::Link;
use crate::rand::Rng;

/// Velikost bufferu spojeni TCP (kazdy smer).
pub const TCP_BUFFER: usize = 64 * 1024;
/// Prijimaci a odesilaci fronta socketu UDP: pocet datagramu a bajty.
const UDP_PACKETS: usize = 32;
const UDP_BYTES: usize = 64 * 1024;
/// Docasne porty (RFC 6335).
const EPHEMERAL: core::ops::RangeInclusive<u16> = 49152..=65535;

/// Protokol obsazeneho portu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PortProto {
    Tcp,
    Udp,
}

pub struct Stack {
    pub iface: Interface,
    pub sockets: SocketSet<'static>,
    pub dev: Link,
    dhcp: SocketHandle,
    /// Servery DNS z DHCP.
    dhcp_dns: Vec<SocketAddr>,
    /// Servery DNS nastavene rucne (`SET_DNS`); maji prednost.
    static_dns: Vec<SocketAddr>,
    pub configured: bool,
    gateway: Option<Ipv4Addr>,
    pub rng: Rng,
    ports: BTreeSet<(PortProto, u16)>,
    /// Spojeni zavrena klientem, ktera jeste dobihaji: (socket, termin,
    /// obsazeny docasny port).
    pub closing: Vec<(SocketHandle, u64, Option<u16>)>,
}

/// Monotonni cas v ns.
pub fn now_ns() -> u64 {
    libos::time::now()
}

pub fn instant(ns: u64) -> Instant {
    Instant::from_micros((ns / 1000) as i64)
}

/// Termin z timeoutu na drate (ms; `FOREVER` = zadny).
pub fn deadline(timeout_ms: u64) -> u64 {
    if timeout_ms == netproto::FOREVER {
        u64::MAX
    } else {
        now_ns().saturating_add(timeout_ms.saturating_mul(1_000_000))
    }
}

/// IPv4 z adresy protokolu; IPv6 zatim = `UNREACHABLE`.
pub fn v4(ip: IpAddr) -> Result<Ipv4Addr> {
    match ip {
        IpAddr::V4(a) => Ok(a),
        IpAddr::V6(_) => Err(Error::Unreachable),
    }
}

impl Stack {
    pub fn new(mut dev: Link) -> Self {
        let mac = dev.info.mac;
        let (mut rng, seeded) = Rng::new();
        if !seeded {
            println!(
                "netd: VAROVANI: generator jadra neni nasazeny, ID dotazu DNS a cisla TCP jsou odhadnutelna"
            );
        }
        let mut config = Config::new(HardwareAddress::Ethernet(EthernetAddress(mac)));
        config.random_seed = rng.next_u64();
        let iface = Interface::new(config, &mut dev, instant(now_ns()));
        let mut sockets = SocketSet::new(vec![]);
        let dhcp = sockets.add(dhcpv4::Socket::new());
        Self {
            iface,
            sockets,
            dev,
            dhcp,
            dhcp_dns: Vec::new(),
            static_dns: Vec::new(),
            configured: false,
            gateway: None,
            rng,
            ports: BTreeSet::new(),
            closing: Vec::new(),
        }
    }

    /// Zpracuje prijate ramce, casovace a odeslani; pak udalosti DHCP.
    pub fn poll(&mut self) {
        let now = instant(now_ns());
        self.iface.poll(now, &mut self.dev, &mut self.sockets);
        self.dhcp_events();
    }

    /// Za kolik ns chce smoltcp znovu zavolat `poll` (casovace TCP, DHCP).
    pub fn poll_delay_ns(&mut self) -> Option<u64> {
        let now = instant(now_ns());
        self.iface
            .poll_delay(now, &self.sockets)
            .map(|d: SmolDuration| d.total_micros().saturating_mul(1000))
    }

    fn dhcp_events(&mut self) {
        let event = self.sockets.get_mut::<dhcpv4::Socket>(self.dhcp).poll();
        match event {
            Some(dhcpv4::Event::Configured(cfg)) => {
                let addr: Ipv4Cidr = cfg.address;
                self.iface.update_ip_addrs(|a| {
                    a.clear();
                    let _ = a.push(IpCidr::Ipv4(addr));
                });
                match cfg.router {
                    Some(r) => {
                        let _ = self.iface.routes_mut().add_default_ipv4_route(r);
                    }
                    None => {
                        self.iface.routes_mut().remove_default_ipv4_route();
                    }
                }
                self.dhcp_dns = cfg
                    .dns_servers
                    .iter()
                    .map(|d| SocketAddr::new(IpAddr::V4(*d), netproto::dns::PORT))
                    .collect();
                self.gateway = cfg.router;
                self.configured = true;
                println!(
                    "netd: DHCP: adresa {}, brana {}, DNS {:?}",
                    addr,
                    cfg.router
                        .map(IpAddr::V4)
                        .map_or(alloc::string::String::from("zadna"), |r| alloc::format!(
                            "{r}"
                        )),
                    cfg.dns_servers.as_slice()
                );
            }
            Some(dhcpv4::Event::Deconfigured) => {
                // Pri startu DHCP hlasi "nenastaveno" i bez predchozi adresy.
                if !self.configured {
                    return;
                }
                self.iface.update_ip_addrs(|a| a.clear());
                self.iface.routes_mut().remove_default_ipv4_route();
                self.dhcp_dns.clear();
                self.gateway = None;
                self.configured = false;
                println!("netd: DHCP: adresa ztracena");
            }
            None => {}
        }
    }

    /// Systemove servery DNS (rucne nastavene, jinak z DHCP).
    pub fn dns_servers(&self) -> &[SocketAddr] {
        if self.static_dns.is_empty() {
            &self.dhcp_dns
        } else {
            &self.static_dns
        }
    }

    pub fn set_dns(&mut self, servers: Vec<SocketAddr>) {
        self.static_dns = servers;
    }

    pub fn info(&self, rights: u32) -> Info {
        let addr = self.iface.ipv4_addr().map(|a| {
            let prefix = self
                .iface
                .ip_addrs()
                .iter()
                .find_map(|c| match c {
                    IpCidr::Ipv4(c) => Some(c.prefix_len()),
                    #[allow(unreachable_patterns)]
                    _ => None,
                })
                .unwrap_or(32);
            (IpAddr::V4(a), prefix)
        });
        let gateway = self.gateway.map(IpAddr::V4);
        Info {
            mac: self.dev.info.mac,
            mtu: self.dev.info.mtu,
            link_up: self.dev.info.flags & netproto::link::FLAG_UP != 0,
            configured: self.configured,
            addr,
            gateway,
            dns: self.dns_servers().iter().map(|s| s.ip()).collect(),
            rights,
        }
    }

    /// Mistni adresa IPv4 rozhrani (nenastavene = `UNREACHABLE`).
    pub fn local_ip(&self) -> Result<Ipv4Addr> {
        if !self.configured {
            return Err(Error::Unreachable);
        }
        self.iface.ipv4_addr().ok_or(Error::Unreachable)
    }

    /// Mistni adresa socketu na portu `port` podle aktualniho stavu
    /// rozhrani (pred DHCP 0.0.0.0). Pocita se pri kazdem dotazu, ne pri
    /// vytvoreni socketu, aby adresa po DHCP nezustala neplatna.
    pub fn local_addr(&self, port: u16) -> SocketAddr {
        let ip = self
            .local_ip()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        SocketAddr::new(ip, port)
    }

    /// Obsadi port `port` (0 = volny docasny). Obsazeny = `ADDRESS_IN_USE`.
    pub fn take_port(&mut self, proto: PortProto, port: u16) -> Result<u16> {
        if port != 0 {
            return if self.ports.insert((proto, port)) {
                Ok(port)
            } else {
                Err(Error::AddressInUse)
            };
        }
        let span = (EPHEMERAL.end() - EPHEMERAL.start()) as u32 + 1;
        let start = self.rng.next_u16() as u32 % span;
        for i in 0..span {
            let p = EPHEMERAL.start() + ((start + i) % span) as u16;
            if self.ports.insert((proto, p)) {
                return Ok(p);
            }
        }
        Err(Error::NoResources)
    }

    pub fn release_port(&mut self, proto: PortProto, port: u16) {
        self.ports.remove(&(proto, port));
    }

    pub fn new_tcp(&mut self) -> SocketHandle {
        let mut s = tcp::Socket::new(
            tcp::SocketBuffer::new(vec![0u8; TCP_BUFFER]),
            tcp::SocketBuffer::new(vec![0u8; TCP_BUFFER]),
        );
        // Udrzovani spojeni: mrtvou druhou stranu poznat do ~2 min.
        s.set_keep_alive(Some(SmolDuration::from_secs(60)));
        s.set_timeout(Some(SmolDuration::from_secs(120)));
        self.sockets.add(s)
    }

    pub fn new_udp(&mut self) -> SocketHandle {
        let rx = udp::PacketBuffer::new(
            vec![udp::PacketMetadata::EMPTY; UDP_PACKETS],
            vec![0u8; UDP_BYTES],
        );
        let tx = udp::PacketBuffer::new(
            vec![udp::PacketMetadata::EMPTY; UDP_PACKETS],
            vec![0u8; UDP_BYTES],
        );
        self.sockets.add(udp::Socket::new(rx, tx))
    }

    pub fn tcp(&mut self, h: SocketHandle) -> &mut tcp::Socket<'static> {
        self.sockets.get_mut::<tcp::Socket>(h)
    }

    pub fn udp(&mut self, h: SocketHandle) -> &mut udp::Socket<'static> {
        self.sockets.get_mut::<udp::Socket>(h)
    }

    pub fn remove(&mut self, h: SocketHandle) {
        self.sockets.remove(h);
    }
}
