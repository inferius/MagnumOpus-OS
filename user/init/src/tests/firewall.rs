//! Firewall: profil site, aplikace podle manifestu, dotazy uzivateli
//! (init hraje agenta uzivatele), trvala rozhodnuti na FS, blokovane domeny
//! a site, prichozi filtr, audit.
//!
//! Testovaci aplikace `netprobe` ma v manifestu jen UDP na DNS a pomocne
//! servery; TCP a ping rozhoduje uzivatel (`ask = true`).

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};
use core::time::Duration;

use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Process, Startup, time};
use netproto::fw::{audit, choice, op};
use netproto::io::Write;
use netproto::{ReadAddr, Reader, Writer, proto};

use super::net::{HOST, Ports, configured, host, net, ports};
use super::{Test, TestResult, devices, fail, net_service};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("sit poznana, uzivatel ji oznaci jako domaci", classify_network),
    ("aplikace v jobu a s relaci podle manifestu", probe_started),
    ("cil z manifestu bez dotazu", manifest_allows),
    ("dotaz: zamitnout jednou", ask_deny),
    ("dotaz: povolit jednou", ask_allow_once),
    ("dotaz: povolit vzdy (ulozi se na FS)", ask_allow_always),
    ("dotaz: zakazat vzdy", ask_deny_always),
    ("dotaz bez odpovedi vyprsi (vychozi odpoved)", ask_timeout),
    ("connect cekajici na dotaz: relace dal odpovida, zruseni", ask_cancel),
    ("plny limit dotazu relace: odlozeny dotaz jde zrusit hned", ask_backlog_cancel),
    ("blokovane domeny a site", blocklists),
    ("verejna sit nepusti prichozi spojeni", public_inbound),
    ("audit a provoz aplikaci", audit_and_stats),
];

const APP: &str = "org.magnumopus.netprobe";

fn wait(secs: u64) -> Option<u64> {
    Some(time::deadline_after(Duration::from_secs(secs)))
}

/// Agent uzivatele: dotazy od fwd a odpovedi na vlastni pozadavky.
pub(super) struct Agent<'a> {
    ch: &'a Channel,
}

/// Zpravy fwd, ktere prisly behem cekani na odpoved (dotazy).
static STASH: spin::Mutex<VecDeque<Vec<u8>>> = spin::Mutex::new(VecDeque::new());

pub(super) fn agent() -> Result<Agent<'static>, String> {
    let ch = net_service()
        .and_then(|n| n.agent.as_ref())
        .ok_or("firewall nebezi (chybi agent)")?;
    Ok(Agent { ch })
}

struct Question {
    id: u32,
    app: String,
    proto: u8,
    dest: SocketAddr,
}

