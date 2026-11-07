//! Sit: DHCP, ping, DNS, TCP, UDP, opravneni relaci a NTP - pres slirp
//! (user-mode sit QEMU) proti pomocnym serverum, ktere spousti
//! `cargo xtask test` na hostiteli.
//!
//! Hostitel je z hosta videt jako 10.0.2.2 (slirp preklada na 127.0.0.1).
//! Porty pomocnych serveru predava xtask v souboru `net-test` v initfs
//! (`klic = port` na radek). Bez nej (`cargo xtask run`) testy, ktere
//! servery potrebuji, jen vypisou, ze se preskakuji.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};
use core::time::Duration;

use libos::initfs::Initfs;
use libos::{Error, Startup, println};
use netproto::io::{Read, Write};
use netproto::{Net, Policy, Rule, Target, proto, rights};

use super::{Test, TestResult, fail, net_service, time_service};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("DHCP: adresa, brana a DNS od slirp", dhcp),
    ("ping brany 10.0.2.2", ping),
    ("DNS: dotaz pres slirp na server hostitele", dns),
    ("TCP: spojeni a echo 256 KiB", tcp_echo),
    ("TCP: odmitnute spojeni -> CONNECTION_REFUSED", tcp_refused),
    ("TCP: preruseni spojeni (RST) -> CONNECTION_RESET", tcp_reset),
    ("TCP: naslouchani a prijeti spojeni z hostitele", tcp_accept),
    ("UDP: echo pres slirp", udp_echo),
    ("200 socketu najednou (driv limit 63)", many_sockets),
    ("opravneni: prava a omezeni cilu relace", restricted),
    ("opravneni: pravidlo se jmenem hostitele", host_rule),
    ("NTP: seridi hodiny pres sluzbu casu", ntp),
];

/// Hostitel ze site slirp.
const HOST: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 0, 2, 2));
/// Jmena, ktera zna pomocny server DNS (viz `xtask/src/nethelper.rs`).
const TEST_NAME: &str = "test.magnumopus";
const TEST_ADDR: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 0, 2, 77));
const ECHO_NAME: &str = "echo.magnumopus";

fn net() -> Result<&'static Net, String> {
    net_service()
        .map(|n| &n.root)
        .ok_or_else(|| String::from("sluzba site nebezi (chybi sitova karta?)"))
}

/// Porty pomocnych serveru z `net-test`.
struct Ports {
    echo: u16,
    udp_echo: u16,
    dns: u16,
    ntp: u16,
    closed: u16,
    /// Hlasi slirp odmitnuti spojeni na hostitele (RST)? Na Windows ne.
    refuses: bool,
    reset: u16,
    callback: u16,
    /// Port v hostovi, na ktery slirp presmeruje spojeni z hostitele
    /// (`hostfwd`).
    forwarded: u16,
}

fn ports(s: &Startup, test: &str) -> Result<Option<Ports>, String> {
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let Some(cfg) = fs.find("net-test") else {
        println!("[preskoceno] sit: {test}: chybi pomocne servery (jen `cargo xtask test`)");
        return Ok(None);
    };
    let text = core::str::from_utf8(cfg).map_err(|_| String::from("net-test neni UTF-8"))?;
    let get = |key: &str| -> Result<u16, String> {
        text.lines()
            .filter_map(|l| l.split_once('='))
            .find(|(k, _)| k.trim() == key)
            .and_then(|(_, v)| v.trim().parse().ok())
            .ok_or_else(|| format!("net-test: chybi {key}"))
    };
    Ok(Some(Ports {
        echo: get("echo")?,
        udp_echo: get("udp_echo")?,
        dns: get("dns")?,
        ntp: get("ntp")?,
        closed: get("closed")?,
        refuses: get("refuses")? != 0,
        reset: get("reset")?,
        callback: get("callback")?,
        forwarded: get("forwarded")?,
    }))
}

fn host(port: u16) -> SocketAddr {
    SocketAddr::new(HOST, port)
}

fn configured(net: &Net) -> Result<(), String> {
    net.wait_configured(Some(Duration::from_secs(15)))
        .map(|_| ())
        .map_err(|e| fail("DHCP", e))
}

