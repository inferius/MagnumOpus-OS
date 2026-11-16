//! Vzdaleny pristup SSH (`docs/cs/ssh.md`): zapnuti sluzby `sshd`, klic
//! hostitele, vyzvednuti vzdalenych relaci a prikazy shellu `sshd`,
//! `ssh-key`, `remote-access`.
//!
//! - Ve vychozim stavu vypnuto. Zapnout i vypnout je ukon spravy
//!   (`ELEVATE` `SSHD`, dialog ukaze otisk klice hostitele). Stav
//!   a klic jsou v `/system/config/ssh` (jen system, bez grantu).
//! - Klic hostitele ed25519 vznikne pri prvnim zapnuti z CSPRNG jadra
//!   (`fill_seeded`: ceka, az je generator nasazeny).
//! - sshd dostane relaci site jen s naslouchanim a TCP, kanal `ssh`
//!   ke sluzbe uctu a initfs (procesy spojeni). Identitu nikdy neurcuje:
//!   relaci si vyzvedne listkem od accountd (`claim`) a dostane kanal
//!   okna terminalu (termd v jobu vzdalene relace) a relaci FS uctu.
//! - Klice SSH a vzdaleny pristup uctu jdou pres kanal `account-admin`
//!   sluzby uctu jmenem uzivatele relace shellu.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use accountproto::elevate::{action, encode_check};
use accountproto::client::SelfService;
use accountproto::{Request, Secret, flags};
use fsproto::FsClient;
use fsproto::perm::{ACL_PROTECTED, Acl};
use libos::spawn::StartHandle;
use libos::abi::bootstrap::kind;
use libos::{Channel, Error, Process, Result, println, time};
use powerproto::Reply;

pub const DIR: &str = "/system/config/ssh";
const KEY: &str = "/system/config/ssh/host_ed25519";
const ENABLED: &str = "/system/config/ssh/enabled";
/// Port SSH.
pub const PORT: u16 = 22;

struct Sshd {
    process: Process,
}

static SSHD: spin::Mutex<Option<Sshd>> = spin::Mutex::new(None);

fn fs() -> Result<&'static FsClient> {
    crate::system::devices().and_then(|d| d.fs.as_ref()).ok_or(Error::NotFound)
}

/// Adresar jen pro system (jako `/system/config/vzhled`).
fn ensure_dir(fs: &FsClient) -> Result<()> {
    let mut dir = String::new();
    for part in DIR.split('/').filter(|p| !p.is_empty()) {
        dir.push('/');
        dir.push_str(part);
        match fs.create(&dir, fsproto::kind::DIR) {
            Ok(()) | Err(Error::AlreadyExists) => {}
            Err(e) => return Err(e),
        }
    }
    if fs.permissions(DIR)?.acl.is_none_or(|a| a.flags & ACL_PROTECTED == 0 || !a.grants.is_empty()) {
        fs.set_acl(DIR, Some(&Acl { flags: ACL_PROTECTED, grants: Vec::new() }))?;
    }
    Ok(())
}

/// Klic hostitele (32 B soukromeho klice ed25519); pri prvnim pouziti
/// vznikne a ulozi se.
fn host_key() -> Result<[u8; 32]> {
    let fs = fs()?;
    if let Some((data, _)) = crate::certs::load(fs, KEY)? {
        return data.as_slice().try_into().map_err(|_| Error::Corrupted);
    }
    ensure_dir(fs)?;
    let mut k = [0u8; 32];
    libos::random::fill_seeded(&mut k)?;
    crate::certs::store(fs, KEY, &k)?;
    println!("init: ssh: vytvoren klic hostitele {}", ssh_fingerprint(&k));
    Ok(k)
}

fn ssh_fingerprint(k: &[u8; 32]) -> String {
    ssh::keys::ed25519_public(k).fingerprint()
}

/// Je vzdaleny pristup zapnuty (ulozeny stav)?
pub fn enabled() -> bool {
    fs().is_ok_and(|fs| fs.stat(ENABLED).is_ok())
}

