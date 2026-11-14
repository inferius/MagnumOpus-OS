//! Sluzba odvolani certifikatu `revocd`.
//!
//! - Mezipamet `/system/config/certs/crl` patri servisnimu uctu revocd
//!   ([`REVOCD_USER`]); ostatni ji smi jen cist (dedi prava uloziste).
//! - Relace FS: `/certs` jen pro cteni (korenove CA pro https), `/crl`
//!   pro zapis. Relace site podle manifestu (TCP 80/443 kamkoli, DNS);
//!   v testovacim behu jen pomocne servery hostitele.
//! - Programy s TLS dostanou vlastni kanal ([`client`]): revocd vidi jen
//!   dotazy (adresy CRL, CA), ne data spojeni.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr};
use core::time::Duration;

use fsproto::perm::{Mount, mount_flags};
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Process, Result, Startup, println, time};
use netproto::{Rule, Target, proto};

use crate::system::{Devices, NetService};

/// Servisni ucet revocd na FS (ucty sluzeb 900-999, fwd ma 900).
pub const REVOCD_USER: u64 = 901;
pub const CRL_DIR: &str = "/system/config/certs/crl";

pub struct Revocd {
    pub process: Process,
    pub control: Channel,
}

/// Port pomocneho serveru HTTP v testovacim behu (`net-test`).
fn test_http_port(fs: &Initfs) -> Option<u16> {
    let text = core::str::from_utf8(fs.find("net-test")?).ok()?;
    text.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == "http")
        .and_then(|(_, v)| v.trim().parse().ok())
}

fn fs_session(devices: &Devices) -> Result<Channel> {
    let fs = devices.fs.as_ref().ok_or(Error::NotFound)?;
    match fs.create(CRL_DIR, fsproto::kind::DIR) {
        Ok(()) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(e),
    }
    fs.set_owner(CRL_DIR, REVOCD_USER)?;
    let mounts = [
        Mount::new("/certs", crate::certs::DIR, mount_flags::READ_ONLY),
        Mount::new("/crl", CRL_DIR, 0),
    ];
    let (_, session) = fs.connect_as(REVOCD_USER, &[], &mounts)?;
    Ok(session.into_channel())
}

pub fn start(startup: &Startup, net: &NetService, devices: &Devices) -> Result<Revocd> {
    let initfs = Initfs::open(startup)?;
    let mut text = String::from(
        core::str::from_utf8(initfs.find("revocd.toml").ok_or(Error::NotFound)?).map_err(|_| Error::Corrupted)?,
    );
    let manifest = crate::apps::manifest(&initfs, "revocd")?.ok_or(Error::NotFound)?;
    let mut r = manifest.network.clone().ok_or(Error::InvalidArgs)?;
    if let Some(port) = test_http_port(&initfs) {
        // Testy: OneCRL a CRL z pomocneho serveru hostitele, nikam jinam.
        text = format!("onecrl = \"http://10.0.2.2:{port}/onecrl.json\"\nretry_minutes = 1\n");
        let host = Target::Net { addr: IpAddr::V4(Ipv4Addr::new(10, 0, 2, 2)), prefix: 32 };
        r.policy.connect = Some(vec![Rule::any_port(proto::TCP, host)]);
    }
    let session = net.root.restrict_with(&r)?;
    let mut handles = vec![StartHandle::user(session.into_channel()?.into_handle())];
    match fs_session(devices) {
        Ok(ch) => handles.push(StartHandle::user(ch.into_handle())),
        Err(e) => println!("init: revocd bez FS (mezipamet jen v pameti, bez CA pro https): {e}"),
    }
    let app = crate::apps::spawn(startup, "revocd", handles)?;
    app.control.write(text.as_bytes(), Vec::new())?;
    let msg = app.control.read(Some(time::deadline_after(Duration::from_secs(10))))?;
    if msg.data != b"ok" {
        println!("init: revocd hlasi: {}", core::str::from_utf8(&msg.data).unwrap_or("?"));
        return Err(Error::IoError);
    }
    Ok(Revocd {
        process: app.process,
        control: app.control,
    })
}

/// Kanal pro program s TLS (`tls::revocation::ChannelSource`); `None` =
/// revocd nebezi (program pak kontroluje jen OCSP stapling).
pub fn client() -> Option<Channel> {
    let r = crate::system::REVOCD.get()?;
    let (ours, theirs) = Channel::create().ok()?;
    let _guard = crate::system::REVOCD_LOCK.lock();
    r.control.write(b"client", vec![theirs.into_handle()]).ok()?;
    let reply = r.control.read(Some(time::deadline_after(Duration::from_secs(5)))).ok()?;
    (reply.data == b"ok").then_some(ours)
}