fn dhcp(_: &Startup) -> TestResult {
    let net = net()?;
    let info = net
        .wait_configured(Some(Duration::from_secs(15)))
        .map_err(|e| fail("cekani na DHCP", e))?;
    check!(info.link_up, "linka neni pripojena");
    check!(info.mac != [0; 6], "MAC je nulova");
    check_eq!(info.mtu, 1500, "MTU");
    check_eq!(
        info.addr,
        Some((IpAddr::V4(Ipv4Addr::new(10, 0, 2, 15)), 24)),
        "adresa od slirp"
    );
    check_eq!(info.gateway, Some(HOST), "brana");
    check!(
        info.dns.contains(&IpAddr::V4(Ipv4Addr::new(10, 0, 2, 3))),
        "server DNS 10.0.2.3 od slirp"
    );
    check_eq!(info.rights, rights::ALL, "korenova relace ma vsechna prava");
    Ok(())
}

fn ping(_: &Startup) -> TestResult {
    let net = net()?;
    configured(net)?;
    for seq in 1..=3 {
        let rtt = net
            .ping(HOST, seq, 56, Duration::from_secs(5))
            .map_err(|e| fail("ping", e))?;
        check!(rtt < Duration::from_secs(5), "odezva");
    }
    // IPv6 sluzba zatim neumi.
    check_eq!(
        net.ping("::1".parse().unwrap(), 1, 8, Duration::from_secs(1)).err(),
        Some(Error::Unreachable),
        "ping na IPv6"
    );
    Ok(())
}

fn dns(s: &Startup) -> TestResult {
    let Some(p) = ports(s, "DNS")? else {
        return Ok(());
    };
    let net = net()?;
    configured(net)?;
    let server = Some(host(p.dns));
    let (addrs, ttl) = net
        .resolve_with(TEST_NAME, 4, server, Some(Duration::from_secs(5)))
        .map_err(|e| fail("dotaz", e))?;
    check_eq!(addrs, vec![TEST_ADDR], "adresy");
    check_eq!(ttl, 60, "TTL");
    // Obe rodiny: server ma jen A, AAAA vrati prazdnou odpoved.
    let (addrs, _) = net
        .resolve_with(TEST_NAME, 0, server, Some(Duration::from_secs(5)))
        .map_err(|e| fail("dotaz A + AAAA", e))?;
    check_eq!(addrs, vec![TEST_ADDR], "adresy (A + AAAA)");
    check_eq!(
        net.resolve_with("neexistuje.magnumopus", 4, server, Some(Duration::from_secs(5)))
            .err(),
        Some(Error::NotFound),
        "neexistujici jmeno"
    );
    check_eq!(
        net.resolve_with("spatne jmeno", 4, server, None).err(),
        Some(Error::InvalidArgs),
        "neplatne jmeno"
    );
    // Adresa misto jmena se neprekladana vrati hned.
    let (addrs, _) = net
        .resolve_with("10.0.2.2", 0, None, None)
        .map_err(|e| fail("adresa jako jmeno", e))?;
    check_eq!(addrs, vec![HOST], "adresa jako jmeno");
    Ok(())
}

/// Testovaci data: neperiodicky vzor, aby se poznalo prohozeni useku.
fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 + i / 251) as u8).collect()
}

fn tcp_echo(s: &Startup) -> TestResult {
    let Some(p) = ports(s, "TCP echo")? else {
        return Ok(());
    };
    let net = net()?;
    configured(net)?;
    let mut stream = net
        .tcp_connect(host(p.echo), Some(Duration::from_secs(10)))
        .map_err(|e| fail("spojeni", e))?;
    check_eq!(stream.peer_addr(), host(p.echo), "vzdalena adresa");
    check_eq!(
        stream.local_addr().ip(),
        IpAddr::V4(Ipv4Addr::new(10, 0, 2, 15)),
        "mistni adresa"
    );
    stream.set_read_timeout(Some(Duration::from_secs(10)));
    stream.set_write_timeout(Some(Duration::from_secs(10)));
    let data = pattern(256 * 1024);
    let mut back = vec![0u8; data.len()];
    // Po usecich: zapsat, precist zpet (echo server odpovida prubezne).
    for (chunk, out) in data.chunks(16 * 1024).zip(back.chunks_mut(16 * 1024)) {
        stream.write_all(chunk).map_err(|e| fail("zapis", e))?;
        stream.read_exact(out).map_err(|e| fail("cteni", e))?;
    }
    check!(back == data, "echo vratilo jina data");
    // Konec zapisu: server spojeni zavre, cteni vrati konec proudu.
    stream.shutdown_write().map_err(|e| fail("shutdown", e))?;
    let mut rest = [0u8; 16];
    check_eq!(stream.read(&mut rest).map_err(|e| fail("cteni konce", e))?, 0, "konec proudu");
    Ok(())
}

