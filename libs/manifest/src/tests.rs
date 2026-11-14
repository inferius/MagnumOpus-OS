//! Testy manifestu (`cargo test -p manifest`).

extern crate std;

use alloc::string::String;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};

use netproto::auth::{Auth, layer_flags};
use netproto::{Rule, Target, proto, rights};

use crate::{Manifest, rule_table, parse_rule, target, target_text};

const FULL: &str = r#"
[app]
id = "org.magnumopus.netprobe"
name = "Sitova sonda"

[limits]
memory = "64M"
kernel_memory = 4194304
stack = "1M"

[network]
rights = ["info", "tcp", "dns"]
ask = true
sockets = 16
buffers = "1M"
ops = 64

[[network.connect]]
proto = ["tcp"]
to = "10.0.2.0/24"
ports = "1000-2000"

[[network.connect]]
to = "*.example.com"
ports = 443

[[network.listen]]
ports = "8080"
"#;

#[test]
fn full_manifest() {
    let m = Manifest::parse(FULL).unwrap();
    assert_eq!(m.id, "org.magnumopus.netprobe");
    assert_eq!(m.name, "Sitova sonda");
    assert_eq!(m.limits.memory, Some(64 << 20));
    assert_eq!(m.limits.kernel_memory, Some(4 << 20));
    assert_eq!(m.limits.stack, Some(1 << 20));
    let n = m.network.unwrap();
    assert_eq!(n.rights, rights::INFO | rights::TCP_CONNECT | rights::RESOLVE);
    assert_eq!(n.flags, layer_flags::ASK);
    assert_eq!(n.app, "org.magnumopus.netprobe");
    assert_eq!(n.quota.sockets, 16);
    assert_eq!(n.quota.bytes, 1 << 20);
    assert_eq!(n.quota.ops, 64);
    let rules = n.policy.connect.as_ref().unwrap();
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].protos, proto::TCP);
    assert_eq!(rules[1].protos, proto::ALL);
    assert_eq!(rules[1].ports, (443, 443));
    assert_eq!(n.policy.bind, Some(alloc::vec![(8080, 8080)]));

    // Relace podle manifestu.
    let a = Auth::root().restrict_with(n).unwrap();
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 2, 2));
    assert!(a.allows(proto::TCP, SocketAddr::new(ip, 1500), 0));
    assert!(!a.allows(proto::TCP, SocketAddr::new(ip, 3000), 0));
    assert!(!a.allows(proto::UDP, SocketAddr::new(ip, 1500), 0));
    assert!(a.allows_bind(8080) && !a.allows_bind(80));
    assert!(!a.has(rights::UDP));
}

#[test]
fn minimal_manifest_has_no_network() {
    let m = Manifest::parse("[app]\nid = \"org.x\"\n").unwrap();
    assert_eq!(m.name, "org.x");
    assert!(m.network.is_none());
    assert_eq!(m.limits.memory, None);
    assert!(!m.jit, "JIT jen kdyz ho manifest vyslovne chce");
    let j = Manifest::parse("[app]\nid = \"org.x\"\njit = true\n").unwrap();
    assert!(j.jit);
    assert_eq!(m.static_tls_reserve, None);
}

#[test]
fn static_tls_reserve() {
    let m = Manifest::parse("[app]\nid = \"org.x\"\nstatic_tls_reserve = \"8K\"\n").unwrap();
    assert_eq!(m.static_tls_reserve, Some(8192));
    let m = Manifest::parse("[app]\nid = \"org.x\"\nstatic_tls_reserve = \"64K\"\n").unwrap();
    assert_eq!(m.static_tls_reserve, Some(65536));
    assert!(Manifest::parse("[app]\nid = \"org.x\"\nstatic_tls_reserve = \"65K\"\n").is_err());
    assert!(Manifest::parse("[app]\nid = \"org.x\"\nstatic_tls_reserve = \"hodne\"\n").is_err());
}

#[test]
fn bad_manifests() {
    let cases = [
        "[app]\nname = \"bez id\"\n",
        "[app]\nid = \"zla identita\"\n",
        "[app]\nid = \"a\"\n[network]\n",
        "[app]\nid = \"a\"\n[network]\nrights = [\"letat\"]\n",
        "[app]\nid = \"a\"\n[network]\nrights = []\n[[network.connect]]\nto = \"bad name\"\n",
        "[app]\nid = \"a\"\n[network]\nrights = []\n[[network.connect]]\nto = \"*\"\nports = \"9-1\"\n",
        "[app]\nid = \"a\"\n[network]\nrights = []\n[[network.connect]]\nto = \"*\"\nproto = \"sctp\"\n",
        "[app]\nid = \"a\"\n[limits]\nmemory = \"hodne\"\n",
        "[app]\nid = \"a\"\n[network]\nrights = []\nsockets = -1\n",
        "[app]\nid = \"a\"\n[network]\nrights = []\nops = 0\n",
        "[app]\nid = \"a\"\n[network]\nrights = []\nops = 5000\n",
    ];
    for c in cases {
        assert!(Manifest::parse(c).is_err(), "{c}");
    }
}

#[test]
fn targets_and_rule_roundtrip() {
    assert_eq!(target("*").unwrap(), Target::Any);
    assert_eq!(
        target("10.0.0.0/8").unwrap(),
        Target::Net {
            addr: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 0)),
            prefix: 8
        }
    );
    assert_eq!(target_text(&target("1.2.3.4").unwrap()), "1.2.3.4");
    assert_eq!(target_text(&target("::1").unwrap()), "::1");
    assert_eq!(target("*.example.com").unwrap(), Target::Host(String::from("*.example.com")));
    assert!(target("ne platny").is_err());
    for rule in [
        Rule::port(proto::TCP, Target::Host(String::from("api.example.com")), 443),
        Rule::any_port(proto::ALL, Target::Any),
        Rule {
            protos: proto::UDP | proto::ICMP,
            target: target("192.168.0.0/16").unwrap(),
            ports: (1000, 2000),
        },
    ] {
        let t = rule_table(&rule);
        let again = conf::parse(&t.to_toml()).unwrap();
        assert_eq!(parse_rule(&again).unwrap(), rule);
    }
}

#[test]
fn commands() {
    let m = Manifest::parse(
        "[app]
id = \"org.x.rg\"
[[command]]
name = \"rg\"
[[command]]
name = \"rg-tool\"
program = \"rgt\"
",
    )
    .unwrap();
    assert_eq!(m.commands.len(), 2);
    assert_eq!(m.commands[0].name, "rg");
    assert_eq!(m.commands[0].program, None);
    assert_eq!(m.commands[1].program.as_deref(), Some("rgt"));
    assert!(Manifest::parse("[app]
id = \"x\"
").unwrap().commands.is_empty());
}

#[test]
fn bad_commands() {
    for bad in [
        "[[command]]
program = \"a\"
",
        "[[command]]
name = \"Velke\"
",
        "[[command]]
name = \"-a\"
",
        "[[command]]
name = \"a b\"
",
        "[[command]]
name = \"a\"
[[command]]
name = \"a\"
",
        "[[command]]
name = \"a\"
program = \"/bin/a\"
",
    ] {
        let text = alloc::format!("[app]
id = \"x\"
{bad}");
        assert!(Manifest::parse(&text).is_err(), "{bad}");
    }
}
