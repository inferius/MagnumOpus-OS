//! `std::net` v aplikaci se std (docs/cs/rust-std.md, faze 2): `std-net`
//! dostane omezenou relaci site (TCP jen na echo server hostitele, UDP na
//! echo, preklad jmen) a overi TCP, vlakna, UDP, DNS a odmitnuti mimo
//! povoleni. Pomocne servery spousti `cargo xtask test`.

use alloc::format;
use alloc::vec;
use core::time::Duration;

use libos::abi::params::tag;
use libos::spawn::StartHandle;
use libos::Startup;
use netproto::{Policy, Rule, Target, proto, rights};

use super::net::{HOST, configured, host, net, ports};
use super::std_app::run_std_app;
use super::{Test, TestResult, fail};

pub const TESTS: &[Test] = &[("std-net: TCP, vlakna, UDP, DNS, opravneni", std_net)];

fn std_net(s: &Startup) -> TestResult {
    let Some(p) = ports(s, "std-net")? else {
        return Ok(());
    };
    let root = net()?;
    configured(root)?;
    let target = Target::Net { addr: HOST, prefix: 32 };
    let policy = Policy::only(vec![
        Rule::port(proto::TCP, target.clone(), p.echo),
        Rule::port(proto::UDP, target, p.udp_echo),
    ]);
    // Prava klienta (TCP, UDP, preklad jmen), bez LISTEN.
    let session = root.restrict(rights::CLIENT, &policy).map_err(|e| fail("restrict", e))?;
    let env = [
        format!("ECHO={}", host(p.echo)),
        format!("UDP_ECHO={}", host(p.udp_echo)),
        format!("CLOSED={}", host(p.closed)),
        format!("NAME=echo.magnumopus"),
    ];
    let env: alloc::vec::Vec<&str> = env.iter().map(|e| e.as_str()).collect();
    let run = run_std_app(
        s,
        "std-net",
        &[],
        &env,
        vec![StartHandle::tagged(tag::NET, session.into_channel().map_err(|e| format!("predani site: {e:?}"))?.into_handle())],
        Duration::from_secs(120),
    )?;
    run.expect("std-net", &["tcp_echo", "tcp_threads", "denied", "udp_echo", "dns", "listen_denied", "shared_stream_serialized"])
}
