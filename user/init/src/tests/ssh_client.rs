//! Sada "ssh": klient SSH v systemu (`user/sshtools`) proti nasemu sshd ve
//! stejnem QEMU pres loopback (`docs/cs/ssh.md`, Klient).
//!
//! Uzivatel ma klic klienta ve sluzbe uctu (`ssh-keygen`), stejny verejny
//! klic je mezi klici prihlaseni k uctu. Shell relace uzivatele spusti
//! `ssh`, `scp` a `sftp` sluzbou exec; klic podepisuje sluzba uctu.
//! Overi se: prvni spojeni (dotaz known_hosts ve vstupu programu,
//! systemova vyzva "Účty" pro potvrzeni klice k serveru), druhe bez dotazu,
//! scp tam a zpet, sftp, odmitnuti zmeneneho klice serveru a ze kanal
//! podpisu nedostane program, ktery neni z initfs.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use accountproto::client::{Login, SelfService, session_of};
use accountproto::{Method, Request, Secret};
use libos::{Channel, Startup, time};
use shlang::value::Value;
use term::proto::{ClientMsg, Item, ServerMsg};

use super::shell::{Reg, Window, start_termd_fs};
use super::{TestResult, fail, net_service};
use crate::check;

/// Polozky bloku a stitky systemovych vyzev, ktere radek ukazal.
struct Run {
    items: Vec<Item>,
    ok: bool,
    labels: Vec<String>,
}

