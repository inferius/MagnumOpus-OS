//! Podepsane testovaci zony a odpovidac (feature `testzone`): testy
//! validatoru a pomocny server DoH/DoT v xtask. Neni pro provoz.
//!
//! Hierarchie (vse ECDSA P-256, podpisy deterministicke RFC 6979):
//! - koren `.` (kotva = DS jeho klice), NSEC,
//! - `test.` (DS v korenu), NSEC; delegace `unsigned.test.` bez DS
//!   (nepodepsana zona) a `bad.test.` s DS jineho klice (podvrzena),
//! - `example.test.` (DS v `test.`), NSEC3 (0 iteraci, bez soli):
//!   `www` A, `alias` CNAME na `www`, `*.wild` A, `v6` AAAA.
//!
//! Odpovidac se chova jako rekurzivni resolver s DO+CD: vrati data
//! z nejhlubsi zony, DS z nadrazene, s RRSIG a dukazy neexistence.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use p256::ecdsa::SigningKey;
use p256::ecdsa::signature::hazmat::PrehashSigner;
use sha1::Sha1;
use sha2::{Digest, Sha256};

use crate::dnssec::{Dnskey, alg, digest};
use crate::message::{CLASS_IN, Message, Question, Record, flags, rcode};
use crate::name::Name;
use crate::rtype;

/// Platnost podpisu (2020 - 2050; okno seriove aritmetiky RFC 1982 je 68 let).
const INCEPTION: u32 = 1_577_836_800;
const EXPIRATION: u32 = 2_524_608_000;
const TTL: u32 = 300;

pub fn name(s: &str) -> Name {
    Name::parse(s).expect("platne jmeno")
}

fn record(owner: &Name, t: u16, rdata: Vec<u8>) -> Record {
    Record {
        name: owner.clone(),
        rtype: t,
        class: CLASS_IN,
        ttl: TTL,
        rdata,
    }
}

pub fn a(owner: &str, ip: [u8; 4]) -> Record {
    record(&name(owner), rtype::A, ip.to_vec())
}

fn bitmap(types: &[u16]) -> Vec<u8> {
    let mut t = types.to_vec();
    t.sort();
    t.dedup();
    let mut out = Vec::new();
    let mut i = 0;
    while i < t.len() {
        let w = (t[i] >> 8) as u8;
        let mut bits = [0u8; 32];
        let mut max = 0;
        while i < t.len() && (t[i] >> 8) as u8 == w {
            let b = (t[i] & 0xFF) as usize;
            bits[b / 8] |= 0x80 >> (b % 8);
            max = max.max(b / 8 + 1);
            i += 1;
        }
        out.push(w);
        out.push(max as u8);
        out.extend_from_slice(&bits[..max]);
    }
    out
}

/// Zona: klic, zaznamy, druh dukazu neexistence.
pub struct Zone {
    pub origin: Name,
    key: Option<SigningKey>,
    pub records: Vec<Record>,
    nsec3: bool,
}

