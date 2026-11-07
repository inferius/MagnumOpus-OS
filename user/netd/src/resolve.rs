//! Resolver DNS: dotazy A/AAAA pres UDP (EDNS0), opakovani a stridani
//! serveru, pri zkracene odpovedi dotaz pres TCP (RFC 7766).
//!
//! Vlastni (ne smoltcp), aby slo zvolit server i port (testy, `dig
//! @server`) a aby odpoved systemoveho resolveru mohla relace pouzit pro
//! pravidla se jmenem hostitele (`Auth::learn`, platnost podle TTL).
//! Jmena na seznamu blokovanych domen firewallu = `ACCESS_DENIED`.
//!
//! TODO: mezipamet odpovedi podle TTL, DNSSEC/DoT/DoH (az s TLS).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::net::{IpAddr, SocketAddr};

use libos::{Error, Result};
use netproto::auth::Usage;
use netproto::fw::audit;
use netproto::{ReadAddr, Reader, WriteAddr, Writer, dns, proto, rights};
use smoltcp::iface::SocketHandle;
use smoltcp::socket::tcp::State;
use smoltcp::wire::IpAddress;

use crate::fw::app_name;
use crate::obj::{AuthRef, Deferred, Gate, Pending, Step, ask_wait, gate};
use crate::stack::{PortProto, Stack, deadline, now_ns, v4};
use crate::tcp::to_sa;

/// Opakovani dotazu (a prechod na dalsi server).
const RETRY_NS: u64 = 1_000_000_000;
/// Dotaz bez limitu se po teto dobe vzda.
const RESOLVE_MAX_NS: u64 = 30_000_000_000;
/// Fronty socketu UDP resolveru (odpoved az `dns::EDNS_UDP`).
const UDP_RX: usize = 4 * dns::EDNS_UDP;
const UDP_TX: usize = 2048;
/// Buffery spojeni TCP pro zkracene odpovedi (zprava DNS az 64 KiB).
const TCP_RX: usize = 66 * 1024;
const TCP_TX: usize = 2048;

struct Query {
    id: u16,
    qtype: u16,
    done: bool,
    /// Odpoved byla zkracena: zeptat se pres TCP.
    via_tcp: bool,
}

/// Dotazy pres TCP u jednoho serveru.
struct TcpDns {
    sock: SocketHandle,
    port: u16,
    /// Dotazy s delkou u16 na zacatku, jeste neodeslane.
    out: Vec<u8>,
    inbuf: Vec<u8>,
    _usage: Usage,
}

pub struct Resolve {
    sock: SocketHandle,
    port: u16,
    name: String,
    queries: Vec<Query>,
    servers: Vec<SocketAddr>,
    server: usize,
    next_retry: u64,
    deadline: u64,
    addrs: Vec<IpAddr>,
    ttl: u32,
    not_found: usize,
    error: Option<Error>,
    /// Systemove servery (odpoved smi relace pouzit pro pravidla se jmenem).
    system: bool,
    tcp: Option<TcpDns>,
    _usage: Usage,
}

/// Parametry `RESOLVE` s vlastnim serverem (ceka na rozhodnuti uzivatele).
pub struct ResolveReq {
    name: String,
    family: u8,
    server: SocketAddr,
    timeout: u64,
}

impl Resolve {
    pub fn wake_at(&self) -> u64 {
        self.next_retry.min(self.deadline)
    }

    pub fn cancel(mut self, stack: &mut Stack) {
        self.close_tcp(stack);
        stack.remove(self.sock);
        stack.release_port(PortProto::Udp, self.port);
    }

    fn close_tcp(&mut self, stack: &mut Stack) {
        if let Some(t) = self.tcp.take() {
            stack.tcp(t.sock).abort();
            stack.remove(t.sock);
            stack.release_port(PortProto::Tcp, t.port);
        }
    }

    fn current_server(&self) -> SocketAddr {
        self.servers[self.server % self.servers.len()]
    }

    fn send(&mut self, stack: &mut Stack) {
        let server = self.current_server();
        let Ok(ip) = v4(server.ip()) else {
            return;
        };
        for q in self.queries.iter().filter(|q| !q.done && !q.via_tcp) {
            if let Ok(msg) = dns::build_query(q.id, &self.name, q.qtype) {
                let _ = stack
                    .udp(self.sock)
                    .send_slice(&msg, (IpAddress::Ipv4(ip), server.port()));
            }
        }
        stack.note_udp_out(self.port, server);
        self.next_retry = now_ns().saturating_add(RETRY_NS);
    }

