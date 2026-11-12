//! Kanal `login` ke sluzbe uctu pro prihlasovaci a zamykaci obrazovku.
//!
//! Pozadavky obrazovky (`comp::greeter::Request`) jdou kanalem hned
//! (asynchronne: overeni Argon2id nebo dotek bezpecnostniho klice
//! nezdrzi snimek); odpovedi chodi ve stejnem poradi, proto si kanal
//! pamatuje druhy cekajicich pozadavku. Heslo se po zakodovani vynuluje.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use accountproto::{AuthReply, Request, Secret, decode_list, parse_reply};
use comp::Comp;
use comp::greeter::{Account, Method, Reply, Request as GReq};
use libos::{Channel, Error, Handle, Signals};

#[derive(Clone, Copy)]
enum Kind {
    List,
    Phrase(u64),
    Auth,
    Lock,
}

pub struct Login {
    pub ch: Channel,
    pending: VecDeque<Kind>,
}

fn method(m: Method) -> accountproto::Method {
    match m {
        Method::Password => accountproto::Method::Password,
        Method::Pin => accountproto::Method::Pin,
        Method::Key => accountproto::Method::Key,
        Method::Recovery => accountproto::Method::Recovery,
    }
}

fn wipe(b: &mut [u8]) {
    for x in b.iter_mut() {
        // SAFETY: platny jedinecny odkaz na bajt; volatile zapis prekladac
        // nevynecha.
        unsafe { core::ptr::write_volatile(x, 0) };
    }
}

fn secret(s: String) -> Secret {
    let mut v = s.into_bytes();
    let out = Secret::from_slice(&v);
    wipe(&mut v);
    out
}

impl Login {
    pub fn new(ch: Channel) -> Self {
        Login { ch, pending: VecDeque::new() }
    }

    pub fn handle(&self) -> &Handle {
        self.ch.handle()
    }

    /// Posle pozadavky obrazovky.
    pub fn send(&mut self, reqs: Vec<GReq>) -> Result<(), Error> {
        for r in reqs {
            let (req, kind) = match r {
                GReq::List => (Request::List, Kind::List),
                GReq::Phrase(uid) => (Request::Phrase { uid }, Kind::Phrase(uid)),
                GReq::Auth { uid, method: m, secret: s } => (Request::Auth { uid, method: method(m), secret: secret(s) }, Kind::Auth),
                GReq::Unlock { session, method: m, secret: s } => {
                    (Request::Unlock { session, method: method(m), secret: secret(s) }, Kind::Auth)
                }
                GReq::Lock { session } => (Request::LockSession { session }, Kind::Lock),
            };
            let mut b = req.encode();
            let w = self.ch.write(&b, Vec::new());
            wipe(&mut b);
            w?;
            self.pending.push_back(kind);
        }
        Ok(())
    }

    /// Precte odpovedi; `Err` = sluzba uctu kanal zavrela.
    pub fn read(&mut self, sig: Signals, c: &mut Comp, now: u64) -> Result<(), Error> {
        if !sig.intersects(Signals::READABLE | Signals::PEER_CLOSED) {
            return Ok(());
        }
        loop {
            let m = match self.ch.try_read() {
                Ok(m) => m,
                Err(Error::ShouldWait) => return Ok(()),
                Err(e) => {
                    // Rozpracovane overeni skonci chybou (obrazovka nevisi).
                    if self.pending.iter().any(|k| matches!(k, Kind::Auth)) {
                        c.login_reply(Reply::Error(String::from("služba účtů neběží")), now);
                    }
                    self.pending.clear();
                    return Err(e);
                }
            };
            let Some(kind) = self.pending.pop_front() else { continue };
            let body = parse_reply(&m.data);
            let r = match (kind, body) {
                (Kind::List, Ok(b)) => match decode_list(b) {
                    Ok(list) => {
                        // Pocet uctu do logu (jmena ne): znacka startu do
                        // prihlasovaci obrazovky (test zive USB).
                        libos::println!("compd: prihlasovaci obrazovka: {} uctu", list.len());
                        Reply::Accounts(
                        list.into_iter()
                            .map(|a| Account { uid: a.uid, name: a.name, display: a.display, methods: a.methods, flags: a.flags })
                            .collect(),
                        )
                    }
                    Err(_) => continue,
                },
                (Kind::Phrase(uid), Ok(b)) => match wire::Reader::new(b).str() {
                    Ok(p) => Reply::Phrase(uid, p),
                    Err(_) => continue,
                },
                (Kind::Auth, Ok(b)) => match AuthReply::decode(b) {
                    Ok(AuthReply::Ok { .. }) => Reply::Ok,
                    Ok(AuthReply::Denied { retry_after_ms, pin_left }) => Reply::Denied { retry_after_ms, pin_left },
                    Ok(AuthReply::LockedOut { retry_after_ms }) => Reply::LockedOut { retry_after_ms },
                    Err(e) => Reply::Error(format!("{e}")),
                },
                (Kind::Auth, Err(e)) => Reply::Error(match e {
                    Error::Locked => String::from("domov je zamčený, použij heslo"),
                    Error::NotFound => String::from("bezpečnostní klíč není připojený"),
                    e => format!("{e}"),
                }),
                (Kind::Lock, Err(e)) => {
                    libos::println!("compd: zamceni relace: {e}");
                    continue;
                }
                (Kind::Lock, Ok(_)) | (Kind::List, Err(_)) | (Kind::Phrase(_), Err(_)) => continue,
            };
            c.login_reply(r, now);
        }
    }
}
