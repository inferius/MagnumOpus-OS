//! Stav sitoveho stacku: rozhrani smoltcp, sockety, DHCP, servery DNS,
//! pridelovani portu a firewall ([`crate::fw`]; kazdy prijaty ramec projde
//! prichozim filtrem driv, nez ho dostane smoltcp).
//!
//! smoltcp je tu schovany: zbytek sluzby pracuje s `Stack` a s handly
//! socketu. Vlastni stack (TODO, viz `docs/cs/sit.md`) nahradi tento modul
//! a moduly socketu; protokol `netproto` a ovladac se nezmeni.

use alloc::collections::{BTreeSet, VecDeque};
use alloc::vec;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};

use libos::{Error, Result, println};
use netproto::Info;
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::socket::{dhcpv4, tcp, udp};
use smoltcp::time::{Duration as SmolDuration, Instant};
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpCidr, Ipv4Cidr};

use crate::fw::Firewall;
use crate::link::Link;
use crate::rand::Rng;

/// Pocet datagramu ve fronte socketu UDP: jeden na kazdy zacaty 2 KiB
/// bufferu (male datagramy), nejmene 8.
fn udp_packets(bytes: usize) -> usize {
    (bytes / 2048).max(8)
}

/// Docasne porty (RFC 6335).
const EPHEMERAL: core::ops::RangeInclusive<u16> = 49152..=65535;

/// Kolik naposledy uvolnenych docasnych portu se nepridely znovu (karantena,
/// ctvrtina rozsahu). Druha strana nebo NAT (slirp drzi relaci UDP 240 s
/// podle mistniho portu) muze mit o starem toku jeste stav; novy socket na
/// stejnem portu by pak zdedil jeho chyby nebo pozdni odpovedi.
const PORT_QUARANTINE: usize = 4096;

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
    /// Naposledy uvolnene docasne porty (poradi, [`PORT_QUARANTINE`]).
    recent: VecDeque<(PortProto, u16)>,
    recent_set: BTreeSet<(PortProto, u16)>,
    /// Porty TCP, na kterych nekdo nasloucha (pro prichozi filtr).
    listening: BTreeSet<u16>,
    pub fw: Firewall,
    /// Systemovy preklad pres dnsd (sifrovane DNS, DNSSEC).
    pub resolver: crate::forward::Forwarder,
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

/// Chyba propadleho terminu: termin "jen zkusit" ([`netproto::TRY`]) =
/// `SHOULD_WAIT` (neblokujici socket), jinak `TIMED_OUT`.
pub fn expired(until: u64) -> Error {
    if until == netproto::TRY { Error::ShouldWait } else { Error::TimedOut }
}

/// IPv4 z adresy protokolu; IPv6 zatim = `UNREACHABLE`.
pub fn v4(ip: IpAddr) -> Result<Ipv4Addr> {
    match ip {
        IpAddr::V4(a) => Ok(a),
        IpAddr::V6(_) => Err(Error::Unreachable),
    }
}

/// Co prichozi filtr potrebuje vedet o portech.
struct PortsView<'a> {
    listening: &'a BTreeSet<u16>,
    ports: &'a BTreeSet<(PortProto, u16)>,
}

impl netfilter::Ports for PortsView<'_> {
    fn tcp_listening(&self, port: u16) -> bool {
        self.listening.contains(&port)
    }

    fn udp_bound(&self, port: u16) -> bool {
        self.ports.contains(&(PortProto::Udp, port))
    }
}