pub(crate) fn set_enabled(on: bool) -> Result<()> {
    let fs = fs()?;
    if on {
        ensure_dir(fs)?;
        crate::certs::store(fs, ENABLED, b"1\n").map(|_| ())
    } else {
        match fs.remove(ENABLED) {
            Ok(()) | Err(Error::NotFound) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Spusti sshd (kdyz uz nebezi). Volat po siti a sluzbe uctu.
pub fn start() -> Result<()> {
    let mut g = SSHD.lock();
    if g.is_some() {
        return Ok(());
    }
    let key = host_key()?;
    let net = crate::system::NET.get().ok_or(Error::NotFound)?;
    let initfs = crate::install::STARTUP_INITFS.get().ok_or(Error::NotFound)?;
    // Sit: jen naslouchat na portu SSH (prichozi spojeni).
    let manifest = crate::apps::manifest(initfs, "sshd")?.ok_or(Error::NotFound)?;
    let mut r = manifest.network.clone().ok_or(Error::InvalidArgs)?;
    r.policy.connect = Some(Vec::new());
    r.policy.bind = Some(vec![(PORT, PORT)]);
    let session = net.root.restrict_with(&r)?;
    let auth = crate::accounts::channel("ssh")?;
    let vmo = crate::install::STARTUP_INITFS_VMO.get().ok_or(Error::NotFound)?.handle().duplicate(None)?;
    let handles = vec![
        StartHandle::user(session.into_channel()?.into_handle()),
        StartHandle::user(auth.into_handle()),
        StartHandle::new(kind::INITFS, 0, vmo),
    ];
    let app = crate::apps::spawn_from(initfs, "sshd", handles, None, false)?;
    let mut k = hex(&key);
    let cfg = format!("port={PORT}\nkey={k}\n");
    // Klic v textu jen pro zapis do kanalu; pak smazat.
    unsafe_zero(&mut k);
    app.control.write(cfg.as_bytes(), Vec::new())?;
    drop(cfg);
    let msg = app.control.read(Some(time::deadline_after(Duration::from_secs(10))))?;
    if msg.data != b"ok" {
        println!("init: sshd hlasi: {}", String::from_utf8_lossy(&msg.data));
        let _ = app.process.kill();
        return Err(Error::IoError);
    }
    println!("init: sshd posloucha na portu {PORT} (klic hostitele {})", ssh_fingerprint(&key));
    let control = app.control;
    libos::thread::spawn("sshd-claims", move || serve(control))?;
    *g = Some(Sshd { process: app.process });
    Ok(())
}

/// Prepise retezec nulami (hex klice hostitele).
fn unsafe_zero(s: &mut String) {
    let n = s.len();
    s.clear();
    s.extend(core::iter::repeat_n('0', n));
}

/// Zastavi sshd a ukonci vzdalene relace.
pub(crate) fn stop() {
    if let Some(s) = SSHD.lock().take() {
        let _ = s.process.kill();
    }
    let ids: Vec<u64> = crate::remote::REMOTE.lock().iter().map(|s| s.id).collect();
    for id in ids {
        close_remote(id);
    }
}

/// Konec vzdalene relace: relace, terminaly a oznameni sluzbe uctu.
pub fn close_remote(id: u64) {
    crate::terminal::TERMINALS.lock().close_session(id);
    if crate::remote::close(id) {
        crate::session::notify_closed(id);
    }
}

/// Pozadavky sshd: `claim` + listek -> kanal okna terminalu a relace FS
/// uctu; `end <relace>` -> konec vzdalene relace.
fn serve(control: Channel) {
    loop {
        let m = match control.read(None) {
            Ok(m) => m,
            Err(_) => {
                println!("init: sshd skoncil");
                SSHD.lock().take();
                return;
            }
        };
        let reply: core::result::Result<(String, Vec<libos::Handle>), Error> = (|| {
            if let Some(ticket) = m.data.strip_prefix(b"claim ") {
                let id = crate::remote::claim(ticket)?;
                let win = crate::terminal::TERMINALS.lock().client_for("ssh", Some(id))?;
                let fs = crate::session::connect(id)?;
                let home = crate::session::user_name(id).map(|n| format!("/home/{n}")).unwrap_or_else(|| String::from("/"));
                return Ok((format!("ok {id} {home}"), vec![win.into_handle(), fs.into_channel().into_handle()]));
            }
            if let Some(id) = m.data.strip_prefix(b"end ") {
                let id: u64 = core::str::from_utf8(id).ok().and_then(|s| s.trim().parse().ok()).ok_or(Error::InvalidArgs)?;
                close_remote(id);
                return Ok((String::from("ok"), Vec::new()));
            }
            Err(Error::InvalidArgs)
        })();
        let _ = match reply {
            Ok((text, hs)) => control.write(text.as_bytes(), hs),
            Err(e) => control.write(format!("err {}", e.name()).as_bytes(), Vec::new()),
        };
    }
}

/// Start pri behu systemu (je-li zapnuto).
pub fn start_if_enabled() {
    if enabled()
        && let Err(e) = start()
    {
        println!("init: sshd nejde spustit: {e}");
    }
}

/// Stav pro `sshd status`.
fn status() -> String {
    let on = enabled();
    let running = SSHD.lock().is_some();
    let fp = fs().ok().and_then(|fs| crate::certs::load(fs, KEY).ok().flatten()).and_then(|(d, _)| {
        let k: [u8; 32] = d.as_slice().try_into().ok()?;
        Some(ssh_fingerprint(&k))
    });
    let (n, who) = crate::remote::summary();
    format!(
        "enabled\t{}\nrunning\t{}\nport\t{PORT}\nhost_key\t{}\nsessions\t{n}\nconnected\t{}\n",
        if on { "on" } else { "off" },
        if running { "yes" } else { "no" },
        fp.unwrap_or_else(|| String::from("-")),
        who.join(", ")
    )
}

/// Ukon spravy za relaci shellu (dialog u pocitace), pak zapnout nebo
/// vypnout.
fn elevate_sshd(session: u64, on: bool) -> Result<String> {
    let fp = if on { ssh_fingerprint(&host_key()?) } else { String::new() };
    let params = if on { format!("enable {fp}") } else { String::from("disable") }.into_bytes();
    // Vzdalena relace kanal self nema: ukon spravy jen u pocitace.
    let own = crate::session::take_self(session).ok_or(Error::AccessDenied)?;
    let me = SelfService(own);
    let r = me.elevate(action::SSHD, params.clone());
    crate::session::put_self(session, me.0);
    let token = r?;
    let grants = crate::accounts::channel("grants")?;
    grants.write(&encode_check(&token, action::SSHD, &params), Vec::new())?;
    let m = grants.read(Some(time::deadline_after(Duration::from_secs(5))))?;
    accountproto::parse_reply(&m.data)?;
    set_enabled(on)?;
    if on {
        start()?;
        println!("init: vzdaleny pristup SSH zapnut (klic hostitele {fp})");
        Ok(format!(
            "Vzdálený přístup SSH je zapnutý: port {PORT} na všech síťových rozhraních, přihlášení jen klíčem.\nOtisk klíče serveru (ověřte ho při prvním připojení): {fp}\nÚčet se připojí, až si povolí vzdálený přístup (remote-access on) a přidá klíč (ssh-key add).\n"
        ))
    } else {
        stop();
        println!("init: vzdaleny pristup SSH vypnut");
        Ok(String::from("Vzdálený přístup SSH je vypnutý; připojené relace skončily.\n"))
    }
}

/// Pozadavek `account-admin` jmenem uzivatele `uid`.
pub(crate) fn account_admin(uid: u64, req: Request) -> Result<Vec<u8>> {
    let ch = crate::accounts::channel("account-admin")?;
    let mut msg = uid.to_le_bytes().to_vec();
    msg.extend_from_slice(&req.encode());
    let w = ch.write(&msg, Vec::new());
    zeroize::Zeroize::zeroize(&mut msg);
    w?;
    let m = ch.read(Some(time::deadline_after(Duration::from_secs(30))))?;
    accountproto::parse_reply(&m.data).map(<[u8]>::to_vec)
}

/// Prikaz shellu. `Ok(Some(kanal))` = ukon spravy ceka na potvrzeni
/// (vysledek prijde na kanal).
pub fn from_shell(session: Option<u64>, op: &str, arg: &str, password: &str) -> core::result::Result<Option<Channel>, Reply> {
    let fail = |e| Reply::Failed(e);
    let session = session.ok_or(fail(Error::AccessDenied))?;
    let uid = crate::session::uid(session).ok_or(fail(Error::AccessDenied))?;
    let text = |r: Result<String>| match r {
        Ok(t) => Err(Reply::Text(t)),
        Err(e) => Err(Reply::Failed(e)),
    };
    match op {
        "sshd-status" => text(Ok(status())),
        "sshd-enable" | "sshd-disable" => {
            let on = op == "sshd-enable";
            let (mine, theirs) = Channel::create().map_err(fail)?;
            libos::thread::spawn("sshd-elevate", move || {
                let reply = match elevate_sshd(session, on) {
                    Ok(t) => Reply::Text(t),
                    Err(e) => Reply::Failed(e),
                };
                let _ = mine.write(&reply.encode(), Vec::new());
            })
            .map_err(fail)?;
            Ok(Some(theirs))
        }
        "key-add" => text(account_admin(uid, Request::SshKeyAdd { password: Secret::from(password), line: String::from(arg) }).and_then(|b| {
            let mut r = wire::Reader::new(&b);
            Ok(format!("{}\t{}\n", r.u32()?, r.str()?))
        })),
        "key-list" => text(account_admin(uid, Request::SshKeyList).and_then(|b| {
            let mut s = String::new();
            for k in accountproto::ssh::decode_keys(&b)? {
                s.push_str(&format!("{}\t{}\t{}\t{}\t{}\n", k.id, k.algo, k.fingerprint, k.comment, k.added));
            }
            Ok(s)
        })),
        "key-remove" => {
            let id: u32 = arg.trim().parse().map_err(|_| fail(Error::InvalidArgs))?;
            text(account_admin(uid, Request::SshKeyRemove { id }).map(|_| String::new()))
        }
        "access-on" | "access-off" => {
            let on = op == "access-on";
            text(account_admin(uid, Request::SetRemote { password: Secret::from(password), on }).map(|_| String::from(if on { "on\n" } else { "off\n" })))
        }
        "access" => {
            let login = accountproto::client::Login(crate::accounts::channel("login").map_err(fail)?);
            let on = login.list().map_err(fail)?.iter().any(|a| a.uid == uid && a.flags & flags::REMOTE != 0);
            text(Ok(String::from(if on { "on\n" } else { "off\n" })))
        }
        _ => Err(fail(Error::InvalidArgs)),
    }
}