impl Agent<'_> {
    /// Dalsi zprava od fwd s kodem `code` (ostatni zustanou ve fronte).
    fn next(&self, code: u32, secs: u64) -> Result<Vec<u8>, String> {
        {
            let mut stash = STASH.lock();
            if let Some(i) = stash
                .iter()
                .position(|m| Reader::new(m).u32().ok() == Some(code))
            {
                return Ok(stash.remove(i).unwrap());
            }
        }
        let deadline = wait(secs);
        loop {
            let m = self.ch.read(deadline).map_err(|e| fail("zprava od fwd", e))?;
            if Reader::new(&m.data).u32().ok() == Some(code) {
                return Ok(m.data);
            }
            STASH.lock().push_back(m.data);
        }
    }

    /// Je ve fronte dotaz (bez cekani)?
    fn has_question(&self) -> bool {
        while let Ok(m) = self.ch.try_read() {
            STASH.lock().push_back(m.data);
        }
        STASH
            .lock()
            .iter()
            .any(|m| Reader::new(m).u32().ok() == Some(op::QUESTION))
    }

    fn question(&self) -> Result<Question, String> {
        let m = self.next(op::QUESTION, 10)?;
        let mut r = Reader::new(&m);
        r.u32().map_err(|e| fail("dotaz", e))?;
        let parse = |r: &mut Reader| -> libos::Result<Question> {
            let id = r.u32()?;
            let app = r.str()?;
            let _name = r.str()?;
            let proto = r.u8()?;
            let dest = r.socket_addr()?;
            Ok(Question { id, app, proto, dest })
        };
        parse(&mut r).map_err(|e| fail("dotaz", e))
    }

    fn decide(&self, id: u32, c: u8) -> TestResult {
        let mut w = Writer::op(op::DECIDE);
        w.u32(id).u8(c);
        self.ch.write(&w.finish(), Vec::new()).map_err(|e| fail("rozhodnuti", e))
    }

    /// Pozadavek na fwd; odpoved = zprava se statusem <= 0.
    fn request(&self, w: Writer) -> Result<Vec<u8>, String> {
        self.ch.write(&w.finish(), Vec::new()).map_err(|e| fail("pozadavek fwd", e))?;
        let deadline = wait(10);
        loop {
            let m = self.ch.read(deadline).map_err(|e| fail("odpoved fwd", e))?;
            let first = Reader::new(&m.data).i32().unwrap_or(1);
            if first <= 0 {
                return netproto::fw::parse_agent_reply(&m.data)
                    .map(|mut r| r.rest().to_vec())
                    .map_err(|e| fail("fwd", e));
            }
            STASH.lock().push_back(m.data);
        }
    }

    fn config(&self) -> Result<conf::Table, String> {
        let text = self.request(Writer::op(op::CONFIG_GET))?;
        let text = String::from_utf8(text).map_err(|_| String::from("konfigurace neni UTF-8"))?;
        conf::parse(&text).map_err(|e| format!("konfigurace od fwd: {e}"))
    }

    fn set_config(&self, t: &conf::Table) -> TestResult {
        let mut w = Writer::op(op::CONFIG_SET);
        w.bytes(t.to_toml().as_bytes());
        self.request(w).map(|_| ())
    }

    pub(super) fn profile(&self, p: &str) -> TestResult {
        let mut w = Writer::op(op::PROFILE_SET);
        w.str(p);
        self.request(w).map(|_| ())
    }
}

/// Bezici `netprobe` (spusti ho prvni test, ktery ho potrebuje).
struct Probe {
    _process: Process,
    ch: Channel,
    job: libos::Job,
}

static PROBE: spin::Once<Result<Probe, String>> = spin::Once::new();

fn start_probe(s: &Startup) -> Result<Probe, String> {
    let root = net()?;
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let m = crate::apps::manifest(&fs, "netprobe")
        .map_err(|e| fail("manifest", e))?
        .ok_or("netprobe nema manifest")?;
    let session = crate::apps::net_session(root, &m)
        .map_err(|e| fail("relace podle manifestu", e))?
        .ok_or("manifest nechce sit")?;
    let app = crate::apps::spawn(s, "netprobe", vec![StartHandle::user(session.into_channel().map_err(|e| fail("kanal relace", e))?.into_handle())])
        .map_err(|e| fail("spusteni netprobe", e))?;
    Ok(Probe {
        _process: app.process,
        ch: app.control,
        job: app.job,
    })
}

fn probe(s: &Startup) -> Result<&'static Probe, String> {
    PROBE
        .call_once(|| start_probe(s))
        .as_ref()
        .map_err(|e| e.clone())
}

impl Probe {
    fn send(&self, cmd: &str) -> TestResult {
        self.ch.write(cmd.as_bytes(), Vec::new()).map_err(|e| fail("prikaz netprobe", e))
    }

    fn reply(&self) -> Result<String, String> {
        let m = self.ch.read(wait(30)).map_err(|e| fail("odpoved netprobe", e))?;
        String::from_utf8(m.data).map_err(|_| String::from("odpoved neni UTF-8"))
    }

    fn run(&self, cmd: &str) -> Result<String, String> {
        self.send(cmd)?;
        self.reply()
    }
}

fn need_ports(s: &Startup, test: &str) -> Result<Option<Ports>, String> {
    let p = ports(s, test)?;
    if p.is_some() {
        configured(net()?)?;
    }
    Ok(p)
}

/// Limit na prepnuti profilu: fwd konfiguraci nejdriv ulozi na FS a pak
/// posle netd; pod TCG a zatezi (vic QEMU naraz) to trva i sekundy.
const PROFILE_TIMEOUT: Duration = Duration::from_secs(10);

