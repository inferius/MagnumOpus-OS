//! Testy filtru (`cargo test -p netfilter`).

extern crate std;

use alloc::vec;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr};

use crate::{Bucket, Config, Decision, Filter, InRule, Inbound, Ports, Proto, Rate, Reason, parse_frame};

const MS: u64 = 1_000_000;
const S: u64 = 1_000_000_000;

struct Fake {
    tcp: Vec<u16>,
    udp: Vec<u16>,
}

impl Ports for Fake {
    fn tcp_listening(&self, port: u16) -> bool {
        self.tcp.contains(&port)
    }
    fn udp_bound(&self, port: u16) -> bool {
        self.udp.contains(&port)
    }
}

fn ports() -> Fake {
    Fake {
        tcp: vec![8080],
        udp: vec![5353],
    }
}

/// Ramec Ethernet + IPv4 + zacatek TCP/UDP/ICMP.
fn frame(src: [u8; 4], proto: u8, sport: u16, dport: u16, extra: &[u8]) -> Vec<u8> {
    let mut f = vec![0u8; 12];
    f.extend_from_slice(&0x0800u16.to_be_bytes());
    let mut ip = vec![0x45, 0, 0, 0, 0, 0, 0, 0, 64, proto, 0, 0];
    ip.extend_from_slice(&src);
    ip.extend_from_slice(&[10, 0, 2, 15]);
    f.extend_from_slice(&ip);
    if proto == 1 {
        f.extend_from_slice(extra);
    } else {
        f.extend_from_slice(&sport.to_be_bytes());
        f.extend_from_slice(&dport.to_be_bytes());
        f.extend_from_slice(extra);
    }
    f
}

fn syn(src: [u8; 4], dport: u16) -> Vec<u8> {
    // seq, ack (8 B), offset, flags = SYN
    frame(src, 6, 40000, dport, &[0, 0, 0, 0, 0, 0, 0, 0, 0x50, 0x02])
}

fn ack(src: [u8; 4], dport: u16) -> Vec<u8> {
    frame(src, 6, 40000, dport, &[0, 0, 0, 0, 0, 0, 0, 0, 0x50, 0x10])
}

fn udp(src: [u8; 4], sport: u16, dport: u16) -> Vec<u8> {
    frame(src, 17, sport, dport, &[0, 8, 0, 0])
}

fn ping(src: [u8; 4]) -> Vec<u8> {
    frame(src, 1, 0, 0, &[8, 0, 0, 0])
}

fn home() -> Config {
    Config {
        inbound: Inbound::Listeners,
        ping: true,
        ..Config::public()
    }
}

fn decide(f: &mut Filter, fr: &[u8], now: u64) -> Decision {
    f.inbound(&parse_frame(fr), now, &ports())
}

const PASS: Decision = Decision::Pass;
fn dropped(reason: Reason) -> Decision {
    Decision::Drop { reason }
}

#[test]
fn parse_frames() {
    let p = parse_frame(&syn([1, 2, 3, 4], 80));
    assert_eq!(p.src, Some(Ipv4Addr::new(1, 2, 3, 4)));
    assert_eq!(p.dport, 80);
    assert_eq!(p.proto, Proto::Tcp { syn: true, ack: false, rst: false });
    assert_eq!(parse_frame(&ping([1, 1, 1, 1])).proto, Proto::IcmpEcho);
    assert_eq!(parse_frame(&frame([1, 1, 1, 1], 1, 0, 0, &[0, 0])).proto, Proto::Icmp);
    // ARP a poskozene ramce filtr neresi.
    let mut arp = vec![0u8; 12];
    arp.extend_from_slice(&0x0806u16.to_be_bytes());
    assert_eq!(parse_frame(&arp).src, None);
    assert_eq!(parse_frame(&[0u8; 5]).src, None);
    // Dalsi fragment: bez portu.
    let mut frag = udp([1, 2, 3, 4], 1, 2);
    frag[14 + 6] = 0x00;
    frag[14 + 7] = 0x10;
    assert_eq!(parse_frame(&frag).proto, Proto::Fragment);
}

#[test]
fn public_profile_is_closed() {
    let mut f = Filter::new(Config::public());
    assert_eq!(decide(&mut f, &syn([8, 8, 8, 8], 8080), 0), dropped(Reason::Inbound), "naslouchani");
    assert_eq!(decide(&mut f, &ack([8, 8, 8, 8], 8080), 0), PASS, "existujici spojeni");
    assert_eq!(decide(&mut f, &udp([8, 8, 8, 8], 9, 5353), 0), dropped(Reason::Inbound));
    assert_eq!(decide(&mut f, &ping([8, 8, 8, 8]), 0), dropped(Reason::Ping));
    assert_eq!(decide(&mut f, &udp([10, 0, 2, 2], 67, 68), 0), PASS, "DHCP");
    assert_eq!(f.dropped[Reason::Inbound as usize], 2);
}

