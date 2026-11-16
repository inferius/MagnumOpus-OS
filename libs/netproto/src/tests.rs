//! Testy formatu a opravneni (na hostiteli:
//! `cargo test -p netproto --no-default-features`).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use abi::Error;
use abi::time::WallTime;

use crate::auth::{Auth, Quota, Restriction, UserRules, Verdict, layer_flags};
use crate::policy::{BindScope, Policy, Rule, Target, host_matches, in_network, proto, rights};
use crate::{Info, ReadAddr, Reader, WriteAddr, Writer, dns, link, ntp};

fn v4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(a, b, c, d))
}

fn sa(ip: IpAddr, port: u16) -> SocketAddr {
    SocketAddr::new(ip, port)
}

/// Monotonicky cas testu (ns).
const NOW: u64 = 1_000_000_000_000;

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
        unauthenticated_fallback: true,
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
    assert!(a.allows(proto::TCP, sa(v4(8, 8, 8, 8), 443), NOW));
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
        bind_scope: BindScope::Any,
    };
    let a = root.restrict(rights::CLIENT, lan).unwrap();
    assert!(a.has(rights::TCP_CONNECT));
    assert!(!a.has(rights::PING));
    assert!(!a.has(rights::RESTRICT));
    assert_eq!(a.require(rights::TCP_LISTEN), Err(Error::AccessDenied));
    assert!(a.allows(proto::TCP, sa(v4(10, 0, 2, 2), 22), NOW));
    assert!(!a.allows(proto::TCP, sa(v4(10, 0, 3, 2), 22), NOW));
    assert!(!a.allows(proto::ICMP, sa(v4(10, 0, 2, 2), 0), NOW));
    assert!(a.allows_bind(8080));
    assert!(a.allows_bind(0));
    assert!(!a.allows_bind(80));

    // Druha vrstva muze jen ubrat: port 80 v siti z prvni vrstvy.
    let web = Policy::only(vec![Rule::port(proto::TCP, Target::Any, 80)]);
    let b = a.restrict(rights::ALL, web).unwrap();
    assert_eq!(b.rights, a.rights, "prava nejdou rozsirit");
    assert!(b.allows(proto::TCP, sa(v4(10, 0, 2, 2), 80), NOW));
    assert!(!b.allows(proto::TCP, sa(v4(10, 0, 2, 2), 22), NOW));
    assert!(
        !b.allows(proto::TCP, sa(v4(1, 1, 1, 1), 80), NOW),
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
    assert!(a.has_host_rules(&UserRules::default()));
    let ip = v4(93, 184, 216, 34);
    assert!(!a.allows(proto::TCP, sa(ip, 443), NOW), "pred prekladem nic");
    a.learn("www.example.com", &[ip], 60, NOW);
    assert!(a.allows(proto::TCP, sa(ip, 443), NOW));
    assert!(!a.allows(proto::TCP, sa(ip, 80), NOW), "jiny port");
    a.learn("evil.test", &[v4(6, 6, 6, 6)], 60, NOW);
    assert!(
        !a.allows(proto::TCP, sa(v4(6, 6, 6, 6), 443), NOW),
        "jmeno mimo vzor"
    );
}

#[test]
fn learned_is_bounded_and_expires() {
    let mut a = Auth::root();
    let max = crate::auth::MAX_LEARNED as u32;
    for i in 0..(max + 10) {
        a.learn("x.test", &[IpAddr::V4(Ipv4Addr::from(i))], 60, NOW + i as u64);
    }
    // Interni seznam je omezeny; nejnovejsi zaznam plati.
    let p = Policy::only(vec![Rule::any_port(
        proto::TCP,
        Target::Host(String::from("x.test")),
    )]);
    let b = a.restrict(rights::ALL, p).unwrap();
    let last = IpAddr::V4(Ipv4Addr::from(max + 9));
    assert!(b.allows(proto::TCP, sa(last, 1), NOW));
    assert!(!b.allows(proto::TCP, sa(IpAddr::V4(Ipv4Addr::from(0u32)), 1), NOW));
    // Platnost: TTL 60 s se prodlouzi na LEARNED_MIN_TTL, pak vyprsi.
    let min = crate::auth::LEARNED_MIN_TTL * 1_000_000_000;
    assert!(b.allows(proto::TCP, sa(last, 1), NOW + min));
    assert!(!b.allows(proto::TCP, sa(last, 1), NOW + 2 * min));
}

