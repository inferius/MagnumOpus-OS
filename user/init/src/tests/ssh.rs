//! Vzdaleny pristup SSH (`docs/cs/ssh.md`): klient OpenSSH hostitele se
//! pripoji pres presmerovany port (`xtask/src/sshtest.rs`).
//!
//! Test prida uctu klic (s heslem), povoli mu vzdaleny pristup, zapne
//! sshd (bez dialogu spravy: ukon spravy ma vlastni test) a pozada
//! pomocny server hostitele o kroky klienta: prikaz s vystupem a kodem,
//! chyba prikazu, terminal s PTY, cizi klic a samotne heslo odmitnute,
//! relace v domove uctu. Pak overi konec vzdalenych relaci a po vypnuti,
//! ze server spojeni neprijme.

use alloc::format;
use alloc::string::String;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};
use core::time::Duration;

use accountproto::client::{Login, Setup};
use accountproto::{Request, Secret, ids};
use libos::{Startup, println, time};
use netproto::io::Write;

use super::{Test, TestResult, fail, net_service};
use crate::check;

pub const TESTS: &[Test] = &[
    ("ssh: prihlaseni klicem z OpenSSH, prikaz, terminal, odmitnuti, vypnuti", remote),
    ("ssh: klient v systemu (known_hosts, vyzva Ucty, scp, sftp, zmeneny klic)", super::ssh_client::client),
];

/// Ucty, ktere zakladaji jine sady (jmeno, heslo); jinak vlastni.
const KNOWN: &[(&str, &str)] = &[("alice", "Kocka leze dirou"), ("sdltest", "Zvuk jen s povolenim 42")];
const OWN: (&str, &str) = ("sshtest", "Vzdaleny pristup 7");

fn config() -> Option<(u16, String)> {
    let fs = crate::install::STARTUP_INITFS.get()?;
    let text = core::str::from_utf8(fs.find("ssh-test")?).ok()?;
    let port = text.lines().find_map(|l| l.strip_prefix("helper="))?.trim().parse().ok()?;
    let key = text.lines().find_map(|l| l.strip_prefix("key="))?.trim().into();
    Some((port, key))
}

/// Ucet testu: (uid, jmeno, heslo).
pub(super) fn user() -> Result<(u64, &'static str, &'static str), String> {
    let login = Login(crate::accounts::channel("login").map_err(|e| fail("kanal login", e))?);
    let list = login.list().map_err(|e| fail("vypis uctu", e))?;
    for (name, pw) in KNOWN.iter().chain([&OWN]) {
        if let Some(a) = list.iter().find(|a| a.name == *name && a.uid >= ids::FIRST_USER) {
            return Ok((a.uid, name, pw));
        }
    }
    if list.iter().any(|a| a.uid >= ids::FIRST_USER) {
        return Err(String::from("v systemu je ucet s neznamym heslem (pustte --only ssh nebo s ucty)"));
    }
    let setup = Setup(crate::accounts::channel("setup").map_err(|e| fail("kanal setup", e))?);
    let uid = setup.create_first(OWN.0, "SSH test", Secret::from(OWN.1)).map_err(|e| fail("zalozeni uctu", e))?;
    Ok((uid, OWN.0, OWN.1))
}

/// Krok pomocneho serveru hostitele (radek tam, `ok` / `err ...` zpet).
/// Behem cekani obsluhuje terminaly (v testovacim behu nebezi supervisor,
/// ktery jinak spousti termd vzdalenym relacim).
fn helper(startup: &Startup, port: u16, line: &str) -> TestResult {
    let net = &net_service().ok_or("sit nebezi")?.root;
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 2, 2)), port);
    let mut s = net.tcp_connect(addr, Some(Duration::from_secs(10))).map_err(|e| fail("pomocny server ssh", e))?;
    s.set_read_timeout(Some(Duration::from_millis(100)));
    let end = time::deadline_after(Duration::from_secs(300));
    s.write_all(format!("{line}\n").as_bytes()).map_err(|e| fail("zapis", e))?;
    let mut reply = alloc::vec::Vec::new();
    let mut b = [0u8; 256];
    while !reply.contains(&b'\n') {
        match s.read(&mut b) {
            Ok(0) => break,
            Ok(n) => reply.extend_from_slice(&b[..n]),
            Err(libos::Error::TimedOut) | Err(libos::Error::ShouldWait) if time::now() < end => {
                crate::terminal::TERMINALS.lock().poll(startup);
            }
            Err(e) => return Err(fail("odpoved pomocneho serveru", e)),
        }
    }
    let reply = String::from(String::from_utf8_lossy(&reply).trim());
    if reply == "ok" { Ok(()) } else { Err(format!("hostitel ({line}): {reply}")) }
}

