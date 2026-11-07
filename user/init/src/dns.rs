//! Systemovy resolver `dnsd` (sifrovane DNS a DNSSEC).
//!
//! - Kanal netd <-> dnsd vznika pred startem netd (`system::start_net`):
//!   netd dostane jeden konec jako 4. handle, dnsd druhy. Dotazy pred
//!   startem dnsd cekaji v kanalu.
//! - Relace site dnsd: jen TCP na adresy resolveru z `dnsd.toml` (DoH 443,
//!   DoT 853), zadny systemovy preklad (sel by zpet do dnsd).
//! - V testovacim behu: resolvery DoH a DoT na hostiteli a kotva testovaci
//!   hierarchie z initfs (`net-test-dns-anchor`).

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

use crate::system::{Devices, NetService};

pub struct Dnsd {
    pub process: Process,
    pub control: Channel,
}

fn test_port(fs: &Initfs, key: &str) -> Option<u16> {
    let text = core::str::from_utf8(fs.find("net-test")?).ok()?;
    text.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .and_then(|(_, v)| v.trim().parse().ok())
}

/// Cil TCP pro server `doh:url@adresa` nebo `dot:jmeno@adresa[:port]`.
fn server_target(s: &str) -> Option<SocketAddr> {
    if let Some(rest) = s.strip_prefix("doh:") {
        let (url, addr) = rest.rsplit_once('@')?;
        let port = httpc_port(url)?;
        return Some(SocketAddr::new(addr.parse().ok()?, port));
    }
    let (_, addr) = s.strip_prefix("dot:")?.rsplit_once('@')?;
    addr.parse::<SocketAddr>()
        .ok()
        .or_else(|| addr.parse::<IpAddr>().ok().map(|ip| SocketAddr::new(ip, 853)))
}

/// Port z adresy `https://jmeno[:port]/...`.
fn httpc_port(url: &str) -> Option<u16> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split('/').next()?;
    // [IPv6]:port, jmeno:port, nebo bez portu.
    let port = match authority.strip_prefix('[') {
        Some(v6) => v6.split_once("]:").map(|(_, p)| p),
        None => authority.rsplit_once(':').map(|(_, p)| p),
    };
    match port {
        Some(p) => p.parse().ok(),
        None => Some(443),
    }
}

pub fn start(startup: &Startup, net: &NetService, netd_end: Channel, devices: &Devices) -> Result<Dnsd> {
    let fs = Initfs::open(startup)?;
    let mut text = String::from(
        core::str::from_utf8(fs.find("dnsd.toml").ok_or(Error::NotFound)?).map_err(|_| Error::Corrupted)?,
    );
    if let (Some(https), Some(dot), Some(closed), Some(anchor)) = (
        test_port(&fs, "https"),
        test_port(&fs, "dot"),
        test_port(&fs, "closed"),
        fs.find("net-test-dns-anchor").and_then(|a| core::str::from_utf8(a).ok()),
    ) {
        // Testy: DoH a DoT hostitele; prvni server nefunguje (zavreny port)
        // - dnsd musi prejit na dalsi.
        text = format!(
            "servers = [\"dot:10.0.2.2@10.0.2.2:{closed}\", \"doh:https://10.0.2.2:{https}/dns-query@10.0.2.2\", \"dot:10.0.2.2@10.0.2.2:{dot}\"]\ndnssec = true\ntrust_anchors = [\"{}\"]\n",
            anchor.trim()
        );
    }
    let cfg = conf::parse(&text).map_err(|e| {
        println!("init: dnsd.toml: {e}");
        Error::InvalidArgs
    })?;
    let servers = cfg.get_strs("servers").ok_or(Error::InvalidArgs)?;
    let mut rules = Vec::new();
    for s in &servers {
        let Some(t) = server_target(s) else {
            println!("init: dnsd.toml: neplatny server {s}");
            return Err(Error::InvalidArgs);
        };
        let prefix = if t.is_ipv4() { 32 } else { 128 };
        rules.push(Rule::port(proto::TCP, Target::Net { addr: t.ip(), prefix }, t.port()));
    }
    let manifest = crate::apps::manifest(&fs, "dnsd")?.ok_or(Error::NotFound)?;
    let mut r = manifest.network.clone().ok_or(Error::InvalidArgs)?;
    r.policy.connect = Some(rules);
    let session = net.root.restrict_with(&r)?;
    let mut handles = vec![
        StartHandle::user(session.into_channel().into_handle()),
        StartHandle::user(netd_end.into_handle()),
    ];
    match crate::certs::session(devices) {
        Ok(c) => handles.push(StartHandle::user(c.into_channel().into_handle())),
        Err(e) => println!("init: dnsd bez uloziste certifikatu (sifrovane DNS nepujde): {e}"),
    }
    let app = crate::apps::spawn(startup, "dnsd", handles)?;
    app.control.write(text.as_bytes(), Vec::new())?;
    let msg = app.control.read(Some(time::deadline_after(Duration::from_secs(10))))?;
    if msg.data != b"ok" {
        println!("init: dnsd hlasi: {}", core::str::from_utf8(&msg.data).unwrap_or("?"));
        return Err(Error::IoError);
    }
    Ok(Dnsd {
        process: app.process,
        control: app.control,
    })
}
