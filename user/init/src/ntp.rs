//! Synchronizace casu: spusteni `ntpd`.
//!
//! - Konfigurace `ntpd.toml` z initfs (servery, intervaly). V testovacim
//!   behu (soubor `net-test`) jen pomocny server NTP hostitele s kratkymi
//!   intervaly.
//! - Relace site podle manifestu `ntpd.manifest.toml` a navic jen cile
//!   serveru (nejmensi prava): u `nts://` TCP na NTS-KE a UDP na NTP (server
//!   NTP muze NTS-KE urcit jinak: UDP 123 kamkoli a na server NTS-KE na
//!   libovolny port), u neoverenych serveru jen UDP na ne.
//! - Relace FS s ulozistem certifikatu a kanal k revocd (TLS NTS-KE).
//! - Kanal k `timed` (zprava `ntp`): ntpd jen navrhuje korekce, overi
//!   a provede je `timed`.
//!
//! Po restartu `timed` supervisorem kanal ntpd zanikne (zaloguje se);
//! obnova spojeni prijde se zavislostmi v supervisoru.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::net::{IpAddr, SocketAddr};
use core::time::Duration;

use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Process, Result, Startup, println, time};
use netproto::{Rule, Target, proto};

use crate::system::NetService;

pub struct NtpService {
    pub process: Process,
    pub control: Channel,
}

/// Port pomocneho serveru `key` v testovacim behu (`net-test`).
fn test_port(fs: &Initfs, key: &str) -> Option<u16> {
    let text = core::str::from_utf8(fs.find("net-test")?).ok()?;
    text.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .and_then(|(_, v)| v.trim().parse().ok())
}

fn host_target(host: &str) -> Option<Target> {
    match host.parse::<IpAddr>() {
        Ok(ip) => Some(Target::Net { addr: ip, prefix: if ip.is_ipv4() { 32 } else { 128 } }),
        Err(_) => netproto::dns::valid_name(host).then(|| Target::Host(String::from(host))),
    }
}

/// Port NTS-KE (RFC 8915; init nepotrebuje celou knihovnu NTS).
const NTS_KE_PORT: u16 = 4460;

/// Pravidla pro server `nts://jmeno[:port]`.
fn nts_rules(rest: &str) -> Option<Vec<Rule>> {
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => (h, p.parse().ok()?),
        _ => (rest, NTS_KE_PORT),
    };
    let target = host_target(host)?;
    Some(vec![
        Rule::port(proto::TCP, target.clone(), port),
        Rule::any_port(proto::UDP, target),
        Rule::port(proto::UDP, Target::Any, netproto::ntp::PORT),
    ])
}

/// Pravidlo cile pro server `adresa[:port]` nebo `jmeno[:port]`.
fn server_rule(server: &str) -> Option<Rule> {
    if let Ok(sa) = server.parse::<SocketAddr>() {
        let prefix = if sa.is_ipv4() { 32 } else { 128 };
        return Some(Rule::port(proto::UDP, Target::Net { addr: sa.ip(), prefix }, sa.port()));
    }
    if let Ok(ip) = server.parse::<IpAddr>() {
        let prefix = if ip.is_ipv4() { 32 } else { 128 };
        return Some(Rule::port(proto::UDP, Target::Net { addr: ip, prefix }, netproto::ntp::PORT));
    }
    let (host, port) = match server.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().ok()?),
        None => (server, netproto::ntp::PORT),
    };
    netproto::dns::valid_name(host).then(|| Rule::port(proto::UDP, Target::Host(String::from(host)), port))
}

pub fn start(startup: &Startup, net: &NetService, devices: &crate::system::Devices) -> Result<NtpService> {
    let fs = Initfs::open(startup)?;
    let mut text = String::from(
        core::str::from_utf8(fs.find("ntpd.toml").ok_or(Error::NotFound)?).map_err(|_| Error::Corrupted)?,
    );
    if let (Some(ntp), Some(ke)) = (test_port(&fs, "ntp"), test_port(&fs, "nts_ke")) {
        // Testy: NTS proti serveru hostitele, neoverene NTP jen v zaloze.
        text = format!(
            "servers = [\"nts://10.0.2.2:{ke}\", \"10.0.2.2:{ntp}\"]\nunauthenticated = \"never\"\nmin_poll = 2\nmax_poll = 8\nsamples = 2\n"
        );
    }
    let cfg = conf::parse(&text).map_err(|e| {
        println!("init: ntpd.toml: {e}");
        Error::InvalidArgs
    })?;
    let servers = cfg.get_strs("servers").ok_or(Error::InvalidArgs)?;
    let manifest = crate::apps::manifest(&fs, "ntpd")?.ok_or(Error::NotFound)?;
    let mut r = manifest.network.clone().ok_or(Error::InvalidArgs)?;
    let mut rules: Vec<Rule> = Vec::new();
    for s in &servers {
        let r = match s.strip_prefix("nts://") {
            Some(rest) => nts_rules(rest),
            None => server_rule(s).map(|r| vec![r]),
        };
        match r {
            Some(r) => rules.extend(r),
            None => {
                println!("init: ntpd.toml: neplatny server {s}");
                return Err(Error::InvalidArgs);
            }
        }
    }
    r.policy.connect = Some(rules);
    let session = net.root.restrict_with(&r)?;

    // Kanal k timed jen pro navrhy korekci.
    let (to_timed, timed_end) = Channel::create()?;
    {
        let guard = crate::system::TIME.lock();
        let t = guard.as_ref().ok_or(Error::NotFound)?;
        t.channel.write(b"ntp", vec![timed_end.into_handle()])?;
        let reply = t.channel.read(Some(time::deadline_after(Duration::from_secs(5))))?;
        if reply.data != b"ok" {
            println!("init: timed odmitl ntpd: {}", core::str::from_utf8(&reply.data).unwrap_or("?"));
            return Err(Error::IoError);
        }
    }
    let mut handles = vec![
        StartHandle::user(session.into_channel().into_handle()),
        StartHandle::user(to_timed.into_handle()),
    ];
    // TLS pro NTS-KE: korenove CA a kontrola odvolani (oboje volitelne;
    // bez CA se NTS nenavaze a plati pravidlo pro neoverene servery).
    match crate::certs::session(devices) {
        Ok(fs) => {
            handles.push(StartHandle::user(fs.into_channel().into_handle()));
            if let Some(r) = crate::revocation::client() {
                handles.push(StartHandle::user(r.into_handle()));
            }
        }
        Err(e) => println!("init: ntpd bez uloziste certifikatu (NTS nepujde): {e}"),
    }
    let app = crate::apps::spawn(startup, "ntpd", handles)?;
    app.control.write(text.as_bytes(), Vec::new())?;
    let msg = app.control.read(Some(time::deadline_after(Duration::from_secs(5))))?;
    if msg.data != b"ok" {
        println!("init: ntpd hlasi: {}", core::str::from_utf8(&msg.data).unwrap_or("?"));
        return Err(Error::IoError);
    }
    Ok(NtpService {
        process: app.process,
        control: app.control,
    })
}
