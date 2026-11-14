//! Manifest aplikace: kdo program je a co smi.
//!
//! Soubor `<program>.manifest.toml` lezi vedle programu (v initfs, pozdeji
//! v balicku). Init podle nej spusti program v jobu s rozpoctem a preda mu
//! omezenou relaci site. Bez manifestu program sit nedostane.
//!
//! ```toml
//! [app]
//! id = "org.magnumopus.netprobe"   # povinne: a-z A-Z 0-9 . - _
//! name = "Sitova sonda"            # pro dotazy uzivateli
//! jit = true                       # smi spoustet pamet bez pecete (JIT); vychozi ne
//! static_tls_reserve = "8K"        # rezerva statickeho TLS pro dlopen (vychozi 2K, nejvys 64K)
//!
//! [limits]                         # rozpocet jobu (bez sekce = bez limitu)
//! memory = "64M"                   # uzivatelska pamet
//! kernel_memory = "8M"             # pamet jadra
//! stack = "1M"                     # zasobnik hlavniho vlakna (vychozi 256K)
//!
//! [network]                        # bez sekce = zadna sit
//! rights = ["info", "tcp", "udp", "dns"]
//! ask = true                       # cil mimo pravidla: zeptat se uzivatele
//! sockets = 256                    # nejvic socketu najednou
//! buffers = "16M"                  # nejvic pameti bufferu socketu
//!
//! [[network.connect]]
//! proto = ["tcp"]                  # tcp, udp, icmp (vychozi vse)
//! to = "*.example.com"             # "*", sit "10.0.0.0/8", adresa, jmeno
//! ports = "443"                    # "80-90", "*" (vychozi vse)
//!
//! [[network.listen]]
//! ports = "8080"
//!
//! [[command]]                      # prikaz terminalu (registr prikazu)
//! name = "rg"                      # jmeno pro `Command::new` a shell
//! program = "rg"                   # program balicku (vychozi tento)
//! ```
//!
//! Prava site (`rights`): `info`, `tcp`, `listen`, `udp`, `dns`, `ping`,
//! `restrict`, `configure`, `audit` (stejna jmena jako
//! `netproto::rights_names`).

#![no_std]

extern crate alloc;

#[cfg(test)]
mod tests;

use alloc::string::String;
use alloc::vec::Vec;
use core::net::IpAddr;

use conf::{Table, Value};
use netproto::auth::{Quota, Restriction, layer_flags, valid_app_id};
use netproto::{Policy, Rule, Target, proto, rights};

/// Chyba manifestu (s popisem pro log).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err(msg: &str) -> Error {
    Error(String::from(msg))
}

/// Rozpocet jobu z `[limits]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Limits {
    pub memory: Option<u64>,
    pub kernel_memory: Option<u64>,
    /// Zasobnik hlavniho vlakna.
    pub stack: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub limits: Limits,
    /// Relace site pro program (`None` = bez site).
    pub network: Option<Restriction>,
    /// `[app] jit = true`: program smi spoustet pamet bez pecete (JIT,
    /// napr. prohlizec, .NET). Vychozi ne.
    pub jit: bool,
    /// `[app] static_tls_reserve`: rezerva statickeho TLS pro knihovny
    /// z `dlopen` s initial-exec (`None` = vychozi zavadece 2 KiB).
    pub static_tls_reserve: Option<u64>,
    /// Prikazy terminalu, ktere program (balicek) poskytuje.
    pub commands: Vec<CommandDecl>,
}

/// Prikaz terminalu z `[[command]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandDecl {
    /// Jmeno prikazu (`a-z 0-9 . _ -`, zacina pismenem nebo cislici).
    pub name: String,
    /// Program, ktery prikaz spusti (`None` = program manifestu).
    pub program: Option<String>,
}

