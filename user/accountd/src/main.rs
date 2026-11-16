//! accountd: sluzba uzivatelskych uctu (popis `docs/cs/ucty.md`).
//!
//! Handly ve startovni zprave (kind USER, v tomto poradi): relace FS
//! (`/db` = `/system/accounts`, ucet 902), jeji listek, kanal k initu
//! (`accountproto::ToInit`).
//!
//! Bootstrap: prvni zprava = konfigurace (TOML: `kdf = "qemu" | "hw"`,
//! `dev = true` jen ve vyvojovem behu), odpoved `ok`. Potom init posila
//! `login` / `setup` / `key` + handle (kanal prihlasovaci obrazovky,
//! instalatoru, bezpecnostniho klice) a `status`.
//!
//! Tajemstvi se nikdy nevypisuji; prijate zpravy se po zpracovani
//! vynuluji.

#![no_std]
#![no_main]

extern crate alloc;

mod admin;
#[cfg(feature = "dev-mode")]
mod devmode;
mod perms;
mod remote;
mod os;
mod service;
mod store;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use accountproto::{Method, Request, Secret};
use accounts::{Config, Engine, KdfParams};
use fsproto::FsClient;
use libos::{Channel, Error, Signals, Startup, println};
use zeroize::Zeroize;

use os::OsRng;
use service::Service;
use store::FsStore;

libos::entry!(main);

fn config(text: &str) -> Result<Config, String> {
    let t = conf::parse(text).map_err(|e| format!("{e}"))?;
    let params = match t.get_str("kdf").unwrap_or("hw") {
        "qemu" => KdfParams::QEMU,
        "hw" => KdfParams::HW,
        other => return Err(format!("neznamy profil kdf {other}")),
    };
    // Vyvojove prihlaseni: init ho zapne jen se souborem `accounts-dev`
    // v initfs, ktery pridavaji jen `cargo xtask run` a `cargo xtask usb`
    // (zive USB = vyvojovy profil). Obraz pro uzivatele (`xtask image`,
    // `xtask vmware`) ani testy ho nemaji.
    let dev = t.get_bool("dev").unwrap_or(false);
    Ok(Config { params, dev })
}