    /// Zalozi spojeni TCP k serveru pro zkracene dotazy.
    fn start_tcp(&mut self, stack: &mut Stack, auth: &AuthRef) -> Result<()> {
        let server = self.current_server();
        let ip = v4(server.ip())?;
        let usage = auth.borrow().charge((TCP_RX + TCP_TX) as u64)?;
        let port = stack.take_port(PortProto::Tcp, 0)?;
        let sock = stack.new_tcp(TCP_RX, TCP_TX);
        let cx = stack.iface.context();
        if stack
            .sockets
            .get_mut::<smoltcp::socket::tcp::Socket>(sock)
            .connect(cx, (IpAddress::Ipv4(ip), server.port()), port)
            .is_err()
        {
            stack.remove(sock);
            stack.release_port(PortProto::Tcp, port);
            return Err(Error::Unreachable);
        }
        let mut out = Vec::new();
        for q in self.queries.iter().filter(|q| !q.done && q.via_tcp) {
            if let Ok(msg) = dns::build_query(q.id, &self.name, q.qtype) {
                out.extend_from_slice(&(msg.len() as u16).to_be_bytes());
                out.extend_from_slice(&msg);
            }
        }
        self.tcp = Some(TcpDns {
            sock,
            port,
            out,
            inbuf: Vec::new(),
            _usage: usage,
        });
        Ok(())
    }

    /// Odesle a precte, co jde pres TCP; zpracuje cele odpovedi.
    fn poll_tcp(&mut self, stack: &mut Stack) {
        let Some(t) = &mut self.tcp else {
            return;
        };
        let sock = stack.tcp(t.sock);
        if !t.out.is_empty()
            && sock.may_send()
            && let Ok(n) = sock.send_slice(&t.out)
        {
            t.out.drain(..n);
        }
        let mut buf = [0u8; 4096];
        while sock.can_recv() {
            match sock.recv_slice(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => t.inbuf.extend_from_slice(&buf[..n]),
            }
        }
        let closed = matches!(sock.state(), State::Closed | State::CloseWait | State::TimeWait);
        let mut msgs = Vec::new();
        while t.inbuf.len() >= 2 {
            let len = u16::from_be_bytes([t.inbuf[0], t.inbuf[1]]) as usize;
            if t.inbuf.len() < 2 + len {
                break;
            }
            msgs.push(t.inbuf[2..2 + len].to_vec());
            t.inbuf.drain(..2 + len);
        }
        for m in msgs {
            self.handle_response(&m, true);
        }
        if closed && self.queries.iter().any(|q| !q.done && q.via_tcp) {
            // Server spojeni zavrel bez odpovedi.
            for q in self.queries.iter_mut().filter(|q| !q.done && q.via_tcp) {
                q.done = true;
            }
            self.error.get_or_insert(Error::IoError);
        }
    }

    fn handle_response(&mut self, msg: &[u8], via_tcp: bool) {
        for q in self.queries.iter_mut().filter(|q| !q.done) {
            match dns::parse_response(q.id, msg) {
                Ok(ans) => {
                    q.done = true;
                    for a in ans.addrs {
                        if !self.addrs.contains(&a) {
                            self.addrs.push(a);
                        }
                    }
                    self.ttl = self.ttl.min(ans.ttl);
                }
                Err(Error::InvalidArgs) => {}
                Err(Error::BufferTooSmall) if !via_tcp => q.via_tcp = true,
                Err(Error::NotFound) => {
                    q.done = true;
                    self.not_found += 1;
                }
                Err(e) => {
                    q.done = true;
                    self.error = Some(e);
                }
            }
        }
    }
}

/// `RESOLVE` na relaci.
pub fn start(stack: &mut Stack, auth: &AuthRef, r: &mut Reader) -> Result<Step> {
    let name = r.str()?;
    let family = r.u8()?;
    let server = match r.u8()? {
        0 => None,
        _ => Some(r.socket_addr()?),
    };
    let timeout = r.u64()?;
    auth.borrow().require(rights::RESOLVE)?;
    // Adresa misto jmena (IPv4 i IPv6): vratit hned, jako getaddrinfo.
    // Musi byt pred kontrolou jmena - `::1` neni platne jmeno hostitele.
    if let Ok(ip) = name.parse::<IpAddr>() {
        let fits = match family {
            0 => true,
            4 => ip.is_ipv4(),
            6 => ip.is_ipv6(),
            _ => return Err(Error::InvalidArgs),
        };
        if !fits {
            return Err(Error::NotFound);
        }
        let mut w = Writer::new();
        w.u16(1).ip(ip).u32(0);
        return Ok(Step::reply(Ok(w)));
    }
    if !dns::valid_name(&name) || family > 6 || !matches!(family, 0 | 4 | 6) {
        return Err(Error::InvalidArgs);
    }
    if stack.fw.dns_blocked(&name) {
        let app = app_name(&auth.borrow());
        stack.fw.record(audit::DNS_BLOCKED, &app, proto::UDP, None, 0, &name);
        return Err(Error::AccessDenied);
    }
    match server {
        Some(server) => start_server(
            stack,
            auth,
            ResolveReq {
                name,
                family,
                server,
                timeout,
            },
            Vec::new(),
        ),
        None => {
            let servers = stack.dns_servers().to_vec();
            begin(stack, auth, name, family, servers, true, timeout)
        }
    }
}