impl Run {
    fn texts(&self) -> Vec<String> {
        self.items
            .iter()
            .filter_map(|i| match i {
                Item::Value(Value::Str(s)) | Item::Note(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    }
}

/// Radek s odpovedmi: vyzvy vstupu programu dostanou postupne `stdin`
/// (pak se zavrou), systemove vyzvy (stitek) potvrzeni `system`.
fn run(s: &Startup, w: &Window, id: u64, line: &str, stdin: &[&str], system: bool) -> Result<Run, String> {
    w.send(ClientMsg::Run { session: id, line: line.into() })?;
    let mut ours = None;
    let mut r = Run { items: Vec::new(), ok: false, labels: Vec::new() };
    let mut next = stdin.iter();
    let end = time::deadline_after(Duration::from_secs(120));
    loop {
        if time::now() > end {
            return Err(format!("'{line}' neskoncil: {:?}", r.texts()));
        }
        // V testovacim behu nebezi supervisor: terminaly vzdalenych relaci
        // (sshd) obsluhuje test.
        crate::terminal::TERMINALS.lock().poll(s);
        let Some(m) = w.recv_for(100)? else { continue };
        match m {
            ServerMsg::BlockStart { block, .. } if ours.is_none() => ours = Some(block),
            ServerMsg::Prompt { prompt, label, .. } => {
                r.labels.push(label);
                w.send(ClientMsg::Answer { prompt, yes: system })?;
            }
            ServerMsg::Ask { prompt, label, .. } => {
                if !label.is_empty() {
                    r.labels.push(label);
                }
                match next.next() {
                    Some(t) => w.send(ClientMsg::Text { prompt, text: String::from(*t) })?,
                    None => w.send(ClientMsg::Answer { prompt, yes: false })?,
                }
            }
            ServerMsg::Item { item, block, .. } if Some(block) == ours => r.items.push(item),
            ServerMsg::BlockEnd { ok, block, .. } if Some(block) == ours => {
                r.ok = ok;
                return Ok(r);
            }
            _ => {}
        }
    }
}

pub fn client(s: &Startup) -> TestResult {
    let (uid, name, pw) = super::ssh::user()?;
    // Klic klienta ve sluzbe uctu a tentyz verejny klic k prihlaseni.
    let b = crate::sshd::account_admin(uid, Request::ClientKeyCreate { password: Secret::from(pw), comment: String::from("klient@qemu") })
        .map_err(|e| fail("ssh-keygen", e))?;
    let (_, line, fp) = accountproto::agent::decode_created(&b).map_err(|e| fail("odpoved ssh-keygen", e))?;
    check!(line.starts_with("ssh-ed25519 ") && fp.starts_with("SHA256:"), format!("verejny klic: {line}"));
    match crate::sshd::account_admin(uid, Request::SshKeyAdd { password: Secret::from(pw), line: line.clone() }) {
        Ok(_) | Err(libos::Error::AlreadyExists) => {}
        Err(e) => return Err(fail("ssh-key add", e)),
    }
    crate::sshd::account_admin(uid, Request::SetRemote { password: Secret::from(pw), on: true }).map_err(|e| fail("remote-access on", e))?;
    // Prichozi spojeni jen v domaci siti (i loopback pres firewall).
    let root = &net_service().ok_or("sit nebezi")?.root;
    let fw = super::firewall::agent()?;
    let before = root.stats().map_err(|e| fail("STATS", e))?.0;
    let from = super::firewall::audit_end(root)?;
    fw.profile("home")?;
    super::firewall::wait_profile(root, from, "home")?;
    crate::sshd::set_enabled(true).map_err(|e| fail("zapnuti sshd", e))?;
    crate::sshd::start().map_err(|e| fail("start sshd", e))?;
    // Mistni relace prihlasena heslem: klice klienta odemcene.
    let login = Login(crate::accounts::channel("login").map_err(|e| fail("kanal login", e))?);
    let reply = login.auth(uid, Method::Password, Secret::from(pw)).map_err(|e| fail("prihlaseni", e))?;
    let session = session_of(reply).map_err(|e| fail("relace", e))?;
    let r = in_session(s, session, name, &line);
    crate::exec::close_session(session);
    if let Some(ch) = crate::session::take_self(session) {
        let _ = SelfService(ch).logout();
    }
    crate::sshd::stop();
    let _ = crate::sshd::set_enabled(false);
    let _ = crate::sshd::account_admin(uid, Request::SetRemote { password: Secret::default(), on: false });
    let from = super::firewall::audit_end(root)?;
    fw.profile(&before)?;
    super::firewall::wait_profile(root, from, &before)?;
    r
}

fn in_session(s: &Startup, session: u64, name: &str, own_line: &str) -> TestResult {
    let fs = crate::session::connect(session).map_err(|e| fail("relace FS", e))?;
    let user_fs = crate::session::connect(session).map_err(|e| fail("relace FS", e))?;
    let exec = crate::exec::endpoint(session).map_err(|e| fail("kanal exec", e))?;
    let mut reg = Reg { store: Default::default(), chan: None };
    reg.store.home = format!("/home/{name}");
    let (proc, w) = start_termd_fs(s, &mut reg, fs, None, Some(exec))?;
    let home = format!("/home/{name}");
    let r = (|| -> TestResult {
        w.send(ClientMsg::Attach { session: 0 })?;
        let id = match w.recv()? {
            ServerMsg::Attached { session, .. } => session,
            other => return Err(format!("cekal jsem Attached: {other:?}")),
        };
        // Prvni spojeni: neznamy server (vstup programu: yes), klic pro
        // novy server (systemova vyzva Účty: ano).
        let r = run(s, &w, id, "ssh 127.0.0.1 echo ahoj-klient", &["yes"], true)?;
        check!(r.ok && r.texts().iter().any(|t| t == "ahoj-klient"), format!("prvni ssh: {:?}", r.texts()));
        check!(r.labels.iter().any(|l| l == term::program::SYSTEM_LABEL), format!("systemova vyzva se stitkem: {:?}", r.labels));
        let kh = user_fs.read(&format!("{home}/.ssh/known_hosts"), 0, 4096).map_err(|e| fail("known_hosts", e))?;
        check!(String::from_utf8_lossy(&kh).starts_with("127.0.0.1 ssh-ed25519 "), "known_hosts zapsany");
        // Druhe: bez dotazu, kod prikazu na serveru.
        let r = run(s, &w, id, "ssh 127.0.0.1 echo dva", &[], false)?;
        check!(r.ok && r.texts().iter().any(|t| t == "dva") && r.labels.is_empty(), format!("druhe ssh: {:?} {:?}", r.texts(), r.labels));
        let r = run(s, &w, id, "ssh 127.0.0.1 rm /neni-tu", &[], false)?;
        check!(!r.ok, "chyba prikazu na serveru = chyba radku");
        // scp tam a zpet (pres SFTP), sftp vypis.
        let data: Vec<u8> = (0..70_000u32).map(|i| (i % 251) as u8).collect();
        let _ = user_fs.create(&format!("{home}/scp-zdroj.bin"), fsproto::kind::FILE);
        user_fs.write(&format!("{home}/scp-zdroj.bin"), 0, &data).map_err(|e| fail("zdroj scp", e))?;
        let r = run(s, &w, id, "scp ~/scp-zdroj.bin 127.0.0.1:scp-cil.bin", &[], false)?;
        check!(r.ok, format!("scp nahoru: {:?}", r.texts()));
        let r = run(s, &w, id, "scp 127.0.0.1:scp-cil.bin ~/scp-zpet.bin", &[], false)?;
        check!(r.ok, format!("scp dolu: {:?}", r.texts()));
        let back = user_fs.read(&format!("{home}/scp-zpet.bin"), 0, 100_000).map_err(|e| fail("stazeny soubor", e))?;
        check!(back == data, format!("scp obsah: {} B tam, {} B zpet", data.len(), back.len()));
        let r = run(s, &w, id, "\"ls\\nrm scp-cil.bin\\n\" | sftp 127.0.0.1", &[], false)?;
        check!(r.ok && r.texts().iter().any(|t| t.starts_with("scp-cil.bin")), format!("sftp: {:?}", r.texts()));
        // Program jmenem ssh v /system/bin: jmeno si nevezme (INITFS_ONLY),
        // pobezi systemovy klient.
        let sys = super::devices().and_then(|d| d.fs.as_ref()).ok_or("FS nebezi")?;
        let _ = sys.create("/system/bin", fsproto::kind::DIR);
        let _ = sys.create("/system/bin/ssh", fsproto::kind::FILE);
        sys.write("/system/bin/ssh", 0, b"podvrh").map_err(|e| fail("podvrh", e))?;
        let r = run(s, &w, id, "ssh 127.0.0.1 echo tri", &[], false);
        let _ = sys.remove("/system/bin/ssh");
        let r = r?;
        check!(r.ok && r.texts().iter().any(|t| t == "tri"), format!("ssh s podvrhem v /system/bin: {:?}", r.texts()));
        // Zmeneny klic serveru: odmitnuto s varovanim.
        let fake = own_line.split_whitespace().nth(1).unwrap_or("");
        let kh = String::from_utf8_lossy(&kh).into_owned();
        let changed: String = kh
            .lines()
            .map(|l| {
                let f: Vec<&str> = l.split_whitespace().collect();
                if f.len() >= 3 { format!("{} {} {fake}\n", f[0], f[1]) } else { format!("{l}\n") }
            })
            .collect();
        let path = format!("{home}/.ssh/known_hosts");
        let _ = user_fs.remove(&path);
        let _ = user_fs.create(&path, fsproto::kind::FILE);
        user_fs.write(&path, 0, changed.as_bytes()).map_err(|e| fail("zmena known_hosts", e))?;
        let r = run(s, &w, id, "ssh 127.0.0.1 echo nesmi", &[], false)?;
        check!(!r.ok && !r.texts().iter().any(|t| t == "nesmi"), format!("zmeneny klic prosel: {:?}", r.texts()));
        check!(r.texts().iter().any(|t| t.contains("ZMĚNIL")), format!("varovani o zmene klice: {:?}", r.texts()));
        for f in ["scp-zdroj.bin", "scp-zpet.bin", ".ssh/known_hosts"] {
            let _ = user_fs.remove(&format!("{home}/{f}"));
        }
        Ok(())
    })();
    let _ = proc.kill();
    r?;
    agent_needs_initfs()
}

/// launchd nepreda kanal podpisu programu bez `INITFS_ONLY`.
fn agent_needs_initfs() -> TestResult {
    let ch = crate::launch::client().map_err(|e| fail("kanal launchd", e))?;
    let job = libos::job::current().map_err(|e| fail("job", e))?.duplicate(None).map_err(|e| fail("job", e))?;
    let (a, _b) = Channel::create().map_err(|e| fail("kanal", e))?;
    let l = launchproto::Launch {
        program: String::from("std-exec"),
        flags: 0,
        roles: alloc::vec![launchproto::Role::Job, launchproto::Role::Agent],
        params: Vec::new(),
    };
    ch.write(&l.encode(7).map_err(|e| fail("LAUNCH", e))?, alloc::vec![job.into_handle(), a.into_handle()]).map_err(|e| fail("LAUNCH", e))?;
    let m = ch.read(Some(time::deadline_after(Duration::from_secs(10)))).map_err(|e| fail("odpoved launchd", e))?;
    let (_, r) = launchproto::parse_reply(&m.data).map_err(|e| fail("odpoved launchd", e))?;
    check!(matches!(r, Err((libos::Error::AccessDenied, _))), format!("Agent bez INITFS_ONLY: {r:?}"));
    Ok(())
}
