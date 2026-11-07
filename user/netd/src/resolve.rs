//! Resolver DNS: dotazy A/AAAA pres UDP, opakovani a stridani serveru.
//!
//! Vlastni (ne smoltcp), aby slo zvolit server i port (testy, `dig
//! @server`) a aby odpoved systemoveho resolveru mohla relace pouzit pro
//! pravidla se jmenem hostitele (`Auth::learn`).
//!
//! TODO: mezipamet odpovedi podle TTL, dotaz pres TCP pri zkracene odpovedi,
//! DNSSEC/DoT/DoH (az s TLS).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::net::{IpAddr, SocketAddr};

use libos::{Error, Result};
use netproto::{ReadAddr, Reader, WriteAddr, Writer, dns, proto, rights};
use smoltcp::iface::SocketHandle;
use smoltcp::wire::IpAddress;

use crate::obj::{AuthRef, Pending, Step};
use crate::stack::{PortProto, Stack, deadline, now_ns, v4};
use crate::tcp::to_sa;

/// Opakovani dotazu (a prechod na dalsi server).
const RETRY_NS: u64 = 1_000_000_000;
/// Dotaz bez limitu se po teto dobe vzda.
const RESOLVE_MAX_NS: u64 = 30_000_000_000;

struct Query {
    id: u16,
    qtype: u16,
    done: bool,
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
}

impl Resolve {
    pub fn wake_at(&self) -> u64 {
        self.next_retry.min(self.deadline)
    }

    pub fn cancel(self, stack: &mut Stack) {
        stack.remove(self.sock);
        stack.release_port(PortProto::Udp, self.port);
    }

    fn send(&mut self, stack: &mut Stack) {
        let server = self.servers[self.server % self.servers.len()];
        let Ok(ip) = v4(server.ip()) else {
            return;
        };
        for q in self.queries.iter().filter(|q| !q.done) {
            if let Ok(msg) = dns::build_query(q.id, &self.name, q.qtype) {
                let _ = stack
                    .udp(self.sock)
                    .send_slice(&msg, (IpAddress::Ipv4(ip), server.port()));
            }
        }
        self.next_retry = now_ns().saturating_add(RETRY_NS);
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
    if !dns::valid_name(&name) {
        return Err(Error::InvalidArgs);
    }
    let qtypes: &[u16] = match family {
        0 => &[dns::TYPE_A, dns::TYPE_AAAA],
        4 => &[dns::TYPE_A],
        6 => &[dns::TYPE_AAAA],
        _ => return Err(Error::InvalidArgs),
    };
    let (servers, system) = match server {
        Some(s) => {
            if !auth.borrow().allows(proto::UDP, s) {
                return Err(Error::AccessDenied);
            }
            (vec![s], false)
        }
        None => (stack.dns_servers().to_vec(), true),
    };
    let servers: Vec<SocketAddr> = servers.into_iter().filter(|s| s.is_ipv4()).collect();
    if servers.is_empty() {
        return Err(Error::Unreachable);
    }
    stack.local_ip()?;
    let port = stack.take_port(PortProto::Udp, 0)?;
    let sock = stack.new_udp();
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
    };
    res.send(stack);
    Ok(Step::Wait(Pending::Resolve(res)))
}

pub fn progress(stack: &mut Stack, auth: &AuthRef, mut res: Resolve, now: u64) -> Step {
    let mut buf = [0u8; 1500];
    while let Ok((n, meta)) = stack.udp(res.sock).recv_slice(&mut buf) {
        let from = to_sa(meta.endpoint);
        if !res.servers.contains(&from) {
            continue;
        }
        for q in res.queries.iter_mut().filter(|q| !q.done) {
            match dns::parse_response(q.id, &buf[..n]) {
                Ok(ans) => {
                    q.done = true;
                    for a in ans.addrs {
                        if !res.addrs.contains(&a) {
                            res.addrs.push(a);
                        }
                    }
                    res.ttl = res.ttl.min(ans.ttl);
                }
                Err(Error::InvalidArgs) => {}
                Err(Error::NotFound) => {
                    q.done = true;
                    res.not_found += 1;
                }
                Err(e) => {
                    q.done = true;
                    res.error = Some(e);
                }
            }
        }
    }
    if res.queries.iter().all(|q| q.done) {
        let result = finish(auth, &res);
        res.cancel(stack);
        return Step::reply(result);
    }
    if now >= res.deadline {
        // Cast odpovedi (napr. A bez AAAA) staci.
        let result = if res.addrs.is_empty() {
            Err(Error::TimedOut)
        } else {
            finish(auth, &res)
        };
        res.cancel(stack);
        return Step::reply(result);
    }
    if now >= res.next_retry {
        res.server += 1;
        res.send(stack);
    }
    Step::Wait(Pending::Resolve(res))
}

fn finish(auth: &AuthRef, res: &Resolve) -> Result<Writer> {
    if res.addrs.is_empty() {
        return Err(if res.not_found > 0 || res.error.is_none() {
            Error::NotFound
        } else {
            res.error.unwrap_or(Error::NotFound)
        });
    }
    if res.system {
        auth.borrow_mut().learn(&res.name, &res.addrs);
    }
    let mut w = Writer::new();
    w.u16(res.addrs.len() as u16);
    for a in &res.addrs {
        w.ip(*a);
    }
    w.u32(if res.ttl == u32::MAX { 0 } else { res.ttl });
    Ok(w)
}
