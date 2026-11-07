//! Testy formatu a opravneni (na hostiteli:
//! `cargo test -p netproto --no-default-features`).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use abi::Error;
use abi::time::WallTime;

use crate::policy::{Auth, Policy, Rule, Target, host_matches, in_network, proto, rights};
use crate::{Info, ReadAddr, Reader, WriteAddr, Writer, dns, link, ntp, parse_reply, reply};

fn v4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(a, b, c, d))
}

fn sa(ip: IpAddr, port: u16) -> SocketAddr {
    SocketAddr::new(ip, port)
}

#[test]
fn codec_roundtrip_addresses() {
    let a6 = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));
    let mut w = Writer::new();
    w.ip(v4(10, 0, 2, 15))
        .socket_addr(sa(a6, 443))
        .str("ahoj")
        .u64(7);
    let b = w.finish();
    let mut r = Reader::new(&b);
    assert_eq!(r.ip().unwrap(), v4(10, 0, 2, 15));
    assert_eq!(r.socket_addr().unwrap(), sa(a6, 443));
    assert_eq!(r.str().unwrap(), "ahoj");
    assert_eq!(r.u64().unwrap(), 7);
    assert!(r.is_empty());
    // Neplatna rodina adresy a zkracena zprava.
    assert_eq!(Reader::new(&[5, 1, 2, 3, 4]).ip(), Err(Error::InvalidArgs));
    assert_eq!(Reader::new(&[4, 1, 2]).ip(), Err(Error::InvalidArgs));
}

#[test]
fn reply_status() {
    let mut body = Writer::new();
    body.u32(42);
    let ok = reply(Ok(body));
    assert_eq!(parse_reply(&ok).unwrap().u32().unwrap(), 42);
    let err = reply(Err(Error::ConnectionRefused));
    assert_eq!(parse_reply(&err).err(), Some(Error::ConnectionRefused));
}

#[test]
fn info_roundtrip() {
    let i = Info {
        mac: [0x52, 0x54, 0, 0x12, 0x34, 0x56],
        mtu: 1500,
        link_up: true,
        configured: true,
        addr: Some((v4(10, 0, 2, 15), 24)),
        gateway: Some(v4(10, 0, 2, 2)),
        dns: vec![v4(10, 0, 2, 3)],
        rights: rights::CLIENT,
    };
    let mut w = Writer::new();
    i.put(&mut w);
    let b = w.finish();
    assert_eq!(Info::get(&mut Reader::new(&b)).unwrap(), i);
}

#[test]
fn network_prefixes() {
    assert!(in_network(v4(10, 0, 2, 99), v4(10, 0, 2, 0), 24));
    assert!(!in_network(v4(10, 0, 3, 1), v4(10, 0, 2, 0), 24));
    assert!(in_network(v4(1, 2, 3, 4), v4(0, 0, 0, 0), 0));
    assert!(in_network(v4(10, 0, 2, 2), v4(10, 0, 2, 2), 32));
    // Ruzne rodiny se nikdy neshoduji.
    assert!(!in_network(
        IpAddr::V6(Ipv6Addr::LOCALHOST),
        v4(0, 0, 0, 0),
        0
    ));
}

#[test]
fn host_patterns() {
    assert!(host_matches("api.example.com", "API.Example.com."));
    assert!(!host_matches("api.example.com", "x.api.example.com"));
    assert!(host_matches("*.example.com", "a.b.example.com"));
    assert!(!host_matches("*.example.com", "example.com"));
    assert!(!host_matches("*.example.com", "badexample.com"));
}

#[test]
fn root_allows_everything() {
    let a = Auth::root();
    assert!(a.has(rights::ALL));
    assert!(a.allows(proto::TCP, sa(v4(8, 8, 8, 8), 443)));
    assert!(a.allows_bind(80));
}

#[test]
fn restrict_rights_and_targets() {
    let root = Auth::root();
    let lan = Policy {
        connect: Some(vec![Rule::any_port(
            proto::TCP | proto::UDP,
            Target::Net {
                addr: v4(10, 0, 2, 0),
                prefix: 24,
            },
        )]),
        bind: Some(vec![(8000, 8099)]),
    };
    let a = root.restrict(rights::CLIENT, lan).unwrap();
    assert!(a.has(rights::TCP_CONNECT));
    assert!(!a.has(rights::PING));
    assert!(!a.has(rights::RESTRICT));
    assert_eq!(a.require(rights::TCP_LISTEN), Err(Error::AccessDenied));
    assert!(a.allows(proto::TCP, sa(v4(10, 0, 2, 2), 22)));
    assert!(!a.allows(proto::TCP, sa(v4(10, 0, 3, 2), 22)));
    assert!(!a.allows(proto::ICMP, sa(v4(10, 0, 2, 2), 0)));
    assert!(a.allows_bind(8080));
    assert!(a.allows_bind(0));
    assert!(!a.allows_bind(80));

    // Druha vrstva muze jen ubrat: port 80 v siti z prvni vrstvy.
    let web = Policy::only(vec![Rule::port(proto::TCP, Target::Any, 80)]);
    let b = a.restrict(rights::ALL, web).unwrap();
    assert_eq!(b.rights, a.rights, "prava nejdou rozsirit");
    assert!(b.allows(proto::TCP, sa(v4(10, 0, 2, 2), 80)));
    assert!(!b.allows(proto::TCP, sa(v4(10, 0, 2, 2), 22)));
    assert!(
        !b.allows(proto::TCP, sa(v4(1, 1, 1, 1), 80)),
        "mimo sit prvni vrstvy"
    );
    assert!(!b.allows_bind(8080), "Policy::only zakazuje naslouchani");
}