#[test]
fn session_budget_is_hierarchical() {
    // Relace sama stoji jeden socket a SESSION_COST bajtu rodice.
    let session = crate::auth::SESSION_COST;
    let root = Auth::root_with(Quota {
        sockets: 4,
        bytes: 1000 + session,
        ops: 2,
    });
    let child = root
        .restrict_with(Restriction {
            rights: rights::ALL,
            quota: Quota {
                sockets: 10,
                bytes: 600,
                ops: 5,
            },
            ..Restriction::default()
        })
        .unwrap();
    let a = child.charge(500).unwrap();
    assert_eq!(child.charge(200).err(), Some(Error::NoResources), "limit relace");
    let b = root.charge(400).unwrap();
    assert_eq!(root.budget().used(), (3, 900 + session), "predek vidi potomka");
    assert_eq!(root.charge(200).err(), Some(Error::NoResources), "bajty predka");
    let c = child.charge(10).unwrap();
    assert_eq!(child.charge(0).err(), Some(Error::NoResources), "sockety predka");
    drop((a, b, c));
    assert_eq!(root.budget().used(), (1, session), "strzeni socketu se vratilo");
    drop(child);
    assert_eq!(root.budget().used(), (0, 0), "zanikla relace se vratila");
}

/// Rozpocet identity: sockety vsech relaci stejne identity dohromady
/// (zuzena relace ho zdedi), kazda relace ma presto svuj limit.
#[test]
fn identity_budget_spans_sessions() {
    let root = Auth::root();
    let id = crate::auth::Budget::identity(Quota { sockets: 3, bytes: u64::MAX, ops: 4 });
    let mut a = root.restrict_with(Restriction { rights: rights::ALL, ..Restriction::default() }).unwrap();
    a.set_identity(id.clone());
    let mut b = root.restrict_with(Restriction { rights: rights::ALL, ..Restriction::default() }).unwrap();
    b.set_identity(id.clone());
    // Potomek (role Net) zdedi rozpocet identity.
    let child = a.restrict_with(Restriction { rights: rights::ALL, ..Restriction::default() }).unwrap();
    let x = a.charge(10).unwrap();
    let y = b.charge(10).unwrap();
    let z = child.charge(10).unwrap();
    assert_eq!(b.charge(0).err(), Some(Error::NoResources), "limit identity pres relace");
    assert_eq!(id.used(), (3, 30));
    drop((x, y, z));
    assert_eq!(id.used(), (0, 0), "strzeni identity se vratila");
}

#[test]
fn ask_layers_and_user_rules() {
    let only_lan = Policy::only(vec![Rule::any_port(
        proto::TCP,
        Target::Net {
            addr: v4(10, 0, 0, 0),
            prefix: 8,
        },
    )]);
    let app = root_app(only_lan.clone(), true);
    let lan = sa(v4(10, 1, 1, 1), 80);
    let web = sa(v4(93, 184, 216, 34), 443);
    let none = UserRules::default();
    assert_eq!(app.check(proto::TCP, lan, NOW, &none, &[]), Verdict::Allow);
    assert_eq!(
        app.check(proto::TCP, web, NOW, &none, &[]),
        Verdict::Ask {
            app: String::from("org.test.app")
        }
    );
    assert!(!app.allows(proto::TCP, web, NOW), "bez rozhodnuti ne");
    // Povoleni jen pro tento pozadavek.
    assert_eq!(app.check(proto::TCP, web, NOW, &none, &[String::from("org.test.app")]), Verdict::Allow);
    // Trvale rozhodnuti: povolit 443, zakaz ma prednost.
    let mut user = UserRules::default();
    user.rules.push((String::from("org.test.app"), Rule::port(proto::TCP, Target::Any, 443), true));
    assert_eq!(app.check(proto::TCP, web, NOW, &user, &[]), Verdict::Allow);
    user.rules.push((
        String::from("org.test.app"),
        Rule::any_port(proto::TCP, Target::Net { addr: v4(93, 0, 0, 0), prefix: 8 }),
        false,
    ));
    assert_eq!(app.check(proto::TCP, web, NOW, &user, &[]), Verdict::Deny);
    // Rozhodnuti jine aplikace neplati.
    let other = UserRules {
        rules: vec![(String::from("org.jina"), Rule::any_port(proto::TCP, Target::Any), true)],
    };
    assert!(matches!(app.check(proto::TCP, web, NOW, &other, &[]), Verdict::Ask { .. }));
    // Vrstva bez dotazu je pevna hranice i s rozhodnutim uzivatele.
    let strict = root_app(only_lan, false);
    assert_eq!(strict.check(proto::TCP, web, NOW, &user, &[]), Verdict::Deny);
    // Dotaz bez identity aplikace nejde.
    assert!(
        Auth::root()
            .restrict_with(Restriction {
                rights: rights::ALL,
                flags: layer_flags::ASK,
                ..Restriction::default()
            })
            .is_err()
    );
}

