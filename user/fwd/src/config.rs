//! Konfigurace firewallu v TOML: preklad na `FwConfig` pro `netd`
//! a zapis rozhodnuti uzivatele.
//!
//! ```toml
//! [firewall]
//! ask_default = "deny"        # bez agenta / po vyprseni dotazu: deny | allow
//! ask_timeout = 60            # s
//! unknown_network = "public"  # profil neznamej site, nez rozhodne uzivatel
//! audit_entries = 4096
//!
//! [profiles.public]
//! inbound = "deny"            # deny | listeners
//! ping = false
//! stealth = true
//!
//! [profiles.home]
//! inbound = "listeners"
//! ping = true
//! [[profiles.home.allow_in]]
//! proto = ["tcp"]
//! from = "10.0.0.0/8"
//! ports = "22"
//!
//! [rate]
//! syn_per_sec = 100
//! syn_burst = 200
//! icmp_per_sec = 10
//! icmp_burst = 20
//!
//! [block]
//! networks = ["192.0.2.0/24"]
//! domains = ["ads.example", "*.tracker.example"]
//!
//! [[apps]]                    # rozhodnuti uzivatele (zapisuje fwd)
//! id = "org.magnumopus.netprobe"
//! [[apps.rule]]
//! action = "allow"            # allow | deny
//! proto = ["tcp"]
//! to = "api.example.com"
//! ports = "443"
//!
//! [[networks]]                # znamé site (zapisuje fwd)
//! id = "52:54:00:12:35:02"    # MAC brany
//! gateway = "10.0.2.2"
//! profile = "home"
//! ```

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::net::IpAddr;

use conf::{Table, Value};
use netproto::Rule;
use netproto::fw::{FwConfig, InRule, inbound};

pub type Res<T> = core::result::Result<T, String>;

/// Nastaveni samotne sluzby.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    /// Bez agenta / po vyprseni dotazu povolit?
    pub ask_allow: bool,
    pub ask_timeout_ns: u64,
    pub unknown_network: String,
}

pub fn service(t: &Table) -> Res<Service> {
    let ask_allow = match t.get_str("firewall.ask_default").unwrap_or("deny") {
        "deny" => false,
        "allow" => true,
        v => return Err(format!("firewall.ask_default: nezname {v}")),
    };
    let timeout = t.get_int("firewall.ask_timeout").unwrap_or(60);
    if !(1..=3600).contains(&timeout) {
        return Err(String::from("firewall.ask_timeout: 1..=3600 s"));
    }
    Ok(Service {
        ask_allow,
        ask_timeout_ns: timeout as u64 * 1_000_000_000,
        unknown_network: String::from(t.get_str("firewall.unknown_network").unwrap_or("public")),
    })
}

fn net(s: &str) -> Res<(IpAddr, u8)> {
    match manifest::target(s).map_err(|e| e.0)? {
        netproto::Target::Net { addr, prefix } => Ok((addr, prefix)),
        netproto::Target::Any => Ok(("0.0.0.0".parse().unwrap(), 0)),
        _ => Err(format!("{s}: ocekavana sit")),
    }
}

fn rate(t: &Table, key: &str, default: u32) -> Res<u32> {
    match t.get_int(&format!("rate.{key}")) {
        None => Ok(default),
        Some(v) => u32::try_from(v).map_err(|_| format!("rate.{key}: mimo rozsah")),
    }
}

/// Profil site pro sit `network_id` (zname site, jinak `unknown_network`).
pub fn profile_for(t: &Table, network_id: Option<&str>) -> String {
    if let Some(id) = network_id {
        for n in t.tables("networks") {
            if n.get_str("id") == Some(id)
                && let Some(p) = n.get_str("profile")
            {
                return String::from(p);
            }
        }
    }
    String::from(t.get_str("firewall.unknown_network").unwrap_or("public"))
}

/// Je sit znama (ma ulozeny profil)?
pub fn network_known(t: &Table, id: &str) -> bool {
    t.tables("networks").iter().any(|n| n.get_str("id") == Some(id))
}

