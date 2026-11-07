//! Sifrovane DNS a DNSSEC: systemovy preklad pres dnsd proti resolverum
//! DoH a DoT hostitele (`xtask/src/dnshelper.rs`) s podepsanou testovaci
//! hierarchii.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use core::time::Duration;

use libos::{Error, Startup, time};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("DNSSEC: overene adresy (A, CNAME, wildcard, AAAA)", secure),
    ("DNSSEC: neexistence overena (NXDOMAIN)", nxdomain),
    ("DNSSEC: nepodepsana zona projde, podvrzena ne", insecure_and_bogus),
    ("dnsd: zalozni resolver po nefunkcnim, DoT", transports),
    ("profil site: neoverene DNS a NTP jen v zaloze mimo verejnou sit", profile_rule),
];

fn ready() -> Result<Option<(&'static netproto::Net, &'static crate::dns::Dnsd)>, String> {
    let Some(d) = crate::system::DNSD.get() else {
        libos::println!("[preskoceno] dns: dnsd nebezi");
        return Ok(None);
    };
    let net = super::net::net()?;
    super::net::configured(net)?;
    Ok(Some((net, d)))
}

fn status(d: &crate::dns::Dnsd, cmd: &[u8]) -> Result<String, String> {
    d.control.write(cmd, Vec::new()).map_err(|e| fail("dnsd", e))?;
    let m = d
        .control
        .read(Some(time::deadline_after(Duration::from_secs(10))))
        .map_err(|e| fail("dnsd", e))?;
    Ok(String::from_utf8(m.data).unwrap_or_default())
}

fn resolve(net: &netproto::Net, name: &str, family: u8) -> Result<Vec<IpAddr>, Error> {
    net.resolve_with(name, family, None, Some(Duration::from_secs(60))).map(|(a, _)| a)
}

fn v4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(a, b, c, d))
}

fn secure(_: &Startup) -> TestResult {
    let Some((net, d)) = ready()? else {
        return Ok(());
    };
    for (name, want) in [
        ("www.example.test", v4(10, 0, 2, 50)),
        ("alias.example.test", v4(10, 0, 2, 50)),
        ("cokoli.wild.example.test", v4(10, 0, 2, 51)),
        ("host.test", v4(10, 0, 2, 54)),
    ] {
        let a = resolve(net, name, 4).map_err(|e| fail(name, e))?;
        check_eq!(a, vec![want], name);
    }
    let v6 = resolve(net, "v6.example.test", 0).map_err(|e| fail("v6", e))?;
    check_eq!(v6, vec![IpAddr::V6(Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1))], "AAAA (A neexistuje)");
    let s = status(d, b"status")?;
    check!(s.contains("bogus=0") && !s.contains("secure=0 "), format!("stav: {s}"));
    Ok(())
}

fn nxdomain(_: &Startup) -> TestResult {
    let Some((net, _)) = ready()? else {
        return Ok(());
    };
    check_eq!(resolve(net, "nope.example.test", 0).err(), Some(Error::NotFound), "NSEC3");
    check_eq!(resolve(net, "nope.test", 4).err(), Some(Error::NotFound), "NSEC");
    check_eq!(resolve(net, "www.example.test", 6).err(), Some(Error::NotFound), "NODATA");
    Ok(())
}

fn insecure_and_bogus(_: &Startup) -> TestResult {
    let Some((net, d)) = ready()? else {
        return Ok(());
    };
    let a = resolve(net, "www.unsigned.test", 4).map_err(|e| fail("nepodepsana", e))?;
    check_eq!(a, vec![v4(10, 0, 2, 52)], "nepodepsana zona");
    check_eq!(resolve(net, "www.bad.test", 4).err(), Some(Error::Corrupted), "podvrzena delegace");
    let s = status(d, b"status")?;
    check!(!s.contains("bogus=0") && !s.contains("insecure=0"), format!("stav: {s}"));
    Ok(())
}

fn transports(_: &Startup) -> TestResult {
    let Some((net, d)) = ready()? else {
        return Ok(());
    };
    // Prvni resolver (zavreny port) nefunguje, dnsd presel na DoH.
    let s = status(d, b"status")?;
    check!(s.contains("server=DoH"), format!("po nefunkcnim resolveru: {s}"));
    let r = status(d, b"prefer 2")?;
    check_eq!(r.as_str(), "ok", "prefer DoT");
    let a = resolve(net, "www.example.test", 4).map_err(|e| fail("pres DoT", e))?;
    check_eq!(a, vec![v4(10, 0, 2, 50)], "DoT");
    let s = status(d, b"status")?;
    check!(s.contains("server=DoT"), format!("DoT: {s}"));
    // Mezipamet: dalsi dotaz bez resolveru.
    let before = s.split_whitespace().find_map(|w| w.strip_prefix("upstream=")).map(String::from).unwrap_or_default();
    resolve(net, "www.example.test", 4).map_err(|e| fail("z mezipameti", e))?;
    let s = status(d, b"status")?;
    let after = s.split_whitespace().find_map(|w| w.strip_prefix("upstream=")).unwrap_or("");
    check_eq!(after, before.as_str(), "druhy dotaz z mezipameti");
    let _ = status(d, b"prefer 1");
    Ok(())
}

fn profile_rule(_: &Startup) -> TestResult {
    let Some((net, _)) = ready()? else {
        return Ok(());
    };
    // Pravidlo odpovida aktualnimu profilu (firewall.toml: public = never,
    // home a work = fallback); profil mohly zmenit testy firewallu.
    let info = net.info().map_err(|e| fail("info", e))?;
    let (profile, _) = net.stats().map_err(|e| fail("stats", e))?;
    check_eq!(info.unauthenticated_fallback, profile != "public", format!("profil {profile}"));
    // ntpd sleduje profil (vychozi), v testech ale ma "never".
    if let Some(ntp) = crate::system::NTP.get() {
        ntp.control.write(b"status", Vec::new()).map_err(|e| fail("ntpd", e))?;
        let m = ntp
            .control
            .read(Some(time::deadline_after(Duration::from_secs(5))))
            .map_err(|e| fail("ntpd", e))?;
        let st = String::from_utf8(m.data).unwrap_or_default();
        check!(st.contains("policy=Never"), format!("ntpd: {st}"));
    }
    Ok(())
}