fn root_app(policy: Policy, ask: bool) -> Auth {
    Auth::root()
        .restrict_with(Restriction {
            rights: rights::CLIENT,
            policy,
            app: String::from("org.test.app"),
            flags: if ask { layer_flags::ASK } else { 0 },
            quota: Quota::UNLIMITED,
        })
        .unwrap()
}

#[test]
fn restriction_wire_roundtrip() {
    let r = Restriction {
        rights: rights::CLIENT,
        policy: Policy::only(vec![Rule::port(proto::TCP, Target::Any, 443)]),
        app: String::from("org.magnumopus.test"),
        flags: layer_flags::ASK,
        quota: Quota {
            sockets: 100,
            bytes: 1 << 20,
            ops: 8,
        },
    };
    let mut w = Writer::new();
    r.put(&mut w);
    let b = w.finish();
    assert_eq!(Restriction::get(&mut Reader::new(&b)).unwrap(), r);
    let mut bad = r.clone();
    bad.app = String::from("zla identita!");
    let mut w = Writer::new();
    bad.put(&mut w);
    let b = w.finish();
    assert_eq!(Restriction::get(&mut Reader::new(&b)).err(), Some(Error::InvalidArgs));
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
        bind_scope: BindScope::Loopback,
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
fn dns_edns_and_truncation() {
    let q = dns::build_query(9, "big.test", dns::TYPE_A).unwrap();
    assert_eq!(dns::query_udp_size(&q), dns::EDNS_UDP, "dotaz nabizi EDNS0");
    let parsed = dns::parse_query(&q).unwrap();
    assert_eq!(parsed.name, "big.test");
    // Dotaz bez EDNS: 512 B.
    let mut plain = q[..q.len() - 11].to_vec();
    plain[11] = 0;
    assert_eq!(dns::query_udp_size(&plain), dns::MAX_UDP);
    // Zkracena odpoved = zeptat se pres TCP.
    let tc = dns::build_truncated(&parsed);
    assert_eq!(dns::parse_response(9, &tc), Err(Error::BufferTooSmall));
    // Velka odpoved (200 adres) se rozebere cela.
    let addrs: Vec<IpAddr> = (0..200u32).map(|i| IpAddr::V4(Ipv4Addr::from(0x0a00_0000 + i))).collect();
    let big = dns::build_response(&parsed, &addrs, 60, dns::RCODE_OK);
    assert!(big.len() > dns::MAX_UDP);
    assert_eq!(dns::parse_response(9, &big).unwrap().addrs.len(), 200);
}

#[test]
fn fw_messages_roundtrip() {
    use crate::fw::{AppStats, Ask, AuditRecord, FwConfig, InRule, audit, inbound};
    let cfg = FwConfig {
        profile: String::from("home"),
        inbound: inbound::LISTENERS,
        ping: true,
        stealth: false,
        allow_in: vec![InRule {
            protos: proto::TCP,
            from: (v4(10, 0, 0, 0), 8),
            ports: (22, 22),
        }],
        blocked: vec![(v4(192, 0, 2, 0), 24)],
        syn_rate: (50, 100),
        icmp_rate: (5, 10),
        dns_block: vec![String::from("*.ads.example")],
        user_rules: vec![(
            String::from("org.test.app"),
            Rule::port(proto::TCP, Target::Host(String::from("api.example.com")), 443),
            true,
        )],
        audit_capacity: 100,
        unauthenticated_fallback: true,
    };
    let mut w = Writer::new();
    cfg.put(&mut w);
    let b = w.finish();
    assert_eq!(FwConfig::get(&mut Reader::new(&b)).unwrap(), cfg);

    let ask = Ask {
        id: 7,
        app: String::from("org.test.app"),
        proto: proto::TCP,
        dest: sa(v4(1, 2, 3, 4), 443),
        host: String::from("api.example.com"),
    };
    let b = ask.encode();
    let mut r = Reader::new(&b);
    assert_eq!(r.u32().unwrap(), crate::fw::op::ASK);
    assert_eq!(Ask::get(&mut r).unwrap(), ask);

    let rec = AuditRecord {
        seq: 5,
        time: 123,
        kind: audit::DENIED,
        app: String::from("org.test.app"),
        proto: proto::UDP,
        remote: Some(sa(v4(8, 8, 8, 8), 53)),
        local_port: 50000,
        detail: String::from("pravidla relace"),
    };
    let mut w = Writer::new();
    rec.put(&mut w);
    let b = w.finish();
    assert_eq!(AuditRecord::get(&mut Reader::new(&b)).unwrap(), rec);
    let st = AppStats {
        app: String::from("a"),
        sent: 1,
        received: 2,
        sockets: 3,
    };
    let mut w = Writer::new();
    st.put(&mut w);
    let b = w.finish();
    assert_eq!(AppStats::get(&mut Reader::new(&b)).unwrap(), st);
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
    // Bez zaznamu OPT (11 B na konci dotazu), odpovedi jdou za otazku.
    let mut d = q[..q.len() - 11].to_vec();
    d[11] = 0;
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
        flags: link::FLAG_UP | link::FLAG_FULL_DUPLEX,
        speed_mbps: 2500,
    };
    assert_eq!(link::Info::decode(&i.encode()).unwrap(), i);
    assert!(link::Info::decode(&[link::FRAME; 10]).is_err());
    // Starsi zprava bez rychlosti (10 B): rychlost neznama.
    let old = link::Info::decode(&i.encode()[..10]).unwrap();
    assert_eq!(old, link::Info { speed_mbps: 0, ..i });
}