/// Konfigurace pro `netd` s profilem `profile`. Chyba = neplatny soubor
/// (fwd ho neprevezme).
pub fn build(t: &Table, profile: &str) -> Res<FwConfig> {
    service(t)?;
    let path = format!("profiles.{profile}");
    let p = t
        .get_table(&path)
        .ok_or_else(|| format!("{path}: profil neexistuje"))?;
    let mode = match p.get_str("inbound").unwrap_or("deny") {
        "deny" => inbound::DENY,
        "listeners" => inbound::LISTENERS,
        v => return Err(format!("{path}.inbound: nezname {v}")),
    };
    let mut allow_in = Vec::new();
    for r in p.tables("allow_in") {
        let protos = manifest::protos(r.get("proto")).map_err(|e| e.0)?;
        let from = net(r.get_str("from").unwrap_or("*"))?;
        allow_in.push(InRule {
            protos,
            from,
            ports: manifest::ports(r.get("ports")).map_err(|e| e.0)?,
        });
    }
    let mut blocked = Vec::new();
    for n in t.get_strs("block.networks").unwrap_or_default() {
        blocked.push(net(n)?);
    }
    let mut dns_block = Vec::new();
    for d in t.get_strs("block.domains").unwrap_or_default() {
        let name = d.strip_prefix("*.").unwrap_or(d);
        if !netproto::dns::valid_name(name) {
            return Err(format!("block.domains: neplatne jmeno {d}"));
        }
        dns_block.push(String::from(d));
    }
    let mut user_rules = Vec::new();
    for app in t.tables("apps") {
        let id = app.get_str("id").ok_or("apps: chybi id")?;
        for r in app.tables("rule") {
            let allow = match r.get_str("action").unwrap_or("allow") {
                "allow" => true,
                "deny" => false,
                v => return Err(format!("apps.rule.action: nezname {v}")),
            };
            let rule = manifest::parse_rule(r).map_err(|e| format!("{id}: {}", e.0))?;
            user_rules.push((String::from(id), rule, allow));
        }
    }
    let audit = t.get_int("firewall.audit_entries").unwrap_or(4096);
    Ok(FwConfig {
        profile: String::from(profile),
        inbound: mode,
        ping: p.get_bool("ping").unwrap_or(false),
        stealth: p.get_bool("stealth").unwrap_or(true),
        allow_in,
        blocked,
        syn_rate: (rate(t, "syn_per_sec", 100)?, rate(t, "syn_burst", 200)?),
        icmp_rate: (rate(t, "icmp_per_sec", 10)?, rate(t, "icmp_burst", 20)?),
        dns_block,
        user_rules,
        audit_capacity: u32::try_from(audit.clamp(16, 1 << 20)).unwrap_or(4096),
    })
}

/// Prida trvale rozhodnuti uzivatele pro aplikaci.
pub fn add_rule(t: &mut Table, app: &str, rule: &Rule, allow: bool) {
    let apps = t.array_mut("apps");
    let i = match apps
        .iter()
        .position(|a| a.as_table().and_then(|t| t.get_str("id")) == Some(app))
    {
        Some(i) => i,
        None => {
            let mut a = Table::new();
            a.set("id", Value::Str(String::from(app)));
            apps.push(Value::Table(a));
            apps.len() - 1
        }
    };
    let Value::Table(entry) = &mut apps[i] else {
        return;
    };
    let mut r = manifest::rule_table(rule);
    r.entries
        .insert(0, (String::from("action"), Value::Str(String::from(if allow { "allow" } else { "deny" }))));
    entry.array_mut("rule").push(Value::Table(r));
}

/// Zapamatuje profil site.
pub fn set_network(t: &mut Table, id: &str, gateway: &str, profile: &str) {
    let nets = t.array_mut("networks");
    for n in nets.iter_mut() {
        if let Value::Table(n) = n
            && n.get_str("id") == Some(id)
        {
            n.set("profile", Value::Str(String::from(profile)));
            return;
        }
    }
    let mut n = Table::new();
    n.set("id", Value::Str(String::from(id)));
    n.set("gateway", Value::Str(String::from(gateway)));
    n.set("profile", Value::Str(String::from(profile)));
    nets.push(Value::Table(n));
}

/// Je profil v konfiguraci?
pub fn has_profile(t: &Table, profile: &str) -> bool {
    t.get_table(&format!("profiles.{profile}")).is_some()
}