fn tcp_refused(s: &Startup) -> TestResult {
    let Some(p) = ports(s, "TCP odmitnuti")? else {
        return Ok(());
    };
    let net = net()?;
    configured(net)?;
    if p.refuses {
        let r = net.tcp_connect(host(p.closed), Some(Duration::from_secs(10)));
        check_eq!(r.err(), Some(Error::ConnectionRefused), "spojeni na zavreny port");
    } else {
        // Slirp na Windows odmitnuti neohlasi: SYN zustane bez odpovedi
        // a spojeni vyprsi (overi aspon timeout navazovani).
        println!("init: slirp na tomto hostiteli odmitnuti spojeni nehlasi, overuji vyprseni");
        let r = net.tcp_connect(host(p.closed), Some(Duration::from_secs(1)));
        check_eq!(r.err(), Some(Error::TimedOut), "spojeni na zavreny port (bez RST)");
    }
    check_eq!(
        net.tcp_connect(host(0), None).err(),
        Some(Error::InvalidArgs),
        "port 0"
    );
    Ok(())
}

fn tcp_reset(s: &Startup) -> TestResult {
    let Some(p) = ports(s, "TCP preruseni")? else {
        return Ok(());
    };
    let net = net()?;
    configured(net)?;
    let mut stream = net
        .tcp_connect(host(p.reset), Some(Duration::from_secs(10)))
        .map_err(|e| fail("spojeni", e))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)));
    stream.write_all(b"data, ktera nikdo neprecte").map_err(|e| fail("zapis", e))?;
    let mut b = [0u8; 16];
    check_eq!(stream.read(&mut b).err(), Some(Error::ConnectionReset), "cteni po RST");
    check_eq!(stream.write(b"x").err(), Some(Error::ConnectionReset), "zapis po RST");
    Ok(())
}

fn tcp_accept(s: &Startup) -> TestResult {
    let Some(p) = ports(s, "TCP naslouchani")? else {
        return Ok(());
    };
    let net = net()?;
    configured(net)?;
    let listener = net
        .tcp_listen(p.forwarded, 2)
        .map_err(|e| fail("naslouchani", e))?;
    check_eq!(listener.local_addr().port(), p.forwarded, "port naslouchani");
    // Stejny port podruhe nejde.
    check_eq!(
        net.tcp_listen(p.forwarded, 1).err(),
        Some(Error::AddressInUse),
        "obsazeny port"
    );
    // Pozadat hostitele, at se pripoji zpet (pres hostfwd slirp).
    let mut ctl = net
        .tcp_connect(host(p.callback), Some(Duration::from_secs(10)))
        .map_err(|e| fail("spojeni na callback", e))?;
    ctl.write_all(b"connect\n").map_err(|e| fail("zapis callback", e))?;
    let (mut conn, remote) = listener
        .accept_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| fail("accept", e))?;
    check_eq!(remote.ip(), HOST, "spojeni prislo od hostitele (slirp)");
    conn.set_read_timeout(Some(Duration::from_secs(10)));
    let mut got = Vec::new();
    conn.read_to_end(&mut got).map_err(|e| fail("cteni", e))?;
    check_eq!(got.as_slice(), b"ahoj z hostitele\n", "data od hostitele");
    // Bez dalsiho spojeni accept vyprsi.
    check_eq!(
        listener
            .accept_timeout(Some(Duration::from_millis(200)))
            .err(),
        Some(Error::TimedOut),
        "accept bez spojeni"
    );
    Ok(())
}