#[test]
fn link_status_roundtrip() {
    let s = link::Status {
        flags: link::FLAG_UP,
        speed_mbps: 1000,
    };
    assert_eq!(link::Status::decode(&s.encode()).unwrap(), s);
    // Starsi zprava jen s priznaky.
    let old = link::Status::decode(&[link::LINK, 0]).unwrap();
    assert_eq!(old, link::Status { flags: 0, speed_mbps: 0 });
    assert!(link::Status::decode(&[link::INFO, 1]).is_err());
    assert!(link::Status::decode(&[link::LINK]).is_err());
}

#[test]
fn ntp_filter_and_discipline() {
    use crate::ntp::{Discipline, Sample, best_sample, combine};
    let s = [
        Sample { offset_ns: 40, delay_ns: 900 },
        Sample { offset_ns: 10, delay_ns: 200 },
        Sample { offset_ns: 70, delay_ns: 500 },
    ];
    assert_eq!(best_sample(&s).unwrap().offset_ns, 10, "nejmensi zpozdeni");
    assert_eq!(combine(&mut [5, -100, 7]), Some(5), "median");
    assert_eq!(combine(&mut [1, 3]), Some(2));
    assert_eq!(combine(&mut []), None);

    // Krystal predbiha o 50 ppm: kazdych 64 s se posun zmeni o -3,2 ms.
    const S: u64 = 1_000_000_000;
    let mut d = Discipline::new(16, 1024);
    assert_eq!(d.update(0, 0), None, "prvni mereni = zadny odhad");
    let mut t = 0;
    for _ in 0..12 {
        t += 64 * S;
        // Za 64 s (ppb * s = ns) se rozejdou o zbytek po dosavadni oprave
        // frekvence; predchozi maly posun jadro stihlo dorovnat cely.
        let offset = -(64 * (50_000 + d.freq_ppb as i128));
        d.update(t, offset);
    }
    assert!((d.freq_ppb + 50_000).abs() < 100, "frekvence {} ppb", d.freq_ppb);
    // Srovnane hodiny prodlouzi interval, velky posun ho vrati.
    let mut p = Discipline::new(16, 1024);
    p.update(0, 1_000);
    assert_eq!(p.poll_secs, 32);
    p.update(100 * S, 50_000_000);
    assert_eq!(p.poll_secs, 16);
}