/// Vyvojovy ucet a jeho automaticke prihlaseni.
fn dev_login(svc: &mut Service) {
    let uid = match svc.eng.prepare_dev(service::now()) {
        Ok(Some(a)) => match svc.create_account(a) {
            Ok(uid) => uid,
            Err(e) => {
                println!("accountd: vyvojovy ucet nelze zalozit: {e}");
                return;
            }
        },
        Ok(None) => match svc.eng.by_name(accounts::DEV_NAME) {
            Some(a) => a.uid,
            None => return,
        },
        Err(e) => {
            println!("accountd: vyvojovy ucet: {e}");
            return;
        }
    };
    // Vyvojovy ucet ma roli spravy (i ucet z drivejsi verze).
    if let Err(e) = svc.eng.ensure_dev_admin() {
        println!("accountd: role vyvojoveho uctu: {e}");
    }
    match svc.eng.authenticate(uid, Method::Dev, &Secret::default(), None, service::now()) {
        Ok(o) if o.is_ok() => match svc.open_session(uid, None) {
            Ok(s) => println!("accountd: vyvojove prihlaseni {} (relace {s})", accounts::DEV_NAME),
            Err(e) => println!("accountd: vyvojova relace: {e}"),
        },
        Ok(_) => println!("accountd: vyvojove prihlaseni odmitnuto"),
        Err(e) => println!("accountd: vyvojove prihlaseni: {e}"),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Login,
    Setup,
    /// Sluzby ukonu spravy (`CHECK_GRANT`).
    Grants,
    /// Zobrazovana jmena uctu (`NAMES`, okno spravce souboru).
    Names,
    /// Sluzba sshd: overeni klicem na dalku (`SSH_PROBE`, `SSH_AUTH`).
    Ssh,
    /// Init jmenem relace shellu: klice SSH a vzdaleny pristup uctu
    /// (zprava = `u64` uid LE + pozadavek; prikazy `ssh-key`,
    /// `remote-access`).
    AccountAdmin,
}

fn main(startup: Startup) -> i64 {
    let bootstrap = startup.bootstrap;
    let mut handles = startup.handles.into_iter();
    let (Some(fs), Some(lease), Some(init)) = (handles.next(), handles.next(), handles.next()) else {
        println!("accountd: chybi handly (FS, listek, init)");
        return 1;
    };
    let fs = FsClient::with_lease(Channel::from_handle(fs), Channel::from_handle(lease));
    let init = Channel::from_handle(init);
    let cfg = match bootstrap.read(None) {
        Ok(m) => match core::str::from_utf8(&m.data).map_err(|_| String::from("neni UTF-8")).and_then(config) {
            Ok(c) => c,
            Err(e) => {
                let _ = bootstrap.write(format!("err:{e}").as_bytes(), Vec::new());
                return 2;
            }
        },
        Err(_) => return 2,
    };
    let store = match FsStore::new(fs) {
        Ok(s) => s,
        Err(e) => {
            let _ = bootstrap.write(format!("err:DB {e}").as_bytes(), Vec::new());
            return 3;
        }
    };
    let device = match store.lazy_key() {
        Ok(k) => k,
        Err(e) => {
            let _ = bootstrap.write(format!("err:klic zarizeni {e}").as_bytes(), Vec::new());
            return 3;
        }
    };
    let eng = match Engine::new(store, device, OsRng, cfg) {
        Ok(e) => e,
        Err(e) => {
            let _ = bootstrap.write(format!("err:DB {e}").as_bytes(), Vec::new());
            return 3;
        }
    };
    for (name, e) in &eng.broken {
        println!("accountd: poskozeny zaznam uctu {name}: {e}");
    }
    let mut svc = Service { eng, init, sessions: Vec::new(), keys: Vec::new(), events: Vec::new(), admin: Default::default(), perms: Default::default(), remote: Default::default() };
    if bootstrap.write(b"ok", Vec::new()).is_err() {
        return 4;
    }
    println!(
        "accountd: bezim ({} uctu, Argon2id {} MiB t={}{})",
        svc.eng.accounts().len(),
        cfg.params.m_kib / 1024,
        cfg.params.t,
        if cfg.dev { ", vyvojove prihlaseni" } else { "" }
    );
    if cfg.dev {
        dev_login(&mut svc);
    }

    let mut clients: Vec<(Channel, Kind)> = Vec::new();
    // Klientu je promenny pocet: cekani pres port (`WaitSet`), ne pres
    // `wait_many` (limit 64 objektu).
    let mut waits = match libos::WaitSet::new() {
        Ok(w) => w,
        Err(e) => {
            println!("accountd: port nejde vytvorit: {e}");
            return 5;
        }
    };
    loop {
        for e in core::mem::take(&mut svc.events) {
            svc.init_event(&e);
        }
        let rd = Signals::READABLE | Signals::PEER_CLOSED;
        let mut items: Vec<(&libos::Handle, Signals)> = Vec::new();
        items.push((bootstrap.handle(), rd));
        items.push((svc.init.handle(), rd));
        for (c, _) in &clients {
            items.push((c.handle(), rd));
        }
        for s in &svc.sessions {
            items.push((s.chan.handle(), rd));
        }
        if let Some(d) = &svc.admin.dialog {
            items.push((d.handle(), rd));
        }
        // Kanaly povoleni az posledni (indexy vyse jsou pevne).
        let perm_idx = svc.perms.chan.as_ref().map(|p| {
            items.push((p.handle(), rd));
            items.len() - 1
        });
        let perm_admin_idx = svc.perms.admin.as_ref().map(|p| {
            items.push((p.handle(), rd));
            items.len() - 1
        });
        let observed = match waits.wait(&items, None) {
            Ok((_, o)) => o,
            Err(e) => {
                println!("accountd: cekani: {e}");
                return 5;
            }
        };
        drop(items);
        let nc = clients.len();
        let svc_sessions_before = svc.sessions.len();
        // Relace (od konce: odebirani nemeni nizsi indexy).
        for i in (0..svc.sessions.len()).rev() {
            let s = observed[2 + nc + i];
            if s.contains(Signals::READABLE) {
                while let Ok(mut msg) = svc.sessions[i].chan.try_read() {
                    let reply = match Request::decode(&msg.data) {
                        Ok(r) if r.is_self() => svc.own(i, r),
                        Ok(_) => Some(accountproto::reply_err(Error::AccessDenied)),
                        Err(e) => Some(accountproto::reply_err(e)),
                    };
                    msg.data.zeroize();
                    if let Some(reply) = reply {
                        let _ = svc.sessions[i].chan.write(&reply, Vec::new());
                    }
                    if svc.sessions[i].closed {
                        break;
                    }
                }
            }
            if svc.sessions[i].closed {
                svc.sessions.remove(i);
            }
            // Zavreny kanal self relaci nekonci (relaci drzi init).
        }
        for i in (0..nc).rev() {
            let s = observed[2 + i];
            if s.contains(Signals::READABLE) {
                while let Ok(mut msg) = clients[i].0.try_read() {
                    let reply = match (Request::decode(&msg.data), clients[i].1) {
                        (_, Kind::Grants) => svc.check_grant(&msg.data),
                        (Ok(r), Kind::Login) if r.is_login() => svc.login(r),
                        (Ok(r), Kind::Setup) => svc.setup(r),
                        (Ok(Request::Names), Kind::Names) => svc.names(),
                        (Ok(r), Kind::Ssh) if r.is_ssh() => svc.ssh(r),
                        (_, Kind::AccountAdmin) => svc.account_admin(&msg.data),
                        (Ok(_), _) => accountproto::reply_err(Error::AccessDenied),
                        (Err(e), _) => accountproto::reply_err(e),
                    };
                    msg.data.zeroize();
                    let _ = clients[i].0.write(&reply, Vec::new());
                }
            } else if s.contains(Signals::PEER_CLOSED) {
                clients.swap_remove(i);
            }
        }
        // Odpovedi dialogu spravy (signal za relacemi).
        if svc.admin.dialog.is_some() && observed.get(2 + nc + svc_sessions_before).is_some_and(|s| s.intersects(rd)) {
            for a in svc.dialog_answers() {
                if let Some(s) = svc.sessions.iter().find(|s| s.id == a.session) {
                    let _ = s.chan.write(&a.reply, Vec::new());
                }
            }
        }
        if perm_idx.is_some_and(|i| observed.get(i).is_some_and(|s| s.intersects(rd))) {
            svc.perm_requests();
        }
        if perm_admin_idx.is_some_and(|i| observed.get(i).is_some_and(|s| s.intersects(rd))) {
            svc.perm_admin();
        }
        if observed[1].contains(Signals::READABLE) {
            while let Ok(msg) = svc.init.try_read() {
                svc.init_event(&msg.data);
            }
        } else if observed[1].contains(Signals::PEER_CLOSED) {
            println!("accountd: init zavrel kanal, koncim");
            return 0;
        }
        if !observed[0].intersects(Signals::READABLE | Signals::PEER_CLOSED) {
            continue;
        }
        let msg = match bootstrap.try_read() {
            Ok(m) => m,
            Err(Error::ShouldWait) => continue,
            Err(Error::PeerClosed) => return 0,
            Err(e) => {
                println!("accountd: cteni: {e}");
                return 6;
            }
        };
        let kind = match &msg.data[..] {
            b"login" => Some(Kind::Login),
            b"setup" => Some(Kind::Setup),
            b"grants" => Some(Kind::Grants),
            b"names" => Some(Kind::Names),
            b"ssh" => Some(Kind::Ssh),
            b"account-admin" => Some(Kind::AccountAdmin),
            _ => None,
        };
        let reply = if let Some(k) = kind {
            match msg.handles.into_iter().next() {
                Some(h) => {
                    clients.push((Channel::from_handle(h), k));
                    String::from("ok")
                }
                None => String::from("err:chybi handle"),
            }
        } else if msg.data == b"dialog" {
            // Kanal duveryhodneho dialogu od kompozitoru (i nova instance).
            match msg.handles.into_iter().next() {
                Some(h) => {
                    svc.admin.dialog = Some(Channel::from_handle(h));
                    // Nova instance kompozitoru dostane stav rezimu vyvojare.
                    #[cfg(feature = "dev-mode")]
                    svc.dev_mode_to_bar();
                    String::from("ok")
                }
                None => String::from("err:chybi handle"),
            }
        } else if msg.data == b"permissions-admin" {
            // Kanal initu pro spravu rozhodnuti (prikaz shellu `povoleni`).
            match msg.handles.into_iter().next() {
                Some(h) => {
                    svc.perms.admin = Some(Channel::from_handle(h));
                    String::from("ok")
                }
                None => String::from("err:chybi handle"),
            }
        } else if msg.data == b"permissions" {
            // Kanal sluzby zarizeni pro povoleni aplikaci (`perms.rs`).
            match msg.handles.into_iter().next() {
                Some(h) => {
                    svc.perms.chan = Some(Channel::from_handle(h));
                    String::from("ok")
                }
                None => String::from("err:chybi handle"),
            }
        } else if msg.data == b"key" {
            match msg.handles.into_iter().next() {
                Some(h) => {
                    svc.keys.push(Channel::from_handle(h));
                    String::from("ok")
                }
                None => String::from("err:chybi handle"),
            }
        } else if msg.data == b"status" {
            format!(
                "ok:accounts={} sessions={} broken={} keys={}",
                svc.eng.accounts().len(),
                svc.sessions.len(),
                svc.eng.broken.len(),
                svc.keys.len()
            )
        } else {
            String::from("err:neznamy prikaz")
        };
        if bootstrap.write(reply.as_bytes(), Vec::new()).is_err() {
            return 0;
        }
    }
}