impl Stack {
    pub fn new(mut dev: Link, fw: Firewall, resolver: crate::forward::Forwarder) -> Self {
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
            recent: VecDeque::new(),
            recent_set: BTreeSet::new(),
            listening: BTreeSet::new(),
            fw,
            resolver,
            closing: Vec::new(),
        }
    }

    /// Precte ramce od ovladace; kazdy projde prichozim filtrem.
    /// `Err(PEER_CLOSED)` = ovladac skoncil.
    pub fn pump(&mut self) -> Result<()> {
        let now = now_ns();
        let view = PortsView {
            listening: &self.listening,
            ports: &self.ports,
        };
        let fw = &mut self.fw;
        let gateway = self.gateway;
        self.dev
            .pump(&mut |frame| fw.inbound(frame, &view, gateway, now))
    }

    pub fn listen_started(&mut self, port: u16) {
        self.listening.insert(port);
    }

    pub fn listen_stopped(&mut self, port: u16) {
        self.listening.remove(&port);
    }

    /// Zpracuje prijate ramce, casovace a odeslani; pak udalosti DHCP.
    pub fn poll(&mut self) {
        if core::mem::take(&mut self.dev.came_up) {
            // Znovu pripojena linka: adresu si vyzadat znovu (jiny kabel =
            // jina sit). Udalost Deconfigured zrusi starou adresu.
            self.sockets.get_mut::<dhcpv4::Socket>(self.dhcp).reset();
        }
        let now = instant(now_ns());
        self.iface.poll(now, &mut self.dev, &mut self.sockets);
        self.dhcp_events();
        self.fw.announce_network();
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
                self.fw.network_lost();
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
            unauthenticated_fallback: self.fw.unauthenticated_fallback,
        }
    }

    /// Mistni adresa IPv4 rozhrani (nenastavene = `UNREACHABLE`).
    pub fn local_ip(&self) -> Result<Ipv4Addr> {
        if !self.configured {
            return Err(Error::Unreachable);
        }
        self.iface.ipv4_addr().ok_or(Error::Unreachable)
    }

    /// Port pro naslouchani nebo socket UDP na mistni adrese `addr`:
    /// `0.0.0.0` (vsechna rozhrani) nebo adresa rozhrani, jinak
    /// `ADDRESS_NOT_AVAILABLE` (i 127.0.0.1 - loopback sluzba zatim nema).
    pub fn bind_port(&self, addr: SocketAddr) -> Result<u16> {
        match addr.ip() {
            IpAddr::V4(ip) if ip.is_unspecified() => Ok(addr.port()),
            IpAddr::V4(ip) if self.local_ip().ok() == Some(ip) => Ok(addr.port()),
            _ => Err(Error::AddressNotAvailable),
        }
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
        // Nejdriv port mimo karantenu; kdyz zadny neni, i z karanteny.
        for avoid_recent in [true, false] {
            for i in 0..span {
                let p = EPHEMERAL.start() + ((start + i) % span) as u16;
                if avoid_recent && self.recent_set.contains(&(proto, p)) {
                    continue;
                }
                if self.ports.insert((proto, p)) {
                    return Ok(p);
                }
            }
        }
        Err(Error::NoResources)
    }

    pub fn release_port(&mut self, proto: PortProto, port: u16) {
        self.ports.remove(&(proto, port));
        if EPHEMERAL.contains(&port) && self.recent_set.insert((proto, port)) {
            self.recent.push_back((proto, port));
            if self.recent.len() > PORT_QUARANTINE
                && let Some(old) = self.recent.pop_front()
            {
                self.recent_set.remove(&old);
            }
        }
        if proto == PortProto::Udp {
            self.fw.filter.forget_udp_port(port);
        }
    }

    /// Socket TCP s buffery `rx` a `tx` bajtu.
    pub fn new_tcp(&mut self, rx: usize, tx: usize) -> SocketHandle {
        let mut s = tcp::Socket::new(
            tcp::SocketBuffer::new(vec![0u8; rx]),
            tcp::SocketBuffer::new(vec![0u8; tx]),
        );
        // Udrzovani spojeni: mrtvou druhou stranu poznat do ~2 min.
        s.set_keep_alive(Some(SmolDuration::from_secs(60)));
        s.set_timeout(Some(SmolDuration::from_secs(120)));
        self.sockets.add(s)
    }

    /// Socket UDP s frontami `rx` a `tx` bajtu (nejvetsi datagram = fronta).
    pub fn new_udp(&mut self, rx: usize, tx: usize) -> SocketHandle {
        let rx = udp::PacketBuffer::new(
            vec![udp::PacketMetadata::EMPTY; udp_packets(rx)],
            vec![0u8; rx],
        );
        let tx = udp::PacketBuffer::new(
            vec![udp::PacketMetadata::EMPTY; udp_packets(tx)],
            vec![0u8; tx],
        );
        self.sockets.add(udp::Socket::new(rx, tx))
    }

    /// Odchozi datagram UDP: odpovedi z cile pak projdou filtrem.
    pub fn note_udp_out(&mut self, local_port: u16, to: SocketAddr) {
        if let IpAddr::V4(ip) = to.ip() {
            self.fw.filter.note_udp_out(local_port, ip, to.port(), now_ns());
        }
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