#[test]
fn resolver_messages() {
    use crate::resolver::{Answer, Query, status};
    let q = Query { id: 7, family: 0, name: String::from("www.example.test") };
    assert_eq!(Query::decode(&q.encode()).unwrap(), q);
    let a = Answer { id: 7, status: status::OK, ttl: 300, addrs: vec![v4(10, 0, 2, 50)], secure: true };
    assert_eq!(Answer::decode(&a.encode()).unwrap(), a);
    assert!(Answer::decode(&a.encode()[..5]).is_err());
}

/// Plna sada na aarch64: test `cas` nastavil hodiny o den dopredu a vratil
/// je, zatimco ntpd meril. t1 o 24 h napred proti t4 dava posun -12 h
/// a zpozdeni -24 h; filtr ho vybral jako "nejlepsi" (nejmensi zpozdeni).
#[test]
fn ntp_clock_step_during_measurement_is_discarded() {
    use crate::ntp::{self, MAX_DELAY_NS, Reply, Sample, best_sample, clock_stepped, plausible};
    use abi::time::WallTime;
    let day = 86_400i64;
    let t = |secs: i64, nanos: u32| WallTime { secs, nanos };
    let real_t1 = t(1_800_000_000, 0);
    let t2 = t(1_800_000_000, 1_000_000);
    let t3 = t(1_800_000_000, 2_000_000);
    let t4 = t(1_800_000_000, 3_000_000);
    let reply = Reply { receive: t2, transmit: t3, stratum: 2 };
    // t1 zmereny v posunutem okne (o den dopredu).
    let stepped_t1 = t(1_800_000_000 + day, 0);
    let (offset, delay) = ntp::offset_delay(stepped_t1, &reply, t4);
    assert_eq!(offset, -(day as i128) * 1_000_000_000 / 2, "posun -12 h");
    assert!(delay < -(day as i128) * 1_000_000_000 + 10_000_000, "zpozdeni -24 h");
    // Kontrola skoku: realne hodiny ubehly -24 h, monotonni 3 ms.
    assert!(clock_stepped(stepped_t1, t4, 3_000_000));
    assert!(!clock_stepped(real_t1, t4, 3_000_000), "bez skoku");
    // Plynula korekce (rozdil pod 100 ms) neni skok.
    assert!(!clock_stepped(real_t1, t4, 50_000_000));
    // Verohodnost a vyber.
    let bad = Sample { offset_ns: offset, delay_ns: delay };
    let good = Sample { offset_ns: 500, delay_ns: 3_000_000 };
    assert!(!plausible(&bad));
    assert!(plausible(&good));
    assert!(!plausible(&Sample { offset_ns: 0, delay_ns: MAX_DELAY_NS + 1 }));
    assert_eq!(best_sample(&[bad, good]), Some(good), "zaporne zpozdeni nevyhraje");
    assert_eq!(best_sample(&[bad]), None, "jen neverohodne = zadny vzorek");
}

#[test]
fn bind_scope_loopback_layer_blocks_network() {
    let root = Auth::root();
    assert!(root.allows_bind_net());
    let mut lo = Policy::unrestricted();
    lo.bind_scope = BindScope::Loopback;
    let a = root.restrict(rights::CLIENT | rights::TCP_LISTEN, lo).unwrap();
    assert!(!a.allows_bind_net());
    assert!(a.allows_bind(8080));
    // Dalsi vrstva bez omezeni sit nevrati.
    let b = a.restrict(rights::CLIENT | rights::TCP_LISTEN, Policy::unrestricted()).unwrap();
    assert!(!b.allows_bind_net());
    // Nezname cislo rozsahu se odmitne.
    let mut w = Writer::new();
    Policy::unrestricted().put(&mut w);
    let mut bytes = w.finish();
    *bytes.last_mut().unwrap() = 9;
    assert_eq!(Policy::get(&mut Reader::new(&bytes)).err(), Some(Error::InvalidArgs));
}