#[test]
fn home_profile_allows_listeners() {
    let mut f = Filter::new(home());
    assert_eq!(decide(&mut f, &syn([8, 8, 8, 8], 8080), 0), PASS);
    assert_eq!(decide(&mut f, &syn([8, 8, 8, 8], 22), 0), dropped(Reason::Inbound), "nikdo nenaslouchá");
    assert_eq!(decide(&mut f, &udp([8, 8, 8, 8], 9, 5353), 0), PASS);
    assert_eq!(decide(&mut f, &ping([8, 8, 8, 8]), 0), PASS);
}

#[test]
fn allow_in_rules_and_blocked_networks() {
    let mut cfg = Config::public();
    cfg.allow_in.push(InRule {
        protos: Proto::TCP_BIT,
        from: (IpAddr::V4(Ipv4Addr::new(10, 0, 0, 0)), 8),
        ports: (22, 22),
    });
    cfg.blocked.push((IpAddr::V4(Ipv4Addr::new(10, 6, 0, 0)), 16));
    let mut f = Filter::new(cfg);
    assert_eq!(decide(&mut f, &syn([10, 1, 1, 1], 22), 0), PASS);
    assert_eq!(decide(&mut f, &syn([11, 1, 1, 1], 22), 0), dropped(Reason::Inbound), "mimo sit");
    assert_eq!(decide(&mut f, &ack([10, 6, 1, 1], 22), 0), dropped(Reason::Blocked), "blokovana sit");
    assert!(f.blocked(IpAddr::V4(Ipv4Addr::new(10, 6, 9, 9))));
    assert!(!f.blocked(IpAddr::V4(Ipv4Addr::new(10, 7, 0, 1))));
}

#[test]
fn udp_flows_allow_replies() {
    let mut f = Filter::new(Config::public());
    let dns = Ipv4Addr::new(10, 0, 2, 3);
    f.note_udp_out(50000, dns, 53, 0);
    assert_eq!(decide(&mut f, &udp([10, 0, 2, 3], 53, 50000), S), PASS, "odpoved");
    assert_eq!(decide(&mut f, &udp([10, 0, 2, 4], 53, 50000), S), dropped(Reason::Inbound), "jiny server");
    assert_eq!(decide(&mut f, &udp([10, 0, 2, 3], 54, 50000), S), dropped(Reason::Inbound), "jiny port");
    assert_eq!(
        decide(&mut f, &udp([10, 0, 2, 3], 53, 50000), crate::UDP_FLOW_NS + S),
        dropped(Reason::Inbound),
        "tok vyprsel"
    );
    f.note_udp_out(50001, dns, 53, 0);
    f.forget_udp_port(50001);
    assert_eq!(decide(&mut f, &udp([10, 0, 2, 3], 53, 50001), 0), dropped(Reason::Inbound));
}

#[test]
fn stealth_off_passes_to_stack_but_counts() {
    let mut cfg = Config::public();
    cfg.stealth = false;
    let mut f = Filter::new(cfg);
    assert_eq!(decide(&mut f, &syn([8, 8, 8, 8], 22), 0), PASS, "zavreny port: RST od stacku");
    assert_eq!(
        decide(&mut f, &syn([8, 8, 8, 8], 8080), 0),
        dropped(Reason::Inbound),
        "posluchac ve verejne siti spojeni nedostane"
    );
    assert_eq!(f.dropped[Reason::Inbound as usize], 2);
}

#[test]
fn syn_rate_limit_per_source() {
    let mut cfg = home();
    cfg.syn = Rate { per_sec: 10, burst: 5 };
    let mut f = Filter::new(cfg);
    let mut passed = 0;
    for _ in 0..20 {
        if decide(&mut f, &syn([6, 6, 6, 6], 8080), 0) == PASS {
            passed += 1;
        }
    }
    assert_eq!(passed, 5, "davka");
    // Jiny zdroj ma vlastni kos.
    assert_eq!(decide(&mut f, &syn([7, 7, 7, 7], 8080), 0), PASS);
    // Po 100 ms pribude jeden token.
    assert_eq!(decide(&mut f, &syn([6, 6, 6, 6], 8080), 100 * MS), PASS);
    assert_eq!(decide(&mut f, &syn([6, 6, 6, 6], 8080), 100 * MS), dropped(Reason::RateSyn));
}

#[test]
fn bucket_refills_smoothly() {
    let mut b = Bucket::new(Rate { per_sec: 1, burst: 1 }, 0);
    assert!(b.take(0));
    assert!(!b.take(500 * MS));
    assert!(b.take(1000 * MS));
    assert!(!b.take(1000 * MS));
}