#[test]
fn host_rules_use_learned_addresses() {
    let p = Policy::only(vec![Rule::port(
        proto::TCP,
        Target::Host(String::from("*.example.com")),
        443,
    )]);
    let mut a = Auth::root().restrict(rights::CLIENT, p).unwrap();
    assert!(a.has_host_rules());
    let ip = v4(93, 184, 216, 34);
    assert!(!a.allows(proto::TCP, sa(ip, 443)), "pred prekladem nic");
    a.learn("www.example.com", &[ip]);
    assert!(a.allows(proto::TCP, sa(ip, 443)));
    assert!(!a.allows(proto::TCP, sa(ip, 80)), "jiny port");
    a.learn("evil.test", &[v4(6, 6, 6, 6)]);
    assert!(
        !a.allows(proto::TCP, sa(v4(6, 6, 6, 6), 443)),
        "jmeno mimo vzor"
    );
}

#[test]
fn learned_is_bounded() {
    let mut a = Auth::root();
    for i in 0..(crate::policy::MAX_LEARNED as u32 + 10) {
        a.learn("x.test", &[IpAddr::V4(Ipv4Addr::from(i))]);
    }
    // Interni seznam je omezeny; nejnovejsi zaznam plati.
    let p = Policy::only(vec![Rule::any_port(
        proto::TCP,
        Target::Host(String::from("x.test")),
    )]);
    let b = a.restrict(rights::ALL, p).unwrap();
    let last = IpAddr::V4(Ipv4Addr::from(crate::policy::MAX_LEARNED as u32 + 9));
    assert!(b.allows(proto::TCP, sa(last, 1)));
    assert!(!b.allows(proto::TCP, sa(IpAddr::V4(Ipv4Addr::from(0u32)), 1)));
}

#[test]
fn policy_wire_roundtrip_and_validation() {
    let p = Policy {
        connect: Some(vec![
            Rule::any_port(proto::ALL, Target::Any),
            Rule::port(
                proto::UDP,
                Target::Net {
                    addr: v4(10, 0, 0, 0),
                    prefix: 8,
                },
                53,
            ),
            Rule::port(proto::TCP, Target::Host(String::from("*.example.com")), 443),
        ]),
        bind: Some(vec![(1024, 2048)]),
    };
    let mut w = Writer::new();
    p.put(&mut w);
    let b = w.finish();
    assert_eq!(Policy::get(&mut Reader::new(&b)).unwrap(), p);

    let bad = [
        Policy::only(vec![Rule::any_port(0, Target::Any)]),
        Policy::only(vec![Rule {
            protos: proto::TCP,
            target: Target::Any,
            ports: (10, 5),
        }]),
        Policy::only(vec![Rule::any_port(
            proto::TCP,
            Target::Net {
                addr: v4(1, 2, 3, 4),
                prefix: 33,
            },
        )]),
        Policy::only(vec![Rule::any_port(
            proto::TCP,
            Target::Host(String::from("bad name")),
        )]),
    ];
    for p in bad {
        assert_eq!(p.validate(), Err(Error::InvalidArgs), "{p:?}");
        assert!(Auth::root().restrict(rights::ALL, p).is_err());
    }
}

#[test]
fn dns_names() {
    assert!(dns::valid_name("example.com"));
    assert!(dns::valid_name("example.com."));
    assert!(dns::valid_name("_sip._tcp.example.com"));
    assert!(!dns::valid_name(""));
    assert!(!dns::valid_name("a..b"));
    assert!(!dns::valid_name("-a.b"));
    assert!(!dns::valid_name(&"a".repeat(64)));
    assert!(!dns::valid_name("ahoj svete"));
}

#[test]
fn dns_query_response_roundtrip() {
    let q = dns::build_query(0x1234, "test.magnumopus", dns::TYPE_A).unwrap();
    let parsed = dns::parse_query(&q).unwrap();
    assert_eq!(parsed.id, 0x1234);
    assert_eq!(parsed.name, "test.magnumopus");
    assert_eq!(parsed.qtype, dns::TYPE_A);

    let addrs = [v4(10, 0, 2, 123), IpAddr::V6(Ipv6Addr::LOCALHOST)];
    let resp = dns::build_response(&parsed, &addrs, 60, dns::RCODE_OK);
    let ans = dns::parse_response(0x1234, &resp).unwrap();
    assert_eq!(ans.addrs, vec![v4(10, 0, 2, 123)], "jen zaznamy typu A");
    assert_eq!(ans.ttl, 60);

    // Cizi ID, NXDOMAIN, poskozena zprava.
    assert_eq!(dns::parse_response(0x9999, &resp), Err(Error::InvalidArgs));
    let nx = dns::build_response(&parsed, &[], 0, dns::RCODE_NXDOMAIN);
    assert_eq!(dns::parse_response(0x1234, &nx), Err(Error::NotFound));
    let sf = dns::build_response(&parsed, &[], 0, dns::RCODE_SERVFAIL);
    assert_eq!(dns::parse_response(0x1234, &sf), Err(Error::IoError));
    assert!(dns::parse_response(0x1234, &resp[..resp.len() - 3]).is_err());
}