/// Poradove cislo, od ktereho prijdou nove zaznamy auditu.
pub(super) fn audit_end(root: &netproto::Net) -> Result<u64, String> {
    let mut from = 0;
    loop {
        let (recs, next) = root.audit(from, 256).map_err(|e| fail("AUDIT", e))?;
        if recs.is_empty() {
            return Ok(from.max(next));
        }
        from = next;
    }
}

/// Pocka na udalost prepnuti profilu na `want` v netd (zaznam auditu
/// PROFILE od `from`) s limitem [`PROFILE_TIMEOUT`]. Kdyz uz profil `want`
/// byl, zaznam nevznikne - staci aktualni profil.
pub(super) fn wait_profile(root: &netproto::Net, mut from: u64, want: &str) -> TestResult {
    let deadline = time::deadline_after(PROFILE_TIMEOUT);
    loop {
        let (recs, next) = root.audit(from, 256).map_err(|e| fail("AUDIT", e))?;
        if recs.iter().any(|r| r.kind == audit::PROFILE && r.detail == want) {
            return Ok(());
        }
        from = next;
        let current = root.stats().map_err(|e| fail("STATS", e))?.0;
        if current == want {
            return Ok(());
        }
        if time::now() > deadline {
            return Err(format!(
                "profil se neprepnul na {want} do {} s (je {current})",
                PROFILE_TIMEOUT.as_secs()
            ));
        }
        time::sleep(Duration::from_millis(20));
    }
}

fn classify_network(_: &Startup) -> TestResult {
    classify()
}

fn classify() -> TestResult {
    let a = agent()?;
    let root = net()?;
    configured(root)?;
    // Provoz na branu: netd zjisti jeji MAC (ARP) a ohlasi sit fwd.
    let _ = root.ping(HOST, 1, 8, Duration::from_secs(2));
    let m = a.next(op::NETWORK_QUESTION, 15)?;
    let mut r = Reader::new(&m);
    r.u32().map_err(|e| fail("dotaz na sit", e))?;
    let id = r.u32().map_err(|e| fail("dotaz na sit", e))?;
    let network = r.str().map_err(|e| fail("dotaz na sit", e))?;
    let gateway = r.str().map_err(|e| fail("dotaz na sit", e))?;
    check_eq!(gateway.as_str(), "10.0.2.2", "brana nezname site");
    check_eq!(network.len(), 17, "id site = MAC brany");
    let (profile, _) = root.stats().map_err(|e| fail("STATS", e))?;
    check_eq!(profile.as_str(), "public", "neznama sit = verejna");
    let from = audit_end(root)?;
    let mut w = Writer::op(op::CLASSIFY);
    w.u32(id).str("home");
    a.ch.write(&w.finish(), Vec::new()).map_err(|e| fail("CLASSIFY", e))?;
    // Profil se prepne (fwd posle netd novou konfiguraci).
    wait_profile(root, from, "home")?;
    let cfg = a.config()?;
    check!(
        cfg.tables("networks")
            .iter()
            .any(|n| n.get_str("id") == Some(network.as_str()) && n.get_str("profile") == Some("home")),
        "sit se nezapamatovala"
    );
    Ok(())
}

/// Sit v profilu `home` (prichozi spojeni na naslouchajici porty). Pro
/// testy site spustene bez sady firewallu (`--only sit`): odpovi na dotaz
/// agenta na neznamou sit, kdyz jeste nebyl.
pub(super) fn ensure_home() -> TestResult {
    let root = net()?;
    if root.stats().map_err(|e| fail("STATS", e))?.0 == "home" {
        return Ok(());
    }
    classify()
}

fn probe_started(s: &Startup) -> TestResult {
    let p = probe(s)?;
    let info = p.job.info().map_err(|e| fail("JOB_INFO", e))?;
    check_eq!(info.user.limit, 16 << 20, "limit pameti z manifestu");
    check_eq!(info.kernel.limit, 4 << 20, "limit pameti jadra z manifestu");
    check!(info.user.used > 0, "program bezi na ucet sveho jobu");
    Ok(())
}

