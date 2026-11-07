//! Validator DNSSEC: retez duvery od korenove kotvy k odpovedi.
//!
//! Hranice zon se hledaji dotazy DS shora (koren, `com`, `example.com`, ...):
//! - DS existuje a je podepsany klici nadrazene zony -> klice podrizene zony
//!   (DNSKEY podepsany klicem s otiskem z DS),
//! - DS neexistuje: overeny dukaz NSEC/NSEC3 rekne, jestli jde o nepodepsanou
//!   delegaci (zona pod ni je **nezabezpecena**) nebo jmeno uvnitr zony
//!   (pokracovat se stejnymi klici),
//! - cokoli neovereneho tam, kde ma byt podpis = **podvrzene** (bogus).
//!
//! Odpoved je pak `Secure` (vse overeno), `Insecure` (zona nepodepsana -
//! data se pouziji bez overeni) nebo `Bogus` (podpis/dukaz neprosel - data
//! se zahodi). Chyba dotazu = `Indeterminate`.

use alloc::vec::Vec;

use super::denial::{DsAbsence, Nsec, Nsec3, Proofs};
use super::{Dnskey, Ds, alg, digest, rrsets, signatures_for, verify_with_keys};
use crate::message::{Message, Record, rcode};
use crate::name::Name;
use crate::rtype;

