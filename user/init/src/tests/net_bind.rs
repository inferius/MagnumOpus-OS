//! Navazany socket TCP (`TCP_BIND`, sada `sit`, `docs/cs/sit.md`, kapitola
//! Navazany socket TCP): port je obsazeny uz pri navazani jako v Linuxu.
//!
//! - navazani na port 0: mistni adresa hned zna port, `listen` ho necha,
//!   spojeni na nej projde,
//! - dve navazani na stejny port: druhe `ADDRESS_IN_USE`, po zavreni volno,
//! - spojeni z pevneho mistniho portu (loopback i sit k echo serveru
//!   hostitele): protejsek vidi ten port,
//! - prevedeni na miste: `LISTEN` pres jeden kanal, druhy kanal po `DUP` je
//!   pak naslouchani taky,
//! - relace s prijmem jen z loopbacku: navazani na sit s portem 0 ano,
//!   `listen` na nem `ACCESS_DENIED`, zvoleny port na siti uz pri navazani,
//! - UDP: navazani na port 0 zna port, druhe na stejny port
//!   `ADDRESS_IN_USE`.

use alloc::format;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};
use core::time::Duration;

use libos::{Error, Startup};
use netproto::{BindScope, BoundNow, Buffers, Deadline, Policy, TcpBound, TcpListener, rights};

use super::net::{configured, host, net, ports};
use super::{TestResult, fail};
use crate::{check, check_eq};

const LO: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const ANY: IpAddr = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
const T: Option<Duration> = Some(Duration::from_secs(10));

fn lo(port: u16) -> SocketAddr {
    SocketAddr::new(LO, port)
}

pub fn port_zero(_s: &Startup) -> TestResult {
    let net = net()?;
    let b = net.tcp_bind_addr(lo(0)).map_err(|e| fail("bind", e))?;
    let a = b.local_addr().map_err(|e| fail("adresa po bind", e))?;
    check!(a.ip() == LO && a.port() != 0, format!("adresa po bind {a}"));
    let l = b.listen(4, Buffers::DEFAULT).map_err(|e| fail("listen", e))?;
    check_eq!(l.local_addr().map_err(|e| fail("adresa po listen", e))?, a, "port po listen");
    let c = net.tcp_connect(a, T).map_err(|e| fail("spojeni", e))?;
    let (_s, from) = l.accept_timeout(T).map_err(|e| fail("accept", e))?;
    check_eq!(from, c.local_addr(), "adresa klienta");
    Ok(())
}

pub fn in_use(_s: &Startup) -> TestResult {
    let net = net()?;
    let b = net.tcp_bind_addr(lo(0)).map_err(|e| fail("bind", e))?;
    let a = b.local_addr().map_err(|e| fail("adresa", e))?;
    check_eq!(net.tcp_bind_addr(a).err(), Some(Error::AddressInUse), "druhy bind na stejny port");
    check_eq!(net.tcp_listen_addr(a, 1, Buffers::DEFAULT).err(), Some(Error::AddressInUse), "listen na obsazeny port");
    drop(b);
    let again = net.tcp_bind_addr(a).map_err(|e| fail("bind po zavreni", e))?;
    check_eq!(again.local_addr().map_err(|e| fail("adresa", e))?, a, "stejny port po zavreni");
    Ok(())
}

pub fn connect_from_port(s: &Startup) -> TestResult {
    let net = net()?;
    // Loopback.
    let l = net.tcp_listen_addr(lo(0), 2, Buffers::DEFAULT).map_err(|e| fail("listen", e))?;
    let target = l.local_addr().map_err(|e| fail("adresa", e))?;
    let b = net.tcp_bind_addr(lo(0)).map_err(|e| fail("bind", e))?;
    let mine = b.local_addr().map_err(|e| fail("adresa bind", e))?;
    let c = b.connect_ctx(target, Deadline::after(T), Buffers::DEFAULT, None).map_err(|e| fail("connect", e))?;
    check_eq!(c.local_addr().port(), mine.port(), "mistni port spojeni");
    let (srv, from) = l.accept_timeout(T).map_err(|e| fail("accept", e))?;
    check_eq!(from.port(), mine.port(), "port klienta u protejsku");
    c.write(b"x").map_err(|e| fail("zapis", e))?;
    let mut buf = [0u8; 4];
    check_eq!(srv.read(&mut buf).map_err(|e| fail("cteni", e))?, 1, "data");
    // Sit: echo server hostitele z pevneho portu (smoltcp).
    let Some(p) = ports(s, "spojeni z navazaneho portu na sit")? else {
        return Ok(());
    };
    configured(net)?;
    let b = net.tcp_bind_addr(SocketAddr::new(ANY, 0)).map_err(|e| fail("bind 0.0.0.0", e))?;
    let port = b.local_addr().map_err(|e| fail("adresa", e))?.port();
    let c = b.connect_ctx(host(p.echo), Deadline::after(T), Buffers::DEFAULT, None).map_err(|e| fail("connect na echo", e))?;
    check_eq!(c.local_addr().port(), port, "mistni port spojeni na sit");
    c.set_read_timeout(T);
    c.write(b"ping").map_err(|e| fail("zapis", e))?;
    let n = c.read(&mut buf).map_err(|e| fail("cteni echa", e))?;
    check!(n > 0 && buf[..n] == b"ping"[..n], format!("echo {:?}", &buf[..n]));
    Ok(())
}

