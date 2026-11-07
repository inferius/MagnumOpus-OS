//! Mezipamet odpovedi: (jmeno, typ) -> stav, adresy, platnost.
//!
//! TTL podle odpovedi (zaporne podle SOA, RFC 2308), nejvys 1 den,
//! nejmene 5 s. Podvrzene a nedostupne odpovedi se neukladaji.

use alloc::vec::Vec;
use core::net::IpAddr;

use dns::Name;
use netproto::resolver::status;

const MAX_ENTRIES: usize = 1024;
const MAX_TTL: u32 = 86_400;
const MIN_TTL: u32 = 5;

struct Entry {
    name: Name,
    qtype: u16,
    status: u8,
    addrs: Vec<IpAddr>,
    secure: bool,
    /// Realny cas, kdy prestane platit.
    until: u32,
    ttl: u32,
}

#[derive(Default)]
pub struct Cache {
    entries: Vec<Entry>,
}

impl Cache {
    /// Platny zaznam; TTL zbyvajici do konce platnosti.
    pub fn get(&self, name: &Name, qtype: u16, now: u32) -> Option<(u8, Vec<IpAddr>, u32, bool)> {
        let e = self.entries.iter().find(|e| e.qtype == qtype && e.name == *name)?;
        let left = e.until.checked_sub(now).filter(|&l| l > 0)?;
        Some((e.status, e.addrs.clone(), left.min(e.ttl), e.secure))
    }

    pub fn put(&mut self, name: &Name, qtype: u16, r: &(u8, Vec<IpAddr>, u32, bool), now: u32) {
        if !matches!(r.0, status::OK | status::NXDOMAIN | status::NODATA) {
            return;
        }
        let ttl = r.2.clamp(MIN_TTL, MAX_TTL);
        self.entries.retain(|e| !(e.qtype == qtype && e.name == *name) && e.until > now);
        if self.entries.len() >= MAX_ENTRIES {
            self.entries.remove(0);
        }
        self.entries.push(Entry {
            name: name.clone(),
            qtype,
            status: r.0,
            addrs: r.1.clone(),
            secure: r.3,
            until: now.saturating_add(ttl),
            ttl,
        });
    }
}