fn manifest_allows(s: &Startup) -> TestResult {
    let Some(ports) = need_ports(s, "firewall")? else {
        return Ok(());
    };
    let p = probe(s)?;
    let a = agent()?;
    let r = p.run(&format!("resolve@{} test.magnumopus", host(ports.dns)))?;
    check_eq!(r.as_str(), "ok:1", "DNS na povoleny server");
    check!(!a.has_question(), "cil z manifestu se nema ptat");
    Ok(())
}

/// Prikaz aplikace, ktery vyvola dotaz: overi dotaz a odpovi `c`.
fn asked(p: &Probe, a: &Agent, cmd: &str, dest: SocketAddr, proto_bit: u8, c: u8) -> Result<String, String> {
    p.send(cmd)?;
    let q = a.question()?;
    check_eq!(q.app.as_str(), APP, "aplikace v dotazu");
    check_eq!(q.dest, dest, "cil v dotazu");
    check_eq!(q.proto, proto_bit, "protokol v dotazu");
    a.decide(q.id, c)?;
    p.reply()
}

fn ask_deny(s: &Startup) -> TestResult {
    let Some(ports) = need_ports(s, "firewall")? else {
        return Ok(());
    };
    let (p, a) = (probe(s)?, agent()?);
    let dest = host(ports.echo);
    let r = asked(p, &a, &format!("connect {dest}"), dest, proto::TCP, choice::DENY_ONCE)?;
    check_eq!(r.as_str(), "err:ACCESS_DENIED", "zamitnuto uzivatelem");
    Ok(())
}

fn ask_allow_once(s: &Startup) -> TestResult {
    let Some(ports) = need_ports(s, "firewall")? else {
        return Ok(());
    };
    let (p, a) = (probe(s)?, agent()?);
    let dest = host(ports.echo);
    let cmd = format!("connect {dest}");
    let r = asked(p, &a, &cmd, dest, proto::TCP, choice::ALLOW_ONCE)?;
    check_eq!(r.as_str(), "ok", "povoleno jednou");
    // Jednou = jen ten pozadavek: dalsi se zepta znovu.
    let r = asked(p, &a, &cmd, dest, proto::TCP, choice::DENY_ONCE)?;
    check_eq!(r.as_str(), "err:ACCESS_DENIED", "dalsi pozadavek");
    Ok(())
}

fn ask_allow_always(s: &Startup) -> TestResult {
    let Some(ports) = need_ports(s, "firewall")? else {
        return Ok(());
    };
    let (p, a) = (probe(s)?, agent()?);
    let dest = host(ports.echo);
    let cmd = format!("connect {dest}");
    let r = asked(p, &a, &cmd, dest, proto::TCP, choice::ALLOW_ALWAYS)?;
    check_eq!(r.as_str(), "ok", "povoleno vzdy");
    // Dalsi spojeni bez dotazu.
    let r = p.run(&cmd)?;
    check_eq!(r.as_str(), "ok", "spojeni po trvalem povoleni");
    check!(!a.has_question(), "po trvalem povoleni se nema ptat");
    // Rozhodnuti je v konfiguraci i v souboru na FS.
    let cfg = a.config()?;
    let rule = cfg
        .tables("apps")
        .into_iter()
        .find(|t| t.get_str("id") == Some(APP))
        .ok_or("aplikace neni v konfiguraci")?
        .tables("rule")
        .into_iter()
        .find(|r| r.get_str("to") == Some("10.0.2.2"))
        .cloned()
        .ok_or("pravidlo neni v konfiguraci")?;
    check_eq!(rule.get_str("action"), Some("allow"), "akce pravidla");
    check_eq!(rule.get_str("ports").map(String::from), Some(format!("{}", ports.echo)), "port pravidla");
    if let Some(fs) = devices().and_then(|d| d.fs.as_ref()) {
        let path = format!("{}/firewall.toml", crate::system::FIREWALL_DIR);
        let data = fs.read(&path, 0, 1 << 20).map_err(|e| fail("cteni konfigurace z FS", e))?;
        let text = String::from_utf8(data).map_err(|_| String::from("soubor neni UTF-8"))?;
        check!(text.contains(APP), "rozhodnuti neni ulozene na FS");
    }
    Ok(())
}

