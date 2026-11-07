//! Synchronizace casu: spusteni `ntpd`.
//!
//! - Konfigurace `ntpd.toml` z initfs (servery, intervaly). V testovacim
//!   behu (soubor `net-test`) jen pomocny server NTP hostitele s kratkymi
//!   intervaly.
//! - Relace site podle manifestu `ntpd.manifest.toml` a navic jen UDP na
//!   nakonfigurovane servery (nejmensi prava: ntpd nesmi nikam jinam).
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

/// Port pomocneho serveru NTP v testovacim behu (`net-test`).
fn test_ntp_port(fs: &Initfs) -> Option<u16> {
    let text = core::str::from_utf8(fs.find("net-test")?).ok()?;
    text.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == "ntp")
        .and_then(|(_, v)| v.trim().parse().ok())
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

pub fn start(startup: &Startup, net: &NetService) -> Result<NtpService> {
    let fs = Initfs::open(startup)?;
    let mut text = String::from(
        core::str::from_utf8(fs.find("ntpd.toml").ok_or(Error::NotFound)?).map_err(|_| Error::Corrupted)?,
    );
    if let Some(port) = test_ntp_port(&fs) {
        text = format!("servers = [\"10.0.2.2:{port}\"]\nmin_poll = 2\nmax_poll = 8\nsamples = 2\n");
    }
    let cfg = conf::parse(&text).map_err(|e| {
        println!("init: ntpd.toml: {e}");
        Error::InvalidArgs
    })?;
    let servers = cfg.get_strs("servers").ok_or(Error::InvalidArgs)?;
    let manifest = crate::apps::manifest(&fs, "ntpd")?.ok_or(Error::NotFound)?;
    let mut r = manifest.network.clone().ok_or(Error::InvalidArgs)?;
    let rules: Vec<Rule> = servers.iter().filter_map(|s| server_rule(s)).collect();
    if rules.len() != servers.len() {
        println!("init: ntpd.toml: neplatny server");
        return Err(Error::InvalidArgs);
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
    let app = crate::apps::spawn(
        startup,
        "ntpd",
        vec![
            StartHandle::user(session.into_channel().into_handle()),
            StartHandle::user(to_timed.into_handle()),
        ],
    )?;
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