/// Platne jmeno prikazu.
pub fn valid_command_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let t = conf::parse(text).map_err(|e| Error(alloc::format!("{e}")))?;
        let id = t.get_str("app.id").ok_or_else(|| err("chybi app.id"))?;
        if id.is_empty() || !valid_app_id(id) {
            return Err(err("neplatne app.id"));
        }
        let name = t.get_str("app.name").unwrap_or(id);
        let limits = Limits {
            memory: size(&t, "limits.memory")?,
            kernel_memory: size(&t, "limits.kernel_memory")?,
            stack: size(&t, "limits.stack")?,
        };
        if limits.stack.is_some_and(|s| s == 0 || s > 1 << 30) {
            return Err(err("limits.stack: 4K az 1G"));
        }
        let static_tls_reserve = size(&t, "app.static_tls_reserve")?;
        if static_tls_reserve.is_some_and(|s| s > 64 * 1024) {
            return Err(err("app.static_tls_reserve: nejvys 64K"));
        }
        let network = match t.get_table("network") {
            None => None,
            Some(n) => Some(network(n, id)?),
        };
        let mut commands: Vec<CommandDecl> = Vec::new();
        for c in t.tables("command") {
            let name = c.get_str("name").ok_or_else(|| err("command: chybi name"))?;
            if !valid_command_name(name) {
                return Err(err("command.name: jen a-z 0-9 . _ - (nejvys 64)"));
            }
            if commands.iter().any(|x| x.name == name) {
                return Err(err("command.name: opakovane jmeno"));
            }
            let program = c.get_str("program").map(String::from);
            if program.as_deref().is_some_and(|p| p.is_empty() || p.contains('/')) {
                return Err(err("command.program: jmeno programu balicku bez cesty"));
            }
            commands.push(CommandDecl { name: String::from(name), program });
        }
        Ok(Self {
            id: String::from(id),
            name: String::from(name),
            limits,
            network,
            jit: t.get_bool("app.jit").unwrap_or(false),
            static_tls_reserve,
            commands,
        })
    }
}

fn size(t: &Table, path: &str) -> Result<Option<u64>, Error> {
    match t.get(path) {
        None => Ok(None),
        Some(v) => conf::parse_size(v)
            .map(Some)
            .ok_or_else(|| Error(alloc::format!("{path}: neplatna velikost"))),
    }
}

fn network(n: &Table, id: &str) -> Result<Restriction, Error> {
    let names = n.get_strs("rights").ok_or_else(|| err("network.rights: chybi seznam prav"))?;
    let mut mask = 0;
    for name in names {
        mask |= right(name).ok_or_else(|| Error(alloc::format!("network.rights: nezname pravo {name}")))?;
    }
    let mut connect = Vec::new();
    for r in n.tables("connect") {
        connect.push(parse_rule(r)?);
    }
    let mut bind = Vec::new();
    for l in n.tables("listen") {
        bind.push(ports(l.get("ports"))?);
    }
    let policy = Policy {
        connect: Some(connect),
        bind: Some(bind),
    };
    policy.validate().map_err(|_| err("network: neplatna pravidla"))?;
    let ask = n.get_bool("ask").unwrap_or(false);
    let sockets = match n.get_int("sockets") {
        None => netproto::auth::UNLIMITED_SOCKETS,
        Some(v) => u32::try_from(v).map_err(|_| err("network.sockets: mimo rozsah"))?,
    };
    let bytes = size(n, "buffers")?.unwrap_or(netproto::auth::UNLIMITED_BYTES);
    // Soubezne dotazy na relaci (vic vlaken aplikace, server s mnoha
    // spojenimi); nad limit dotazy pockaji ve fronte.
    let ops = match n.get_int("ops") {
        None => netproto::auth::DEFAULT_OPS,
        Some(v) => u32::try_from(v)
            .ok()
            .filter(|v| (1..=netproto::auth::MAX_OPS).contains(v))
            .ok_or_else(|| err("network.ops: mimo rozsah 1 az 1024"))?,
    };
    Ok(Restriction {
        rights: mask,
        policy,
        app: String::from(id),
        flags: if ask { layer_flags::ASK } else { 0 },
        quota: Quota { sockets, bytes, ops },
    })
}

/// Pravo site podle jmena (`netproto::rights_names`).
pub fn right(name: &str) -> Option<u32> {
    Some(match name {
        "info" => rights::INFO,
        "tcp" => rights::TCP_CONNECT,
        "listen" => rights::TCP_LISTEN,
        "udp" => rights::UDP,
        "dns" => rights::RESOLVE,
        "ping" => rights::PING,
        "restrict" => rights::RESTRICT,
        "configure" => rights::CONFIGURE,
        "audit" => rights::AUDIT,
        _ => return None,
    })
}