fn ask_deny_always(s: &Startup) -> TestResult {
    let Some(_) = need_ports(s, "firewall")? else {
        return Ok(());
    };
    let (p, a) = (probe(s)?, agent()?);
    let dest = SocketAddr::new(HOST, 0);
    let cmd = format!("ping {HOST}");
    let r = asked(p, &a, &cmd, dest, proto::ICMP, choice::DENY_ALWAYS)?;
    check_eq!(r.as_str(), "err:ACCESS_DENIED", "zakazano vzdy");
    let r = p.run(&cmd)?;
    check_eq!(r.as_str(), "err:ACCESS_DENIED", "dalsi ping bez dotazu");
    check!(!a.has_question(), "po trvalem zakazu se nema ptat");
    Ok(())
}

fn ask_timeout(s: &Startup) -> TestResult {
    let Some(ports) = need_ports(s, "firewall")? else {
        return Ok(());
    };
    let (p, a) = (probe(s)?, agent()?);
    let mut cfg = a.config()?;
    cfg.table_mut("firewall").set("ask_timeout", conf::Value::Int(1));
    a.set_config(&cfg)?;
    let dest = host(ports.closed);
    let start = time::now();
    p.send(&format!("connect {dest}"))?;
    let q = a.question()?;
    let r = p.reply()?;
    check_eq!(r.as_str(), "err:ACCESS_DENIED", "bez odpovedi = ask_default (deny)");
    check!(time::now() - start >= 900_000_000, "odpoved az po vyprseni");
    // Pozdni odpoved uz nic nezmeni.
    a.decide(q.id, choice::ALLOW_ONCE)?;
    cfg.table_mut("firewall").set("ask_timeout", conf::Value::Int(60));
    a.set_config(&cfg)
}

fn blocklists(s: &Startup) -> TestResult {
    let Some(ports) = need_ports(s, "firewall")? else {
        return Ok(());
    };
    let a = agent()?;
    let root = net()?;
    let original = a.config()?;
    let mut cfg = original.clone();
    let block = cfg.table_mut("block");
    block.set("domains", conf::Value::Array(vec![conf::Value::Str(String::from("blocked.magnumopus"))]));
    block.set("networks", conf::Value::Array(vec![conf::Value::Str(String::from("10.0.2.77/32"))]));
    a.set_config(&cfg)?;
    let server = Some(host(ports.dns));
    let t = Some(super::net::DNS_TIMEOUT);
    // Blokovane jmeno i jeho poddomena, bez dotazu na server.
    check_eq!(
        root.resolve_with("blocked.magnumopus", 4, server, t).err(),
        Some(Error::AccessDenied),
        "blokovana domena"
    );
    check_eq!(
        root.resolve_with("x.blocked.magnumopus", 4, server, t).err(),
        Some(Error::AccessDenied),
        "poddomena blokovane domeny"
    );
    check!(root.resolve_with("test.magnumopus", 4, server, t).is_ok(), "jina domena");
    // Blokovana sit plati i pro korenovou relaci.
    check_eq!(
        root.tcp_connect(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 2, 77)), 80), t).err(),
        Some(Error::AccessDenied),
        "blokovana sit"
    );
    a.set_config(&original)?;
    check!(root.resolve_with("x.blocked.magnumopus", 4, server, t).err() != Some(Error::AccessDenied), "po odblokovani");
    Ok(())
}

fn public_inbound(s: &Startup) -> TestResult {
    let Some(ports) = need_ports(s, "firewall")? else {
        return Ok(());
    };
    let a = agent()?;
    let root = net()?;
    let from = audit_end(root)?;
    a.profile("public")?;
    wait_profile(root, from, "public")?;
    let result = (|| -> TestResult {
        let listener = root.tcp_listen(ports.forwarded, 2).map_err(|e| fail("naslouchani", e))?;
        let mut ctl = root
            .tcp_connect(host(ports.callback), Some(Duration::from_secs(10)))
            .map_err(|e| fail("spojeni na callback", e))?;
        ctl.write_all(b"connect\n").map_err(|e| fail("zapis callback", e))?;
        check_eq!(
            listener.accept_timeout(Some(Duration::from_secs(2))).err(),
            Some(Error::TimedOut),
            "prichozi spojeni ve verejne siti"
        );
        Ok(())
    })();
    let from = audit_end(root)?;
    a.profile("home")?;
    wait_profile(root, from, "home").map_err(|e| format!("profil vracen: {e}"))?;
    result?;
    // Domaci sit spojeni pusti (overuje test naslouchani v sekci sit).
    Ok(())
}