#[test]
fn dns_compression_loop_is_rejected() {
    // Hlavicka odpovedi s jednou otazkou, jejiz jmeno je ukazatel sam na sebe.
    let mut d = vec![0x12, 0x34, 0x81, 0x80, 0, 1, 0, 0, 0, 0, 0, 0];
    d.extend_from_slice(&[0xC0, 12, 0, 1, 0, 1]);
    assert_eq!(dns::parse_response(0x1234, &d), Err(Error::Corrupted));
}

#[test]
fn dns_cname_then_address() {
    // Odpoved: CNAME a pak A (rucne sestavena, s kompresi jmen).
    let q = dns::build_query(7, "www.test", dns::TYPE_A).unwrap();
    let mut d = q.clone();
    d[2] = 0x81;
    d[3] = 0x80;
    d[7] = 2; // ANCOUNT
    // CNAME: www.test -> cdn.test
    d.extend_from_slice(&[
        0xC0, 12, 0, 5, 0, 1, 0, 0, 0, 30, 0, 6, 3, b'c', b'd', b'n', 0xC0, 16,
    ]);
    let cdn_at = d.len() - 6;
    // A pro cdn.test
    d.extend_from_slice(&[
        0xC0,
        cdn_at as u8,
        0,
        1,
        0,
        1,
        0,
        0,
        0,
        10,
        0,
        4,
        1,
        2,
        3,
        4,
    ]);
    let ans = dns::parse_response(7, &d).unwrap();
    assert_eq!(ans.addrs, vec![v4(1, 2, 3, 4)]);
    assert_eq!(ans.ttl, 10);
}

#[test]
fn ntp_timestamps() {
    let t = WallTime {
        secs: 1_794_000_000,
        nanos: 250_000_000,
    };
    let back = ntp::from_ntp(ntp::to_ntp(t));
    assert_eq!(back.secs, t.secs);
    assert!(back.nanos.abs_diff(t.nanos) < 2);
    // Po preteceni v roce 2036 (era 1).
    let t2040 = WallTime {
        secs: 2_208_988_800,
        nanos: 0,
    };
    assert_eq!(ntp::from_ntp(ntp::to_ntp(t2040)), t2040);
}

#[test]
fn ntp_exchange() {
    let t1 = WallTime {
        secs: 1_000,
        nanos: 0,
    };
    let req = ntp::build_request(t1);
    // Server jde o 10 s napred, cesta trva 2 x 50 ms.
    let t2 = WallTime {
        secs: 1_010,
        nanos: 50_000_000,
    };
    let t3 = WallTime {
        secs: 1_010,
        nanos: 60_000_000,
    };
    let reply = ntp::build_reply(&req, t2, t3).unwrap();
    let r = ntp::parse_reply(t1, &reply).unwrap();
    let t4 = WallTime {
        secs: 1_000,
        nanos: 110_000_000,
    };
    let (offset, delay) = ntp::offset_delay(t1, &r, t4);
    assert!((offset - 10_000_000_000).abs() < 10, "posun {offset}");
    assert!((delay - 100_000_000).abs() < 10, "zpozdeni {delay}");
    let c = ntp::corrected(t4, offset);
    assert_eq!(c.secs, 1_010);

    // Odpoved na jiny pozadavek a server bez synchronizace.
    let other = WallTime { secs: 5, nanos: 0 };
    assert_eq!(ntp::parse_reply(other, &reply), Err(Error::InvalidArgs));
    let mut unsync = reply;
    unsync[1] = 0;
    assert_eq!(ntp::parse_reply(t1, &unsync), Err(Error::BadState));
    assert!(
        ntp::build_reply(&reply, t2, t3).is_err(),
        "odpoved neni pozadavek"
    );
}

#[test]
fn link_info_roundtrip() {
    let i = link::Info {
        mac: [2, 0, 0, 0, 0, 1],
        mtu: 1500,
        flags: link::FLAG_UP,
    };
    assert_eq!(link::Info::decode(&i.encode()).unwrap(), i);
    assert!(link::Info::decode(&[link::FRAME; 10]).is_err());
}

#[test]
fn timeout_conversion() {
    use core::time::Duration;
    assert_eq!(crate::timeout_ms(None), crate::FOREVER);
    assert_eq!(crate::timeout_ms(Some(Duration::ZERO)), 0);
    assert_eq!(crate::timeout_ms(Some(Duration::from_micros(1))), 1);
    assert_eq!(crate::timeout_ms(Some(Duration::from_secs(2))), 2000);
    let _: Vec<u8> = Vec::new();
}