fn udp_echo(s: &Startup) -> TestResult {
    let Some(p) = ports(s, "UDP")? else {
        return Ok(());
    };
    let net = net()?;
    configured(net)?;
    let sock = net.udp_bind(0).map_err(|e| fail("bind", e))?;
    check!(sock.local_addr().port() >= 49152, "docasny port");
    let mut buf = [0u8; 2048];
    for i in 0..3u8 {
        let msg = pattern(1000 + i as usize * 100);
        // UDP muze ztratit datagram: az tri pokusy.
        let mut ok = false;
        for _ in 0..3 {
            sock.send_to(&msg, host(p.udp_echo)).map_err(|e| fail("send_to", e))?;
            match sock.recv_from(&mut buf, Some(Duration::from_secs(2))) {
                Ok((n, from)) => {
                    check_eq!(from, host(p.udp_echo), "odesilatel");
                    check!(&buf[..n] == msg.as_slice(), "echo vratilo jina data");
                    ok = true;
                    break;
                }
                Err(Error::TimedOut) => continue,
                Err(e) => return Err(fail("recv_from", e)),
            }
        }
        check!(ok, "UDP echo neodpovedelo");
    }
    check_eq!(
        sock.recv_from(&mut buf, Some(Duration::from_millis(100))).err(),
        Some(Error::TimedOut),
        "prazdna fronta"
    );
    Ok(())
}

fn many_sockets(s: &Startup) -> TestResult {
    let Some(p) = ports(s, "mnoho socketu")? else {
        return Ok(());
    };
    let net = net()?;
    configured(net)?;
    let mut socks = Vec::new();
    for i in 0..200 {
        socks.push(net.udp_bind(0).map_err(|e| fail(&format!("bind {i}"), e))?);
    }
    // Posledni i prvni socket funguji (sluzba ceka na vsechny kanaly).
    let mut buf = [0u8; 64];
    for sock in [socks.last().unwrap(), &socks[0]] {
        let mut ok = false;
        for _ in 0..3 {
            sock.send_to(b"mnoho", host(p.udp_echo)).map_err(|e| fail("send_to", e))?;
            if let Ok((n, _)) = sock.recv_from(&mut buf, Some(Duration::from_secs(2))) {
                check_eq!(&buf[..n], b"mnoho", "echo");
                ok = true;
                break;
            }
        }
        check!(ok, "UDP echo z jednoho z 200 socketu");
    }
    drop(socks);
    // Po zavreni jdou sockety otevrit znovu (sloty a porty se uvolnily).
    let again = net.udp_bind(0).map_err(|e| fail("bind po uvolneni", e))?;
    drop(again);
    Ok(())
}

fn restricted(s: &Startup) -> TestResult {
    let Some(p) = ports(s, "opravneni")? else {
        return Ok(());
    };
    let root = net()?;
    configured(root)?;
    // Program smi jen TCP na echo server hostitele.
    let policy = Policy::only(vec![Rule::port(
        proto::TCP,
        Target::Net {
            addr: HOST,
            prefix: 32,
        },
        p.echo,
    )]);
    let app = root
        .restrict(rights::CLIENT, &policy)
        .map_err(|e| fail("restrict", e))?;
    let info = app.info().map_err(|e| fail("info", e))?;
    check_eq!(info.rights, rights::CLIENT, "prava relace");
    check_eq!(
        app.ping(HOST, 1, 8, Duration::from_secs(1)).err(),
        Some(Error::AccessDenied),
        "ping bez prava PING"
    );
    check_eq!(
        app.tcp_listen(0, 1).err(),
        Some(Error::AccessDenied),
        "naslouchani bez prava"
    );
    check_eq!(
        app.restrict(rights::ALL, &Policy::unrestricted()).err(),
        Some(Error::AccessDenied),
        "dalsi relace bez prava RESTRICT"
    );
    check_eq!(
        app.set_dns(&[host(p.dns)]).err(),
        Some(Error::AccessDenied),
        "nastaveni DNS bez prava CONFIGURE"
    );
    check_eq!(
        app.tcp_connect(host(p.closed), Some(Duration::from_secs(5))).err(),
        Some(Error::AccessDenied),
        "port mimo omezeni"
    );
    let mut s = app
        .tcp_connect(host(p.echo), Some(Duration::from_secs(10)))
        .map_err(|e| fail("povolene spojeni", e))?;
    s.set_read_timeout(Some(Duration::from_secs(10)));
    s.write_all(b"ping").map_err(|e| fail("zapis", e))?;
    let mut b = [0u8; 4];
    s.read_exact(&mut b).map_err(|e| fail("cteni", e))?;
    check_eq!(&b, b"ping", "echo v omezene relaci");
    // UDP pravo ma, ale cil mimo omezeni.
    let u = app.udp_bind(0).map_err(|e| fail("udp_bind", e))?;
    check_eq!(
        u.send_to(b"x", host(p.udp_echo)).err(),
        Some(Error::AccessDenied),
        "UDP mimo omezeni"
    );
    // Vlastni server DNS je cil UDP - taky mimo omezeni.
    check_eq!(
        app.resolve_with(TEST_NAME, 4, Some(host(p.dns)), None).err(),
        Some(Error::AccessDenied),
        "DNS na vlastni server mimo omezeni"
    );
    // Socket lze predat jinam: kanal spojeni z jine relace funguje dal.
    let stream = netproto::TcpStream::from_channel(s.into_channel()).map_err(|e| fail("predani", e))?;
    check_eq!(stream.peer_addr(), host(p.echo), "predane spojeni");
    Ok(())
}

