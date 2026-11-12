//! Obsluha pozadavku kanalu login, self a setup a rozhovor s initem.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use accountproto::ctap::hidclient::HidTransport;
use accountproto::ctap::{Client, Transport};
use accountproto::{
    AuthReply, Method, Request, Secret, ToInit, WhoAmI, encode_list, flags, parse_reply, reply_err, reply_ok,
};
use accounts::{Engine, Outcome};
use libos::{Channel, Error, Handle, Result, println, time};
use wire::Writer;

use crate::os::{ChannelHid, OsRng};
use crate::store::FsStore;

pub type Eng = Engine<FsStore, crate::store::LazyKey, OsRng>;

/// Prihlasena relace (cislo od initu) a jeji kanal self.
pub struct Session {
    pub id: u64,
    pub uid: u64,
    pub chan: Channel,
    pub locked: bool,
    /// Odhlaseno: kanal se zavre po odeslani odpovedi.
    pub closed: bool,
}

pub struct Service {
    pub eng: Eng,
    pub init: Channel,
    pub sessions: Vec<Session>,
    /// Kanaly bezpecnostnich klicu (hlaseni HID). Init posila kanal za
    /// kazde zarizeni HID; ovladac zarizeni, ktere klic neni, ho zavre.
    pub keys: Vec<Channel>,
    /// Udalosti initu prijate behem cekani na odpoved.
    pub events: Vec<Vec<u8>>,
    /// Sprava pocitace: dialog, cekajici ukony, granty, rezim spravy.
    pub admin: crate::admin::Admin,
}

/// Realny cas v sekundach (zpomaleni po chybach).
pub fn now() -> i64 {
    time::wall().map_or(0, |t| t.secs)
}

const INIT_TIMEOUT: Duration = Duration::from_secs(30);