/// Protokoly (`["tcp", "udp"]`); bez klice = vsechny.
pub fn protos(v: Option<&Value>) -> Result<u8, Error> {
    let Some(v) = v else {
        return Ok(proto::ALL);
    };
    let list: Vec<&str> = match v {
        Value::Str(s) => alloc::vec![s.as_str()],
        Value::Array(a) => a
            .iter()
            .map(|x| x.as_str().ok_or_else(|| err("proto: ocekavan retezec")))
            .collect::<Result<_, _>>()?,
        _ => return Err(err("proto: ocekavan seznam")),
    };
    let mut m = 0;
    for p in list {
        m |= match p {
            "tcp" => proto::TCP,
            "udp" => proto::UDP,
            "icmp" => proto::ICMP,
            "*" => proto::ALL,
            _ => return Err(Error(alloc::format!("proto: neznamy protokol {p}"))),
        };
    }
    Ok(m)
}

/// Porty: cislo, `"443"`, `"80-90"`, `"*"`; bez klice = vsechny.
pub fn ports(v: Option<&Value>) -> Result<(u16, u16), Error> {
    let bad = || err("ports: ocekavano cislo, \"od-do\" nebo \"*\"");
    match v {
        None => Ok((0, u16::MAX)),
        Some(Value::Int(n)) => u16::try_from(*n).map(|p| (p, p)).map_err(|_| bad()),
        Some(Value::Str(s)) => {
            let s = s.trim();
            if s == "*" {
                return Ok((0, u16::MAX));
            }
            let (lo, hi) = s.split_once('-').unwrap_or((s, s));
            let lo: u16 = lo.trim().parse().map_err(|_| bad())?;
            let hi: u16 = hi.trim().parse().map_err(|_| bad())?;
            if lo > hi {
                return Err(bad());
            }
            Ok((lo, hi))
        }
        _ => Err(bad()),
    }
}

/// Cil: `"*"`, sit `"10.0.0.0/8"`, adresa `"1.2.3.4"` / `"::1"`, jmeno
/// `"api.example.com"` nebo `"*.example.com"`.
pub fn target(s: &str) -> Result<Target, Error> {
    let s = s.trim();
    if s == "*" {
        return Ok(Target::Any);
    }
    if let Some((a, p)) = s.split_once('/') {
        let addr: IpAddr = a.parse().map_err(|_| err("to: neplatna sit"))?;
        let prefix: u8 = p.parse().map_err(|_| err("to: neplatny prefix"))?;
        return Ok(Target::Net { addr, prefix });
    }
    if let Ok(addr) = s.parse::<IpAddr>() {
        let prefix = if addr.is_ipv4() { 32 } else { 128 };
        return Ok(Target::Net { addr, prefix });
    }
    let name = s.strip_prefix("*.").unwrap_or(s);
    if netproto::dns::valid_name(name) {
        Ok(Target::Host(String::from(s)))
    } else {
        Err(Error(alloc::format!("to: neplatny cil {s}")))
    }
}

/// Pravidlo z tabulky (`proto`, `to`, `ports`).
pub fn parse_rule(t: &Table) -> Result<Rule, Error> {
    let to = t.get_str("to").ok_or_else(|| err("pravidlo: chybi to"))?;
    let rule = Rule {
        protos: protos(t.get("proto"))?,
        target: target(to)?,
        ports: ports(t.get("ports"))?,
    };
    rule.validate().map_err(|_| err("pravidlo: neplatne"))?;
    Ok(rule)
}

/// Pravidlo zpet do tabulky (zapis rozhodnuti uzivatele).
pub fn rule_table(rule: &Rule) -> Table {
    let mut t = Table::new();
    let mut list = Vec::new();
    for (bit, name) in [(proto::TCP, "tcp"), (proto::UDP, "udp"), (proto::ICMP, "icmp")] {
        if rule.protos & bit != 0 {
            list.push(Value::Str(String::from(name)));
        }
    }
    t.set("proto", Value::Array(list));
    t.set("to", Value::Str(target_text(&rule.target)));
    let ports = if rule.ports == (0, u16::MAX) {
        String::from("*")
    } else if rule.ports.0 == rule.ports.1 {
        alloc::format!("{}", rule.ports.0)
    } else {
        alloc::format!("{}-{}", rule.ports.0, rule.ports.1)
    };
    t.set("ports", Value::Str(ports));
    t
}

/// Text cile (opak [`target`]).
pub fn target_text(t: &Target) -> String {
    match t {
        Target::Any => String::from("*"),
        Target::Net { addr, prefix } => {
            let full = if addr.is_ipv4() { 32 } else { 128 };
            if *prefix == full {
                alloc::format!("{addr}")
            } else {
                alloc::format!("{addr}/{prefix}")
            }
        }
        Target::Host(h) => h.clone(),
    }
}