fn base32hex(data: &[u8]) -> String {
    const A: &[u8] = b"0123456789abcdefghijklmnopqrstuv";
    let mut out = String::new();
    let (mut acc, mut bits) = (0u32, 0);
    for &b in data {
        acc = acc << 8 | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(A[(acc >> bits & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(A[(acc << (5 - bits) & 31) as usize] as char);
    }
    out
}

fn nsec3_hash(n: &Name) -> Vec<u8> {
    Sha1::digest(n.to_wire(true)).to_vec()
}

impl Zone {
    /// Zona `origin` s klicem ze semene `seed` (nebo nepodepsana).
    pub fn new(origin: &str, seed: Option<u8>, nsec3: bool) -> Self {
        let key = seed.map(|s| SigningKey::from_bytes(&[s; 32].into()).expect("klic"));
        let mut z = Self {
            origin: name(origin),
            key,
            records: Vec::new(),
            nsec3,
        };
        let soa = {
            let under = |l: &str| if origin == "." { name(l) } else { name(&alloc::format!("{l}.{origin}")) };
            let mut rd = under("ns").to_wire(false);
            rd.extend(under("hostmaster").to_wire(false));
            for v in [1u32, 3600, 600, 86400, TTL] {
                rd.extend_from_slice(&v.to_be_bytes());
            }
            rd
        };
        let o = z.origin.clone();
        z.records.push(record(&o, rtype::SOA, soa));
        if let Some(k) = z.dnskey() {
            z.records.push(k);
        }
        z
    }

    /// DNSKEY zony (KSK+ZSK v jednom, priznak 257).
    pub fn dnskey(&self) -> Option<Record> {
        let k = self.key.as_ref()?;
        let point = k.verifying_key().to_encoded_point(false);
        let mut rd = vec![1, 1, 3, alg::ECDSAP256SHA256];
        rd.extend_from_slice(&point.as_bytes()[1..]);
        Some(record(&self.origin, rtype::DNSKEY, rd))
    }

    /// DS teto zony (do nadrazene).
    pub fn ds(&self) -> Option<Record> {
        let key = Dnskey::parse(&self.dnskey()?).ok()?;
        let mut rd = key.key_tag().to_be_bytes().to_vec();
        rd.push(alg::ECDSAP256SHA256);
        rd.push(digest::SHA256);
        rd.extend(key.ds_digest(digest::SHA256)?);
        Some(record(&self.origin, rtype::DS, rd))
    }

    pub fn add(&mut self, r: Record) {
        self.records.push(r);
    }

    pub fn signed(&self) -> bool {
        self.key.is_some()
    }

    fn is_cut(&self, n: &Name) -> bool {
        *n != self.origin && self.records.iter().any(|r| r.name == *n && r.rtype == rtype::NS)
    }

    /// RRSIG sady `set` (klic zony); `labels` = navesti puvodniho vlastnika.
    pub fn sign(&self, set: &[&Record], labels: Option<u8>) -> Option<Record> {
        let key = self.key.as_ref()?;
        let dnskey = Dnskey::parse(&self.dnskey()?).ok()?;
        let first = set[0];
        let wildcard_labels = first.name.labels.len() - usize::from(first.name.is_wildcard());
        let labels = labels.unwrap_or(wildcard_labels as u8);
        let mut rd = Vec::new();
        rd.extend_from_slice(&first.rtype.to_be_bytes());
        rd.push(alg::ECDSAP256SHA256);
        rd.push(labels);
        rd.extend_from_slice(&first.ttl.to_be_bytes());
        rd.extend_from_slice(&EXPIRATION.to_be_bytes());
        rd.extend_from_slice(&INCEPTION.to_be_bytes());
        rd.extend_from_slice(&dnskey.key_tag().to_be_bytes());
        self.origin.write(&mut rd, true);
        let mut data = rd.clone();
        let mut rdatas: Vec<Vec<u8>> = set.iter().map(|r| r.canonical_rdata()).collect();
        rdatas.sort();
        for r in rdatas {
            first.name.write(&mut data, true);
            data.extend_from_slice(&first.rtype.to_be_bytes());
            data.extend_from_slice(&first.class.to_be_bytes());
            data.extend_from_slice(&first.ttl.to_be_bytes());
            data.extend_from_slice(&(r.len() as u16).to_be_bytes());
            data.extend_from_slice(&r);
        }
        let sig: p256::ecdsa::Signature = key.sign_prehash(&Sha256::digest(&data)).ok()?;
        rd.extend_from_slice(&sig.to_bytes());
        Some(record(&first.name, rtype::RRSIG, rd))
    }

    /// Autoritativni jmena zony (bez zaznamu pod delegacemi, krome NS/DS).
    fn names(&self) -> Vec<Name> {
        let mut v: Vec<Name> = Vec::new();
        for r in &self.records {
            let below_cut = self
                .records
                .iter()
                .any(|c| c.rtype == rtype::NS && c.name != self.origin && r.name.is_in(&c.name) && r.name != c.name);
            if !below_cut && !v.contains(&r.name) {
                v.push(r.name.clone());
            }
        }
        v.sort_by(|a, b| a.canonical_cmp(b));
        v
    }

    fn types_at(&self, n: &Name) -> Vec<u16> {
        let mut t: Vec<u16> = self.records.iter().filter(|r| r.name == *n).map(|r| r.rtype).collect();
        let cut = self.is_cut(n);
        if self.signed() && !cut {
            t.push(rtype::RRSIG);
        }
        if cut && t.contains(&rtype::DS) {
            t.push(rtype::RRSIG);
        }
        t
    }

    /// Zaznamy NSEC nebo NSEC3 cele zony (podepsane).
    fn denial(&self) -> Vec<Record> {
        let names = self.names();
        let mut out = Vec::new();
        if !self.nsec3 {
            for (i, n) in names.iter().enumerate() {
                let next = &names[(i + 1) % names.len()];
                let mut rd = next.to_wire(false);
                let mut types = self.types_at(n);
                types.push(rtype::NSEC);
                rd.extend(bitmap(&types));
                out.push(record(n, rtype::NSEC, rd));
            }
        } else {
            // Prazdne nekoncove uzly (napr. `wild` pro `*.wild`) maji NSEC3 taky.
            let mut all = names.clone();
            for n in &names {
                let mut p = n.parent();
                while let Some(x) = p {
                    if !x.is_in(&self.origin) || x == self.origin {
                        break;
                    }
                    if !all.contains(&x) {
                        all.push(x.clone());
                    }
                    p = x.parent();
                }
            }
            let mut hashed: Vec<(Vec<u8>, Name)> = all.iter().map(|n| (nsec3_hash(n), n.clone())).collect();
            hashed.sort_by(|a, b| a.0.cmp(&b.0));
            for (i, (h, n)) in hashed.iter().enumerate() {
                let next = &hashed[(i + 1) % hashed.len()].0;
                let mut rd = vec![1, 0, 0, 0, 0, next.len() as u8];
                rd.extend_from_slice(next);
                rd.extend(bitmap(&self.types_at(n)));
                let owner = name(&alloc::format!("{}.{}", base32hex(h), self.origin.to_text().trim_end_matches('.')));
                let owner = if self.origin.is_root() { name(&base32hex(h)) } else { owner };
                out.push(record(&owner, rtype::NSEC3, rd));
            }
        }
        let mut signed = Vec::new();
        for r in out {
            let sig = self.sign(&[&r], None);
            signed.push(r);
            signed.extend(sig);
        }
        signed
    }

    fn rrset(&self, n: &Name, t: u16) -> Vec<&Record> {
        self.records.iter().filter(|r| r.name == *n && r.rtype == t).collect()
    }

    fn push_set(&self, out: &mut Vec<Record>, set: &[&Record], owner: Option<&Name>, labels: Option<u8>) {
        for r in set {
            let mut r = (*r).clone();
            if let Some(o) = owner {
                r.name = o.clone();
            }
            out.push(r);
        }
        if let Some(mut sig) = self.sign(set, labels) {
            if let Some(o) = owner {
                sig.name = o.clone();
            }
            out.push(sig);
        }
    }

    /// Dukazy neexistence pro jmena `names` (NSEC/NSEC3, ktere je
    /// pokryvaji nebo odpovidaji) - zjednodusene: cela sada (zony jsou male).
    fn proofs(&self, out: &mut Vec<Record>) {
        if self.signed() {
            out.extend(self.denial());
            let soa = self.rrset(&self.origin, rtype::SOA);
            self.push_set(out, &soa, None, None);
        } else {
            out.extend(self.rrset(&self.origin, rtype::SOA).into_iter().cloned());
        }
    }
}

/// Sada zon a odpovidac.
pub struct Hierarchy {
    pub zones: Vec<Zone>,
}

impl Hierarchy {
    /// Testovaci hierarchie (viz popis modulu).
    pub fn standard() -> Self {
        let mut root = Zone::new(".", Some(1), false);
        let mut test = Zone::new("test.", Some(2), false);
        let mut example = Zone::new("example.test.", Some(3), true);
        let mut unsigned = Zone::new("unsigned.test.", None, false);
        let mut bad = Zone::new("bad.test.", Some(4), false);
        example.add(a("www.example.test.", [10, 0, 2, 50]));
        example.add(a("*.wild.example.test.", [10, 0, 2, 51]));
        example.add(record(&name("alias.example.test."), rtype::CNAME, name("www.example.test.").to_wire(false)));
        example.add(record(&name("v6.example.test."), rtype::AAAA, vec![0xfd, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]));
        unsigned.add(a("www.unsigned.test.", [10, 0, 2, 52]));
        bad.add(a("www.bad.test.", [10, 0, 2, 53]));
        let ns = |z: &str| record(&name(z), rtype::NS, name(&alloc::format!("ns.{z}")).to_wire(false));
        root.add(ns("test."));
        root.add(test.ds().unwrap());
        test.add(ns("example.test."));
        test.add(example.ds().unwrap());
        test.add(ns("unsigned.test."));
        test.add(ns("bad.test."));
        // DS jineho klice: podpisy bad.test. neodpovidaji.
        let mut wrong = Zone::new("bad.test.", Some(9), false).ds().unwrap();
        wrong.name = name("bad.test.");
        test.add(wrong);
        test.add(a("host.test.", [10, 0, 2, 54]));
        Self {
            zones: vec![root, test, example, unsigned, bad],
        }
    }

    /// DS korenove zony (kotva pro validator).
    pub fn anchor(&self) -> Record {
        self.zones[0].ds().unwrap()
    }

    /// Nejhlubsi zona obsahujici `n`; pro DS nadrazena zona hranice.
    fn zone_for(&self, n: &Name, qtype: u16) -> &Zone {
        let mut best = &self.zones[0];
        for z in &self.zones {
            let deeper = z.origin.labels.len() > best.origin.labels.len();
            let applies = n.is_in(&z.origin) && !(qtype == rtype::DS && *n == z.origin && !z.origin.is_root());
            if applies && deeper {
                best = z;
            }
        }
        best
    }

    /// Odpoved jako resolver s DO a CD.
    pub fn answer(&self, query: &Message) -> Message {
        let mut resp = Message {
            id: query.id,
            flags: flags::QR | flags::RD | flags::RA | (query.flags & flags::CD),
            questions: query.questions.clone(),
            ..Message::default()
        };
        let Some(q) = query.questions.first() else {
            resp.flags |= rcode::SERVFAIL as u16;
            return resp;
        };
        let mut qname = q.name.clone();
        for _ in 0..8 {
            let z = self.zone_for(&qname, q.qtype);
            let exact = z.rrset(&qname, q.qtype);
            if !exact.is_empty() {
                z.push_set(&mut resp.answers, &exact, None, None);
                return resp;
            }
            let cname = z.rrset(&qname, rtype::CNAME);
            if !cname.is_empty() && q.qtype != rtype::CNAME {
                z.push_set(&mut resp.answers, &cname, None, None);
                qname = cname[0].target().unwrap();
                continue;
            }
            let exists = z.records.iter().any(|r| r.name.is_in(&qname));
            if !exists {
                // Wildcard u nejblizsiho existujiciho predka.
                let mut ce = qname.parent();
                while let Some(c) = ce.clone() {
                    if z.records.iter().any(|r| r.name.is_in(&c)) {
                        break;
                    }
                    ce = c.parent();
                }
                let mut wildcard_exists = false;
                if let Some(c) = ce {
                    let wc = c.wildcard_child();
                    let set = z.rrset(&wc, q.qtype);
                    if !set.is_empty() {
                        z.push_set(&mut resp.answers, &set, Some(&qname), Some(wc.labels.len() as u8 - 1));
                        z.proofs(&mut resp.authority);
                        return resp;
                    }
                    wildcard_exists = z.records.iter().any(|r| r.name == wc);
                }
                // Wildcard bez typu = NODATA, jinak NXDOMAIN.
                if !wildcard_exists {
                    resp.flags |= rcode::NXDOMAIN as u16;
                }
            }
            z.proofs(&mut resp.authority);
            return resp;
        }
        resp.flags |= rcode::SERVFAIL as u16;
        resp
    }

    /// Dotaz pro testy (bez site).
    pub fn ask(&self, n: &Name, qtype: u16) -> Message {
        self.answer(&Message {
            id: 1,
            flags: flags::RD | flags::CD,
            questions: vec![Question {
                name: n.clone(),
                qtype,
                qclass: CLASS_IN,
            }],
            ..Message::default()
        })
    }
}