fn audit_and_stats(s: &Startup) -> TestResult {
    let Some(_) = need_ports(s, "firewall")? else {
        return Ok(());
    };
    let root = net()?;
    let mut all = Vec::new();
    let mut from = 0;
    loop {
        let (recs, next) = root.audit(from, 256).map_err(|e| fail("AUDIT", e))?;
        if recs.is_empty() {
            break;
        }
        all.extend(recs);
        from = next;
    }
    let has = |kind: u8| all.iter().any(|r| r.kind == kind && (r.app == APP || kind >= audit::INBOUND_DROP));
    check!(has(audit::ASK_ALLOWED), "audit: povoleno uzivatelem");
    check!(has(audit::ASK_DENIED), "audit: zamitnuto uzivatelem");
    check!(has(audit::DENIED), "audit: trvaly zakaz");
    check!(has(audit::DNS_BLOCKED), "audit: blokovana domena");
    check!(has(audit::BLOCKED_NET), "audit: blokovana sit");
    check!(has(audit::INBOUND_DROP), "audit: zahozene prichozi spojeni");
    check!(has(audit::PROFILE), "audit: zmena profilu");
    let seqs: Vec<u64> = all.iter().map(|r| r.seq).collect();
    check!(seqs.windows(2).all(|w| w[0] < w[1]), "poradova cisla rostou");
    let (_, stats) = root.stats().map_err(|e| fail("STATS", e))?;
    let probe = stats.iter().find(|s| s.app == APP).ok_or("netprobe chybi ve statistikach")?;
    check!(probe.sockets >= 2, "spojeni aplikace");
    // Relace aplikace audit cist nesmi.
    check_eq!(
        crate::apps::net_session(root, &manifest::Manifest::parse("[app]\nid = \"x\"\n[network]\nrights = [\"info\"]\n").unwrap())
            .map_err(|e| fail("relace", e))?
            .ok_or("relace")?
            .audit(0, 1)
            .err(),
        Some(Error::AccessDenied),
        "audit bez prava"
    );
    Ok(())
}

/// netproto v2: connect, ktery ceka na rozhodnuti uzivatele, neblokuje
/// dalsi dotazy na stejne relaci a jde zrusit (`CANCEL`) - driv cekal az
/// do odpovedi nebo `ask_timeout`.
fn ask_cancel(s: &Startup) -> TestResult {
    use netproto::{Buffers, CancelToken, Deadline, Restriction, layer_flags, rights};
    let Some(ports) = need_ports(s, "firewall")? else {
        return Ok(());
    };
    let a = agent()?;
    let root = net()?;
    configured(root)?;
    const ME: &str = "org.magnumopus.nettest";
    let app = root
        .restrict_with(&Restriction {
            rights: rights::CLIENT,
            policy: netproto::Policy::only(Vec::new()),
            app: String::from(ME),
            flags: layer_flags::ASK,
            ..Restriction::default()
        })
        .map_err(|e| fail("relace s dotazem", e))?;
    let token = CancelToken::new();
    let dest = host(ports.echo);
    let (session, tk) = (app.clone(), token.clone());
    let out = alloc::sync::Arc::new(libos::sync::Mutex::new(None));
    let o = out.clone();
    let t = libos::thread::spawn("fw-cancel", move || {
        let r = session.tcp_connect_ctx(dest, Deadline::after(Some(Duration::from_secs(60))), Buffers::DEFAULT, Some(&tk));
        *o.lock() = Some(r.map(|_| ()));
    })
    .map_err(|e| fail("vlakno", e))?;
    let q = a.question()?;
    check_eq!(q.app.as_str(), ME, "aplikace v dotazu");
    check_eq!(q.dest, dest, "cil v dotazu");
    // Relace odpovida, i kdyz na ni ceka connect.
    let start = time::now();
    app.info().map_err(|e| fail("INFO behem connectu", e))?;
    let ms = (time::now() - start) / 1_000_000;
    check!(ms < 2_000, format!("INFO za connectem trvalo {ms} ms"));
    let start = time::now();
    token.cancel();
    t.join(wait(20)).map_err(|e| fail("vlakno connectu nedobehlo", e))?;
    let ms = (time::now() - start) / 1_000_000;
    let r = out.lock().take().ok_or("vlakno nic nevratilo")?;
    check_eq!(r.err(), Some(Error::Canceled), "zruseny connect");
    check!(ms < 2_000, format!("zruseni trvalo {ms} ms"));
    // Pozdni odpoved uzivatele uz nic nezmeni (netd dotaz zapomnel).
    a.decide(q.id, choice::DENY_ONCE)?;
    app.info().map_err(|e| fail("INFO po zruseni", e))?;
    Ok(())
}