fn remote(startup: &Startup) -> TestResult {
    let Some((port, key)) = config() else {
        return Err(String::from("chybi ssh-test v initfs (xtask nenasel klienta OpenSSH, viz vypis xtask)"));
    };
    let (uid, name, pw) = user()?;
    // Klic a povoleni vzdaleneho pristupu (jako prikazy ssh-key add
    // a remote-access on; heslo chtene). Ucet muze byt po predchozich
    // sadach (`ucty`: spatna hesla) zpomaleny: pockat (ShouldWait), pak
    // spravne heslo (vynuluje pocitadlo) a az potom spatne.
    let end = time::deadline_after(Duration::from_secs(180));
    loop {
        match crate::sshd::account_admin(uid, Request::SshKeyAdd { password: Secret::from(pw), line: key.clone() }) {
            Ok(_) | Err(libos::Error::AlreadyExists) => break,
            Err(libos::Error::ShouldWait) if time::now() < end => {
                crate::terminal::TERMINALS.lock().poll(startup);
                time::sleep(Duration::from_millis(500));
            }
            Err(e) => return Err(fail("ssh-key add", e)),
        }
    }
    match crate::sshd::account_admin(uid, Request::SshKeyAdd { password: Secret::from("spatne heslo!"), line: key }) {
        Err(libos::Error::AccessDenied) => {}
        other => return Err(format!("klic bez spravneho hesla: {other:?}")),
    }
    crate::sshd::account_admin(uid, Request::SetRemote { password: Secret::from(pw), on: true }).map_err(|e| fail("remote-access on", e))?;
    // Verejna sit (vychozi v QEMU) prichozi spojeni zahodi: SSH jen
    // v domaci siti nebo s vyjimkou firewallu (docs/cs/ssh.md).
    let root = &net_service().ok_or("sit nebezi")?.root;
    let fw = super::firewall::agent()?;
    let before = root.stats().map_err(|e| fail("STATS", e))?.0;
    let from = super::firewall::audit_end(root)?;
    fw.profile("home")?;
    super::firewall::wait_profile(root, from, "home")?;
    crate::sshd::set_enabled(true).map_err(|e| fail("zapnuti", e))?;
    crate::sshd::start().map_err(|e| fail("start sshd", e))?;
    println!("init: test ssh: sshd bezi, klient hostitele se pripoji jako {name}");
    let r = helper(startup, port, &format!("positive {name}"));
    // Vzdalene relace po odpojeni skonci.
    let ended = (|| -> TestResult {
        r.clone()?;
        let end = time::deadline_after(Duration::from_secs(20));
        while crate::remote::summary().0 > 0 {
            crate::terminal::TERMINALS.lock().poll(startup);
            check!(time::now() < end, format!("vzdalene relace po odpojeni bezi: {:?}", crate::remote::summary().1));
            time::sleep(Duration::from_millis(100));
        }
        Ok(())
    })();
    // Vypnuti: server spojeni neprijme.
    crate::sshd::stop();
    let _ = crate::sshd::set_enabled(false);
    let neg = helper(startup, port, "negative");
    let _ = crate::sshd::account_admin(uid, Request::SetRemote { password: Secret::default(), on: false });
    let from = super::firewall::audit_end(root)?;
    fw.profile(&before)?;
    super::firewall::wait_profile(root, from, &before)?;
    ended?;
    neg
}