impl Service {
    /// Pozadavek na init; zpravy udalosti (kod operace > 0) cekajici pred
    /// odpovedi si odlozi.
    pub fn to_init(&mut self, m: &ToInit, handles: Vec<Handle>) -> Result<Vec<u8>> {
        let mut req = m.encode();
        self.init.write(&req, handles)?;
        zeroize::Zeroize::zeroize(&mut req);
        loop {
            let msg = self.init.read(Some(time::deadline_after(INIT_TIMEOUT)))?;
            let code = msg.data.get(..4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
            if code.is_some_and(|c| c > 0) {
                self.events.push(msg.data);
                continue;
            }
            return parse_reply(&msg.data).map(<[u8]>::to_vec);
        }
    }

    /// Posledni pripojeny klic (zavrene kanaly zahodi, stara hlaseni
    /// vycte).
    pub fn key(&mut self) -> Option<Channel> {
        self.keys.retain(|k| {
            libos::wait_one(k.handle(), libos::Signals::PEER_CLOSED, Some(0)).map_or(true, |s| !s.contains(libos::Signals::PEER_CLOSED))
        });
        let k = self.keys.pop()?;
        while k.try_read().is_ok() {}
        Some(k)
    }

    fn key_transport<'a>(&self, ch: &'a Channel) -> Result<HidTransport<ChannelHid<'a>>> {
        let mut nonce = [0u8; 8];
        rand_core::RngCore::fill_bytes(&mut OsRng, &mut nonce);
        HidTransport::open(ChannelHid(ch), nonce).map_err(|_| Error::IoError)
    }

    /// Overi a pri uspechu zalozi relaci u initu.
    fn authenticate(&mut self, uid: u64, method: Method, secret: &Secret) -> Result<(Outcome, Option<Secret>)> {
        let now = now();
        let outcome = if method == Method::Key {
            let ch = self.key();
            let r = match &ch {
                Some(c) => {
                    let mut nonce = [0u8; 8];
                    rand_core::RngCore::fill_bytes(&mut OsRng, &mut nonce);
                    match HidTransport::open(ChannelHid(c), nonce) {
                        Ok(mut t) => self.eng.authenticate(uid, method, secret, Some(&mut t as &mut dyn Transport), now),
                        Err(_) => Err(Error::IoError),
                    }
                }
                None => Err(Error::NotFound),
            };
            self.keys.extend(ch);
            r?
        } else {
            self.eng.authenticate(uid, method, secret, None, now)?
        };
        let name = self.eng.account(uid).map_or(String::from("?"), |a| a.name.clone());
        match &outcome {
            Outcome::Ok { home_key, .. } => {
                println!("accountd: {name}: prihlaseni ({}) ok", method.name());
                let k = home_key.clone();
                Ok((outcome, k))
            }
            Outcome::Denied { .. } => {
                println!("accountd: {name}: prihlaseni ({}) odmitnuto", method.name());
                Ok((outcome, None))
            }
            Outcome::LockedOut { .. } => Ok((outcome, None)),
        }
    }

    /// Otevre relaci u initu (domov odemkne klicem, je-li sifrovany).
    pub fn open_session(&mut self, uid: u64, home_key: Option<Secret>) -> Result<u64> {
        let a = self.eng.account(uid).ok_or(Error::NotFound)?;
        let (groups, home, encrypted) = (a.groups.clone(), a.home.clone(), a.crypt.is_some());
        if encrypted {
            match home_key {
                Some(key) => {
                    self.to_init(&ToInit::UnlockHome { uid, key }, Vec::new())?;
                }
                // Metoda bez obalu: jen kdyz uz je domov odemceny jinou
                // relaci tehoz uctu.
                None if self.sessions.iter().any(|s| s.uid == uid) => {}
                None => return Err(Error::Locked),
            }
        }
        let (ours, theirs) = Channel::create()?;
        let body = self.to_init(&ToInit::OpenSession { uid, groups, home }, vec![theirs.into_handle()])?;
        let id = wire::Reader::new(&body).u64()?;
        self.sessions.push(Session { id, uid, chan: ours, locked: false, closed: false });
        Ok(id)
    }

    /// Zalozi domov u initu a pak ulozi ucet.
    pub fn create_account(&mut self, a: accounts::Account) -> Result<u64> {
        self.to_init(&ToInit::CreateHome { uid: a.uid, name: a.name.clone() }, Vec::new())?;
        let name = a.name.clone();
        let uid = self.eng.commit(a)?;
        println!("accountd: zalozen ucet {name} ({uid})");
        Ok(uid)
    }

    /// Kanal login.
    pub fn login(&mut self, req: Request) -> Vec<u8> {
        let r = match req {
            Request::List => Ok(encode_list(&self.eng.list())),
            Request::Phrase { uid } => match self.eng.account(uid) {
                Some(a) if a.flags & flags::DEV == 0 || self.eng.cfg.dev => {
                    let mut w = Writer::new();
                    w.str(&a.phrase);
                    Ok(w.finish())
                }
                _ => Err(Error::NotFound),
            },
            Request::Auth { uid, method, secret } => match self.authenticate(uid, method, &secret) {
                Ok((Outcome::Ok { .. }, key)) => self.open_session(uid, key).map(|s| AuthReply::Ok { session: s }.encode()),
                Ok((o, _)) => Ok(o.reply(0).encode()),
                Err(e) => Err(e),
            },
            Request::Unlock { session, method, secret } => self.unlock(session, method, &secret),
            Request::LockSession { session } => self.lock(session),
            _ => Err(Error::AccessDenied),
        };
        reply(r)
    }

    /// Zamknout relaci z obrazovky (bezpecna zkratka). Zamknout smi
    /// kanal login kazdou relaci; odemknout jen overeni stejneho uctu.
    fn lock(&mut self, session: u64) -> Result<Vec<u8>> {
        let i = self.sessions.iter().position(|s| s.id == session).ok_or(Error::NotFound)?;
        self.to_init(&ToInit::LockSession { session }, Vec::new())?;
        self.sessions[i].locked = true;
        Ok(Vec::new())
    }

    fn unlock(&mut self, session: u64, method: Method, secret: &Secret) -> Result<Vec<u8>> {
        let uid = self.sessions.iter().find(|s| s.id == session).ok_or(Error::NotFound)?.uid;
        match self.authenticate(uid, method, secret)? {
            (Outcome::Ok { .. }, _) => {
                self.to_init(&ToInit::UnlockSession { session }, Vec::new())?;
                if let Some(s) = self.sessions.iter_mut().find(|s| s.id == session) {
                    s.locked = false;
                }
                Ok(AuthReply::Ok { session }.encode())
            }
            (o, _) => Ok(o.reply(session).encode()),
        }
    }

    /// Kanal setup: zalozeni prvniho uctu.
    pub fn setup(&mut self, req: Request) -> Vec<u8> {
        let r = match req {
            Request::CreateFirst { name, display, password } => self
                .eng
                .prepare_first(&name, &display, &password, now())
                .and_then(|a| self.create_account(a))
                .map(|uid| {
                    let mut w = Writer::new();
                    w.u64(uid);
                    w.finish()
                }),
            Request::List => Ok(encode_list(&self.eng.list())),
            _ => Err(Error::AccessDenied),
        };
        reply(r)
    }

    /// Kanal self relace `i`. `None` = odpoved prijde pozdeji (ukon spravy
    /// ceka na dialog).
    pub fn own(&mut self, i: usize, req: Request) -> Option<Vec<u8>> {
        let (session, uid) = (self.sessions[i].id, self.sessions[i].uid);
        let now = now();
        let r: Result<Vec<u8>> = match req {
            // Ukon pro cely pocitac: duveryhodny dialog kompozitoru.
            Request::Elevate { action, params } => return self.elevate(i, action, params),
            Request::WhoAmI => self.whoami(session, uid),
            Request::SetPassword { old, new } => self.eng.set_password(uid, &old, &new, now).map(|_| Vec::new()),
            Request::SetPin { password, pin } => self.eng.set_pin(uid, &password, &pin, now).map(|_| Vec::new()),
            Request::SetPhrase { phrase } => self.eng.set_phrase(uid, &phrase).map(|_| Vec::new()),
            Request::EnrollKey { password, key_pin, name } => {
                let ch = self.key();
                let r = match &ch {
                    Some(c) => {
                        let mut nonce = [0u8; 8];
                        rand_core::RngCore::fill_bytes(&mut OsRng, &mut nonce);
                        match HidTransport::open(ChannelHid(c), nonce) {
                            Ok(mut t) => self.eng.enroll_key(uid, &password, &key_pin, &name, &mut t, now),
                            Err(_) => Err(Error::IoError),
                        }
                    }
                    None => Err(Error::NotFound),
                };
                self.keys.extend(ch);
                r.map(|id| id.to_le_bytes().to_vec())
            }
            Request::RemoveKey { password, id } => self.eng.remove_key(uid, &password, id, now).map(|_| Vec::new()),
            Request::SetKeyPin { pin } => self.set_key_pin(&pin),
            Request::EncryptHome { password } => self.encrypt_home(uid, &password, now),
            Request::Lock => self.to_init(&ToInit::LockSession { session }, Vec::new()).map(|_| {
                self.sessions[i].locked = true;
                Vec::new()
            }),
            Request::Logout => self.to_init(&ToInit::CloseSession { session }, Vec::new()).map(|_| {
                self.sessions[i].closed = true;
                Vec::new()
            }),
            _ => Err(Error::AccessDenied),
        };
        Some(reply(r))
    }

    fn whoami(&self, session: u64, uid: u64) -> Result<Vec<u8>> {
        let a = self.eng.account(uid).ok_or(Error::NotFound)?;
        let weak = !accounts::DeviceKey::hardware(&self.eng.device);
        Ok(WhoAmI {
            summary: a.summary(weak),
            groups: a.groups.clone(),
            home: a.home.clone(),
            phrase: a.phrase.clone(),
            session,
            keys: a.keys.iter().map(|k| (k.id, k.name.clone(), k.uv)).collect(),
        }
        .encode())
    }

    fn set_key_pin(&mut self, pin: &Secret) -> Result<Vec<u8>> {
        let ch = self.key().ok_or(Error::NotFound)?;
        let r = (|| {
            let t = self.key_transport(&ch)?;
            let mut c = Client::new(t).map_err(|_| Error::IoError)?;
            if c.pin_set() {
                return Err(Error::AlreadyExists);
            }
            c.set_pin(&mut OsRng, pin.expose()).map_err(|_| Error::InvalidArgs)
        })();
        self.keys.push(ch);
        r.map(|_| Vec::new())
    }

    /// Sifrovani domova: nejdriv FS (pres init) prijme klic, pak se
    /// obaly ulozi. Dokud FS sifrovani neumi, init vrati `NOT_SUPPORTED`
    /// a nic se neulozi.
    fn encrypt_home(&mut self, uid: u64, password: &Secret, now: i64) -> Result<Vec<u8>> {
        let p = self.eng.prepare_crypt(uid, password, now)?;
        self.to_init(&ToInit::UnlockHome { uid, key: p.home_key.clone() }, Vec::new())?;
        self.eng.commit_crypt(uid, p.crypt)?;
        println!("accountd: {uid}: domov sifrovany");
        Ok(p.recovery.expose().to_vec())
    }

    /// Udalost initu (relace skoncila).
    pub fn init_event(&mut self, data: &[u8]) {
        if let Ok(session) = accountproto::decode_session_closed(data) {
            self.sessions.retain(|s| s.id != session);
        }
    }
}

fn reply(r: Result<Vec<u8>>) -> Vec<u8> {
    match r {
        Ok(b) => reply_ok(&b),
        Err(e) => reply_err(e),
    }
}