/// Dotaz na vlastni server po kontrole cile (server musi povolit relace
/// jako cil UDP; pres TCP pak smi na stejny server a port).
pub fn start_server(stack: &mut Stack, auth: &AuthRef, req: ResolveReq, granted: Vec<String>) -> Result<Step> {
    let server = req.server;
    if let Gate::Ask(id, app) = gate(stack, &auth.borrow(), proto::UDP, server, &granted)? {
        return Ok(ask_wait(id, app, proto::UDP, server, granted, Deferred::Resolve(req)));
    }
    begin(stack, auth, req.name, req.family, vec![server], false, req.timeout)
}

fn begin(
    stack: &mut Stack,
    auth: &AuthRef,
    name: String,
    family: u8,
    servers: Vec<SocketAddr>,
    system: bool,
    timeout: u64,
) -> Result<Step> {
    let qtypes: &[u16] = match family {
        0 => &[dns::TYPE_A, dns::TYPE_AAAA],
        4 => &[dns::TYPE_A],
        _ => &[dns::TYPE_AAAA],
    };
    let servers: Vec<SocketAddr> = servers.into_iter().filter(|s| s.is_ipv4()).collect();
    if servers.is_empty() {
        return Err(Error::Unreachable);
    }
    stack.local_ip()?;
    let usage = auth.borrow().charge((UDP_RX + UDP_TX) as u64)?;
    let port = stack.take_port(PortProto::Udp, 0)?;
    let sock = stack.new_udp(UDP_RX, UDP_TX);
    if stack.udp(sock).bind(port).is_err() {
        stack.remove(sock);
        stack.release_port(PortProto::Udp, port);
        return Err(Error::NoResources);
    }
    let queries = qtypes
        .iter()
        .map(|&qtype| Query {
            id: stack.rng.next_u16(),
            qtype,
            done: false,
            via_tcp: false,
        })
        .collect();
    let mut res = Resolve {
        sock,
        port,
        name: netproto::policy::normalize_name(&name),
        queries,
        servers,
        server: 0,
        next_retry: 0,
        deadline: deadline(timeout).min(now_ns().saturating_add(RESOLVE_MAX_NS)),
        addrs: Vec::new(),
        ttl: u32::MAX,
        not_found: 0,
        error: None,
        system,
        tcp: None,
        _usage: usage,
    };
    res.send(stack);
    Ok(Step::Wait(Pending::Resolve(res)))
}

pub fn progress(stack: &mut Stack, auth: &AuthRef, mut res: Resolve, now: u64) -> Step {
    let mut buf = vec![0u8; dns::EDNS_UDP];
    while let Ok((n, meta)) = stack.udp(res.sock).recv_slice(&mut buf) {
        let from = to_sa(meta.endpoint);
        if !res.servers.contains(&from) {
            continue;
        }
        res.handle_response(&buf[..n], false);
    }
    let need_tcp = res.queries.iter().any(|q| !q.done && q.via_tcp);
    if need_tcp && res.tcp.is_none() && let Err(e) = res.start_tcp(stack, auth) {
        for q in res.queries.iter_mut().filter(|q| !q.done && q.via_tcp) {
            q.done = true;
        }
        res.error = Some(e);
    }
    res.poll_tcp(stack);
    if res.queries.iter().all(|q| q.done) {
        let result = finish(auth, &res, now);
        res.cancel(stack);
        return Step::reply(result);
    }
    if now >= res.deadline {
        // Cast odpovedi (napr. A bez AAAA) staci.
        let result = if res.addrs.is_empty() {
            Err(Error::TimedOut)
        } else {
            finish(auth, &res, now)
        };
        res.cancel(stack);
        return Step::reply(result);
    }
    if now >= res.next_retry && res.tcp.is_none() {
        res.server += 1;
        res.send(stack);
    }
    Step::Wait(Pending::Resolve(res))
}

fn finish(auth: &AuthRef, res: &Resolve, now: u64) -> Result<Writer> {
    if res.addrs.is_empty() {
        return Err(if res.not_found > 0 || res.error.is_none() {
            Error::NotFound
        } else {
            res.error.unwrap_or(Error::NotFound)
        });
    }
    let ttl = if res.ttl == u32::MAX { 0 } else { res.ttl };
    if res.system {
        auth.borrow_mut().learn(&res.name, &res.addrs, ttl, now);
    }
    let mut w = Writer::new();
    w.u16(res.addrs.len() as u16);
    for a in &res.addrs {
        w.ip(*a);
    }
    w.u32(ttl);
    Ok(w)
}