fn host_rule(s: &Startup) -> TestResult {
    let Some(p) = ports(s, "pravidlo se jmenem")? else {
        return Ok(());
    };
    let root = net()?;
    configured(root)?;
    // Systemovy resolver docasne na pomocny server hostitele.
    root.set_dns(&[host(p.dns)]).map_err(|e| fail("set_dns", e))?;
    let result = host_rule_inner(root, &p);
    root.set_dns(&[]).map_err(|e| fail("set_dns zpet", e))?;
    result
}

fn host_rule_inner(root: &Net, p: &Ports) -> TestResult {
    let policy = Policy::only(vec![
        Rule::port(proto::TCP, Target::Host(String::from("*.magnumopus")), p.echo),
        // Vlastni server DNS povoleny jako cil UDP.
        Rule::port(
            proto::UDP,
            Target::Net {
                addr: HOST,
                prefix: 32,
            },
            p.dns,
        ),
    ]);
    // Relace, ktera se pta vlastniho serveru: preklad se nepocita.
    let own = root
        .restrict(rights::CLIENT, &policy)
        .map_err(|e| fail("restrict", e))?;
    let (a, _) = own
        .resolve_with(ECHO_NAME, 4, Some(host(p.dns)), None)
        .map_err(|e| fail("dotaz na vlastni server", e))?;
    check_eq!(a, vec![HOST], "preklad");
    check_eq!(
        own.tcp_connect(host(p.echo), Some(Duration::from_secs(5))).err(),
        Some(Error::AccessDenied),
        "preklad od vlastniho serveru pravidlo neplni"
    );
    // Systemovy resolver: po prekladu je adresa povolena.
    let app = root
        .restrict(rights::CLIENT, &policy)
        .map_err(|e| fail("restrict", e))?;
    check_eq!(
        app.tcp_connect(host(p.echo), Some(Duration::from_secs(5))).err(),
        Some(Error::AccessDenied),
        "pred prekladem"
    );
    let a = app.resolve(ECHO_NAME).map_err(|e| fail("systemovy preklad", e))?;
    check_eq!(a, vec![HOST], "systemovy preklad");
    let stream = app
        .tcp_connect(host(p.echo), Some(Duration::from_secs(10)))
        .map_err(|e| fail("spojeni po prekladu", e))?;
    drop(stream);
    check_eq!(
        app.tcp_connect(host(p.closed), Some(Duration::from_secs(5))).err(),
        Some(Error::AccessDenied),
        "jiny port nez v pravidle"
    );
    Ok(())
}

fn ntp(s: &Startup) -> TestResult {
    let Some(p) = ports(s, "NTP")? else {
        return Ok(());
    };
    let net = net()?;
    configured(net)?;
    let time = time_service().ok_or("sluzba casu nebezi")?;
    let offset = crate::system::sync_time_ntp(net, time, host(p.ntp)).map_err(|e| fail("NTP", e))?;
    // Hodiny hosta jdou z RTC QEMU = cas hostitele; server NTP je tentyz.
    check!(
        offset.abs() < 10_000_000_000,
        format!("posun {} ms je prilis velky", offset / 1_000_000)
    );
    let r = libos::time::wall_reading().map_err(|e| fail("cteni casu", e))?;
    check_eq!(r.source, libos::time::source::SET, "zdroj casu po NTP");
    Ok(())
}