pub fn convert_in_place(_s: &Startup) -> TestResult {
    let net = net()?;
    let b = net.tcp_bind_addr(lo(0)).map_err(|e| fail("bind", e))?;
    let a = b.local_addr().map_err(|e| fail("adresa", e))?;
    let other = TcpBound::from_channel(b.dup().map_err(|e| fail("DUP", e))?);
    check_eq!(other.local_addr().map_err(|e| fail("adresa DUP", e))?, a, "adresa pres druhy kanal");
    let _l = b.listen(2, Buffers::DEFAULT).map_err(|e| fail("listen", e))?;
    // Druhy kanal je ted naslouchani (`KIND`, jak to zjisti osrt).
    check!(matches!(other.current(), Ok(BoundNow::Listener(_))), "druh druheho kanalu po listen");
    let l2 = TcpListener::from_channel(other.dup().map_err(|e| fail("DUP po listen", e))?);
    drop(other);
    let c = net.tcp_connect(a, T).map_err(|e| fail("spojeni", e))?;
    let (_s, from) = l2.accept_timeout(T).map_err(|e| fail("accept pres druhy kanal", e))?;
    check_eq!(from, c.local_addr(), "adresa klienta");
    // Spojeni pres jeden kanal: druhy kanal vidi spojeni.
    let l = net.tcp_listen_addr(lo(0), 1, Buffers::DEFAULT).map_err(|e| fail("listen", e))?;
    let target = l.local_addr().map_err(|e| fail("adresa", e))?;
    let b = net.tcp_bind_addr(lo(0)).map_err(|e| fail("bind", e))?;
    let other = TcpBound::from_channel(b.dup().map_err(|e| fail("DUP", e))?);
    let _c = b.connect_ctx(target, Deadline::after(T), Buffers::DEFAULT, None).map_err(|e| fail("connect", e))?;
    match other.current() {
        Ok(BoundNow::Stream(s)) => check_eq!(s.peer_addr(), target, "protejsek pres druhy kanal"),
        _ => return Err(format!("druhy kanal po connect neni spojeni")),
    }
    Ok(())
}

pub fn scope(_s: &Startup) -> TestResult {
    let mut policy = Policy::unrestricted();
    policy.bind_scope = BindScope::Loopback;
    let app = net()?.restrict(rights::ALL, &policy).map_err(|e| fail("restrict", e))?;
    let b = app.tcp_bind_addr(SocketAddr::new(ANY, 0)).map_err(|e| fail("bind 0.0.0.0:0", e))?;
    check_eq!(b.listen(1, Buffers::DEFAULT).err(), Some(Error::AccessDenied), "listen na siti");
    check_eq!(app.tcp_bind_addr(SocketAddr::new(ANY, 47210)).err(), Some(Error::AccessDenied), "bind 0.0.0.0:47210");
    let b = app.tcp_bind_addr(lo(0)).map_err(|e| fail("bind 127.0.0.1:0", e))?;
    b.listen(1, Buffers::DEFAULT).map_err(|e| fail("listen na loopbacku", e))?;
    Ok(())
}

pub fn udp(_s: &Startup) -> TestResult {
    let net = net()?;
    let u = net.udp_bind_addr(lo(0), Buffers::DEFAULT).map_err(|e| fail("UDP bind", e))?;
    let a = u.local_addr().map_err(|e| fail("adresa", e))?;
    check!(a.port() != 0, format!("adresa UDP {a}"));
    check_eq!(net.udp_bind_addr(a, Buffers::DEFAULT).err(), Some(Error::AddressInUse), "druhy UDP bind");
    Ok(())
}