/// Relace s limitem 1 dotazu: connect ceka na uzivatele, dalsi dotaz se
/// v netd odlozi (protitlak, ne chyba) a jeho zruseni prijde hned, i kdyz
/// prvni dotaz dal ceka.
fn ask_backlog_cancel(s: &Startup) -> TestResult {
    use alloc::sync::Arc;
    use libos::sync::Mutex;
    use netproto::{Buffers, CancelToken, Deadline, Quota, Restriction, layer_flags, rights};
    let Some(ports) = need_ports(s, "firewall")? else {
        return Ok(());
    };
    let a = agent()?;
    let root = net()?;
    configured(root)?;
    const ME: &str = "org.magnumopus.nettest";
    let app = root
        .restrict_with(&Restriction {
            rights: rights::CLIENT,
            policy: netproto::Policy::only(Vec::new()),
            app: String::from(ME),
            flags: layer_flags::ASK,
            quota: Quota { ops: 1, ..Quota::UNLIMITED },
        })
        .map_err(|e| fail("relace s limitem 1", e))?;
    let dest = host(ports.echo);
    let connect = |token: CancelToken| {
        let session = app.clone();
        let out: Arc<Mutex<Option<libos::Result<()>>>> = Arc::new(Mutex::new(None));
        let o = out.clone();
        let t = libos::thread::spawn("fw-limit", move || {
            let r = session.tcp_connect_ctx(dest, Deadline::after(Some(Duration::from_secs(60))), Buffers::DEFAULT, Some(&token));
            *o.lock() = Some(r.map(|_| ()));
        });
        t.map(|t| (t, out)).map_err(|e| fail("vlakno", e))
    };
    let first = CancelToken::new();
    let (t1, o1) = connect(first.clone())?;
    let q = a.question()?;
    check_eq!(q.app.as_str(), ME, "aplikace v dotazu");
    // Druhy dotaz: limit relace je plny, netd ho odlozi.
    let second = CancelToken::new();
    let (t2, o2) = connect(second.clone())?;
    time::sleep(Duration::from_millis(300));
    check!(o2.lock().is_none(), "odlozeny dotaz skoncil sam (mel cekat)");
    let start = time::now();
    second.cancel();
    t2.join(wait(10)).map_err(|e| fail("vlakno odlozeneho dotazu", e))?;
    let ms = (time::now() - start) / 1_000_000;
    let r2 = o2.lock().take().ok_or("vlakno nic nevratilo")?;
    check_eq!(r2.err(), Some(Error::Canceled), "zruseny odlozeny dotaz");
    check!(ms < 2_000, format!("zruseni odlozeneho dotazu trvalo {ms} ms"));
    check!(o1.lock().is_none(), "prvni dotaz mel dal cekat na uzivatele");
    first.cancel();
    t1.join(wait(20)).map_err(|e| fail("vlakno prvniho dotazu", e))?;
    check_eq!(o1.lock().take().ok_or("prvni nic nevratil")?.err(), Some(Error::Canceled), "zruseny prvni dotaz");
    a.decide(q.id, choice::DENY_ONCE)?;
    // Misto se uvolnilo: relace odpovida.
    app.info().map_err(|e| fail("INFO po uvolneni limitu", e))?;
    Ok(())
}
