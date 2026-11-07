//! netprobe: testovaci aplikace site (testy firewallu v `cargo xtask test`).
//!
//! Dostane relaci site podle sveho manifestu (`netprobe.manifest.toml`)
//! a na bootstrap kanalu od init prikazy; na kazdy odpovi `ok[:data]` nebo
//! `err:<JMENO_CHYBY>`:
//! - `connect <adresa:port>` - spojeni TCP (a hned zavre),
//! - `udp <adresa:port>` - jeden datagram,
//! - `ping <adresa>`,
//! - `resolve <jmeno>` - systemovy resolver,
//! - `resolve@<adresa:port> <jmeno>` - vlastni server DNS.
//!
//! Konec kanalu u rodice = konec procesu.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::net::{IpAddr, SocketAddr};
use core::time::Duration;

use libos::{Channel, Error, Startup, println};
use netproto::Net;

libos::entry!(main);

fn main(startup: Startup) -> i64 {
    let ch = startup.bootstrap;
    let Some(net) = startup.handles.into_iter().next().map(|h| Net::new(Channel::from_handle(h))) else {
        let _ = ch.write(b"err:bez site", Vec::new());
        return 1;
    };
    loop {
        let msg = match ch.read(None) {
            Ok(m) => m,
            Err(Error::PeerClosed) => return 0,
            Err(e) => {
                println!("netprobe: cteni: {e}");
                return 2;
            }
        };
        let cmd = core::str::from_utf8(&msg.data).unwrap_or("");
        let reply = match run(&net, cmd) {
            Ok(s) if s.is_empty() => String::from("ok"),
            Ok(s) => format!("ok:{s}"),
            Err(e) => format!("err:{}", e.name()),
        };
        if ch.write(reply.as_bytes(), Vec::new()).is_err() {
            return 3;
        }
    }
}

fn run(net: &Net, cmd: &str) -> libos::Result<String> {
    let (verb, arg) = cmd.split_once(' ').unwrap_or((cmd, ""));
    // Rezerva pro beh testu pod TCG a zatezi (DNS, spojeni).
    let timeout = Some(Duration::from_secs(30));
    let sa = |s: &str| s.parse::<SocketAddr>().map_err(|_| Error::InvalidArgs);
    match verb {
        "connect" => {
            let s = net.tcp_connect(sa(arg)?, timeout)?;
            drop(s);
            Ok(String::new())
        }
        "udp" => {
            let sock = net.udp_bind(0)?;
            sock.send_to(b"netprobe", sa(arg)?)?;
            Ok(String::new())
        }
        "ping" => {
            let ip: IpAddr = arg.parse().map_err(|_| Error::InvalidArgs)?;
            net.ping(ip, 1, 8, Duration::from_secs(5))?;
            Ok(String::new())
        }
        "resolve" => {
            let addrs = net.resolve(arg)?;
            Ok(format!("{}", addrs.len()))
        }
        v if v.starts_with("resolve@") => {
            let server = sa(&v["resolve@".len()..])?;
            let (addrs, _) = net.resolve_with(arg, 4, Some(server), timeout)?;
            Ok(format!("{}", addrs.len()))
        }
        _ => Err(Error::NotSupported),
    }
}