/// Odkud validator bere dotazy (v `dnsd` sifrovany resolver, s DO a CD).
pub trait Fetch {
    /// Odpoved na dotaz; `None` = dotaz se nepodaril.
    fn query(&mut self, name: &Name, qtype: u16) -> Option<Message>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Security {
    Secure,
    Insecure,
    Bogus(&'static str),
    /// Data pro overeni se nepodarilo ziskat.
    Indeterminate,
}

#[derive(Debug, Clone)]
enum State {
    /// Hranice zony s overenymi klici.
    Secure(Vec<Dnskey>),
    /// Jmeno uvnitr nadrazene zony (neni hranice).
    NotCut,
    /// Nepodepsana delegace: zona a vse pod ni nezabezpecene.
    Insecure,
    /// Jmeno neexistuje (vse pod nim taky ne).
    NxDomain,
}

struct Cached {
    name: Name,
    state: State,
    until: u32,
}

/// Nejvic polozek mezipameti zon a doba platnosti (horni mez TTL).
const CACHE_MAX: usize = 512;
const CACHE_SECS: u32 = 3600;

pub struct Validator {
    anchors: Vec<Ds>,
    cache: Vec<Cached>,
}

/// Zona a jeji klice, nebo duvod, proc ne.
type Zone = (Name, Vec<Dnskey>);

enum Walk {
    Secure(Zone),
    Insecure,
    Fail(Security),
}

fn min_ttl(records: &[&Record]) -> u32 {
    records.iter().map(|r| r.ttl).min().unwrap_or(0)
}

/// Overene NSEC/NSEC3 z `records` (podepsane klici zony).
pub fn verified_proofs(records: &[Record], keys: &[Dnskey], now: u32) -> Proofs {
    let mut p = Proofs::default();
    for set in rrsets(records) {
        let t = set[0].rtype;
        if t != rtype::NSEC && t != rtype::NSEC3 {
            continue;
        }
        let sigs = signatures_for(records, &set[0].name, t);
        if verify_with_keys(&set, &sigs, keys, now).is_err() {
            continue;
        }
        for r in set {
            if t == rtype::NSEC {
                if let Ok(n) = Nsec::parse(r) {
                    p.nsec.push(n);
                }
            } else if let Ok(n) = Nsec3::parse(r) {
                p.nsec3.push(n);
            }
        }
    }
    p
}

impl Validator {
    /// `anchors` = DS korenove zony (IANA: KSK-2017 20326, KSK-2024 38696).
    pub fn new(anchors: Vec<Ds>) -> Self {
        Self { anchors, cache: Vec::new() }
    }

    fn cached(&self, name: &Name, now: u32) -> Option<State> {
        self.cache
            .iter()
            .find(|c| c.name == *name && (c.until.wrapping_sub(now) as i32) > 0)
            .map(|c| c.state.clone())
    }

    fn remember(&mut self, name: &Name, state: &State, ttl: u32, now: u32) {
        self.cache.retain(|c| c.name != *name && (c.until.wrapping_sub(now) as i32) > 0);
        if self.cache.len() >= CACHE_MAX {
            self.cache.remove(0);
        }
        self.cache.push(Cached {
            name: name.clone(),
            state: state.clone(),
            until: now.wrapping_add(ttl.clamp(1, CACHE_SECS)),
        });
    }

    /// Overene klice zony `zone`, jejichz DNSKEY odpovida nekteremu z `ds`.
    fn zone_keys(&mut self, f: &mut dyn Fetch, zone: &Name, ds: &[Ds], now: u32) -> Result<(Vec<Dnskey>, u32), Security> {
        let resp = f.query(zone, rtype::DNSKEY).ok_or(Security::Indeterminate)?;
        let sets = rrsets(&resp.answers);
        let set = sets
            .iter()
            .find(|s| s[0].rtype == rtype::DNSKEY && s[0].name == *zone)
            .ok_or(Security::Bogus("chybi DNSKEY zony"))?;
        let keys: Vec<Dnskey> = set.iter().filter_map(|r| Dnskey::parse(r).ok()).collect();
        let entry: Vec<Dnskey> = keys
            .iter()
            .filter(|k| k.usable() && ds.iter().any(|d| d.matches(k)))
            .cloned()
            .collect();
        if entry.is_empty() {
            return Err(Security::Bogus("zadny DNSKEY neodpovida DS"));
        }
        let sigs = signatures_for(&resp.answers, zone, rtype::DNSKEY);
        verify_with_keys(set, &sigs, &entry, now).map_err(|_| Security::Bogus("DNSKEY neni podepsany klicem z DS"))?;
        Ok((keys.into_iter().filter(Dnskey::usable).collect(), min_ttl(set)))
    }

    /// Stav zony, ve ktere lezi `name` (projde hranice od korene).
    fn walk(&mut self, f: &mut dyn Fetch, name: &Name, now: u32) -> Walk {
        let root = Name::root();
        let mut keys = match self.cached(&root, now) {
            Some(State::Secure(k)) => k,
            _ => {
                let anchors = self.anchors.clone();
                match self.zone_keys(f, &root, &anchors, now) {
                    Ok((k, ttl)) => {
                        self.remember(&root, &State::Secure(k.clone()), ttl, now);
                        k
                    }
                    Err(e) => return Walk::Fail(e),
                }
            }
        };
        let mut zone = root;
        for i in 1..=name.labels.len() {
            let cand = name.suffix(i);
            let state = match self.cached(&cand, now) {
                Some(s) => s,
                None => match self.probe(f, &cand, &keys, now) {
                    Ok((s, ttl)) => {
                        self.remember(&cand, &s, ttl, now);
                        s
                    }
                    Err(e) => return Walk::Fail(e),
                },
            };
            match state {
                State::Secure(k) => {
                    zone = cand;
                    keys = k;
                }
                State::NotCut => {}
                State::Insecure => return Walk::Insecure,
                State::NxDomain => break,
            }
        }
        Walk::Secure((zone, keys))
    }

    /// Dotaz DS na `cand` v zone s klici `keys`.
    fn probe(&mut self, f: &mut dyn Fetch, cand: &Name, keys: &[Dnskey], now: u32) -> Result<(State, u32), Security> {
        let resp = f.query(cand, rtype::DS).ok_or(Security::Indeterminate)?;
        let sets = rrsets(&resp.answers);
        if let Some(ds_set) = sets.iter().find(|s| s[0].rtype == rtype::DS && s[0].name == *cand) {
            let sigs = signatures_for(&resp.answers, cand, rtype::DS);
            verify_with_keys(ds_set, &sigs, keys, now).map_err(|_| Security::Bogus("podpis DS"))?;
            let ds: Vec<Ds> = ds_set
                .iter()
                .filter_map(|r| Ds::parse(r).ok())
                .filter(|d| alg::supported(d.algorithm) && digest::supported(d.digest_type))
                .collect();
            // Jen nepodporovane algoritmy: zona se bere jako nepodepsana.
            if ds.is_empty() {
                return Ok((State::Insecure, min_ttl(ds_set)));
            }
            let (k, ttl) = self.zone_keys(f, cand, &ds, now)?;
            return Ok((State::Secure(k), ttl.min(min_ttl(ds_set))));
        }
        // CNAME na miste kandidata: neni hranice zony.
        if sets.iter().any(|s| s[0].rtype == rtype::CNAME && s[0].name == *cand) {
            return Ok((State::NotCut, CACHE_SECS));
        }
        let proofs = verified_proofs(&resp.authority, keys, now);
        if proofs.unusable_nsec3() {
            return Ok((State::Insecure, CACHE_SECS));
        }
        let ttl = resp.authority.iter().map(|r| r.ttl).min().unwrap_or(CACHE_SECS);
        match proofs.ds_absence(cand) {
            Some(DsAbsence::NotDelegation) => Ok((State::NotCut, ttl)),
            Some(DsAbsence::Insecure) => Ok((State::Insecure, ttl)),
            Some(DsAbsence::NxDomain) => Ok((State::NxDomain, ttl)),
            None => Err(Security::Bogus("chybi dukaz neexistence DS")),
        }
    }

    /// Overi odpoved `resp` na dotaz (`qname`, `qtype`) v case `now`.
    pub fn validate(&mut self, f: &mut dyn Fetch, qname: &Name, qtype: u16, resp: &Message, now: u32) -> Security {
        let sets = rrsets(&resp.answers);
        let positive = sets.iter().any(|s| s[0].rtype == qtype);
        if resp.rcode() == rcode::NXDOMAIN || !positive {
            let (_, keys) = match self.walk(f, qname, now) {
                Walk::Secure(z) => z,
                Walk::Insecure => return Security::Insecure,
                Walk::Fail(e) => return e,
            };
            // CNAME pred NXDOMAIN/NODATA: retez overit stejne jako kladnou odpoved.
            for s in &sets {
                if let Err(e) = self.check_rrset(f, &resp.answers, s, &resp.authority, now) {
                    return e;
                }
            }
            let target = sets
                .iter()
                .rev()
                .find(|s| s[0].rtype == rtype::CNAME)
                .and_then(|s| s[0].target())
                .unwrap_or_else(|| qname.clone());
            let proofs = verified_proofs(&resp.authority, &keys, now);
            if target != *qname {
                // Cil CNAME muze byt v jine zone: jeho dukaz z jeho zony.
                return match self.walk(f, &target, now) {
                    Walk::Insecure => Security::Insecure,
                    Walk::Fail(e) => e,
                    Walk::Secure((_, k)) => self.negative(&verified_proofs(&resp.authority, &k, now), resp, &target, qtype),
                };
            }
            if proofs.unusable_nsec3() {
                return Security::Insecure;
            }
            return self.negative(&proofs, resp, qname, qtype);
        }
        let mut insecure = false;
        for s in &sets {
            match self.check_rrset(f, &resp.answers, s, &resp.authority, now) {
                Ok(true) => {}
                Ok(false) => insecure = true,
                Err(e) => return e,
            }
        }
        if insecure { Security::Insecure } else { Security::Secure }
    }

    fn negative(&self, proofs: &Proofs, resp: &Message, name: &Name, qtype: u16) -> Security {
        let ok = if resp.rcode() == rcode::NXDOMAIN { proofs.nxdomain(name) } else { proofs.nodata(name, qtype) };
        if ok { Security::Secure } else { Security::Bogus("chybi dukaz neexistence") }
    }

    /// Overi jednu sadu odpovedi. `Ok(true)` = overena, `Ok(false)` = zona
    /// nepodepsana.
    fn check_rrset(&mut self, f: &mut dyn Fetch, answers: &[Record], set: &[&Record], authority: &[Record], now: u32) -> Result<bool, Security> {
        let owner = &set[0].name;
        let (zone, keys) = match self.walk(f, owner, now) {
            Walk::Secure(z) => z,
            Walk::Insecure => return Ok(false),
            Walk::Fail(e) => return Err(e),
        };
        let sigs: Vec<_> = signatures_for(answers, owner, set[0].rtype)
            .into_iter()
            .filter(|s| s.signer == zone)
            .collect();
        match verify_with_keys(set, &sigs, &keys, now) {
            Ok(None) => Ok(true),
            Ok(Some(wildcard)) => {
                let proofs = verified_proofs(authority, &keys, now);
                if proofs.wildcard_answer(owner, &wildcard) {
                    Ok(true)
                } else {
                    Err(Security::Bogus("wildcard bez dukazu"))
                }
            }
            Err(_) => Err(Security::Bogus("podpis odpovedi")),
        }
    }
}
