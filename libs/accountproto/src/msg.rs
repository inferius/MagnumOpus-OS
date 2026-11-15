//! Zpravy kanalu login, self, setup a init.

use alloc::string::String;
use alloc::vec::Vec;

use wire::{Reader, Writer};

use crate::{Error, Method, Result, Secret, get_blob, get_secret, get_u64s, op, put_blob, put_secret, put_u64s};

/// Pozadavek klienta na accountd (kanaly login, self, setup).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    List,
    Phrase { uid: u64 },
    Auth { uid: u64, method: Method, secret: Secret },
    Unlock { session: u64, method: Method, secret: Secret },
    LockSession { session: u64 },
    WhoAmI,
    SetPassword { old: Secret, new: Secret },
    /// Prazdny `pin` = PIN smazat.
    SetPin { password: Secret, pin: Secret },
    /// Registrace bezpecnostniho klice (`key_pin` = PIN klice, prazdny =
    /// klic bez PINu).
    EnrollKey { password: Secret, key_pin: Secret, name: String },
    RemoveKey { password: Secret, id: u32 },
    SetPhrase { phrase: String },
    EncryptHome { password: Secret },
    Lock,
    Logout,
    /// Ukon spravy pro cely pocitac (viz "Sprava pocitace").
    Elevate { action: u32, params: Vec<u8> },
    /// Nastavi PIN bezpecnostniho klice, ktery PIN jeste nema.
    SetKeyPin { pin: Secret },
    CreateFirst { name: String, display: String, password: Secret },
    /// Vlastni ulozena povoleni aplikaci (`perm::encode_entries`).
    PermList,
    /// Odvolat vlastni rozhodnuti (aplikace se priste zepta znovu).
    PermRevoke { app_id: String, perm: crate::perm::Perm },
    /// Kanal names: prihlasovaci a zobrazovana jmena uctu
    /// ([`encode_names`]). Nic dalsiho o uctu (metody, priznaky) - stejna
    /// jmena ukazuje prihlasovaci obrazovka.
    Names,
    /// Rezim vyvojare (jen vyvojove sestaveni, feature `dev-mode`): `set`
    /// [`DEV_QUERY`] = jen stav, [`DEV_OFF`] / [`DEV_ON`] = prepnout.
    /// Odpoved: 1 B, 1 = zapnuto. Sestaveni bez feature operaci nezna
    /// (`NOT_SUPPORTED`).
    #[cfg(feature = "dev-mode")]
    DevMode { set: u8 },
}

/// Hodnoty `Request::DevMode::set` (cislo operace [`op::DEV_MODE`] je
/// rezervovane i ve vydani).
pub const DEV_QUERY: u8 = 0;
pub const DEV_OFF: u8 = 1;
pub const DEV_ON: u8 = 2;

impl Request {
    pub fn op(&self) -> u32 {
        match self {
            Request::List => op::LIST,
            Request::Phrase { .. } => op::PHRASE,
            Request::Auth { .. } => op::AUTH,
            Request::Unlock { .. } => op::UNLOCK,
            Request::LockSession { .. } => op::LOGIN_LOCK,
            Request::WhoAmI => op::WHOAMI,
            Request::SetPassword { .. } => op::SET_PASSWORD,
            Request::SetPin { .. } => op::SET_PIN,
            Request::EnrollKey { .. } => op::ENROLL_KEY,
            Request::RemoveKey { .. } => op::REMOVE_KEY,
            Request::SetPhrase { .. } => op::SET_PHRASE,
            Request::EncryptHome { .. } => op::ENCRYPT_HOME,
            Request::Lock => op::LOCK,
            Request::Logout => op::LOGOUT,
            Request::Elevate { .. } => op::ELEVATE,
            Request::SetKeyPin { .. } => op::SET_KEY_PIN,
            Request::CreateFirst { .. } => op::CREATE_FIRST,
            Request::PermList => op::PERM_LIST,
            Request::PermRevoke { .. } => op::PERM_REVOKE,
            Request::Names => op::NAMES,
            #[cfg(feature = "dev-mode")]
            Request::DevMode { .. } => op::DEV_MODE,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::op(self.op());
        match self {
            Request::List | Request::WhoAmI | Request::Lock | Request::Logout | Request::PermList | Request::Names => {}
            Request::PermRevoke { app_id, perm } => {
                w.str(app_id).u8(*perm as u8);
            }
            Request::Phrase { uid } => {
                w.u64(*uid);
            }
            Request::LockSession { session } => {
                w.u64(*session);
            }
            Request::Auth { uid, method, secret } => {
                w.u64(*uid).u8(method.to_u8());
                put_secret(&mut w, secret);
            }
            Request::Unlock { session, method, secret } => {
                w.u64(*session).u8(method.to_u8());
                put_secret(&mut w, secret);
            }
            Request::SetPassword { old, new } => {
                put_secret(&mut w, old);
                put_secret(&mut w, new);
            }
            Request::SetPin { password, pin } => {
                put_secret(&mut w, password);
                put_secret(&mut w, pin);
            }
            Request::EnrollKey { password, key_pin, name } => {
                put_secret(&mut w, password);
                put_secret(&mut w, key_pin);
                w.str(name);
            }
            Request::RemoveKey { password, id } => {
                put_secret(&mut w, password);
                w.u32(*id);
            }
            Request::SetPhrase { phrase } => {
                w.str(phrase);
            }
            Request::EncryptHome { password } => put_secret(&mut w, password),
            Request::Elevate { action, params } => {
                w.u32(*action);
                put_blob(&mut w, params);
            }
            Request::SetKeyPin { pin } => put_secret(&mut w, pin),
            #[cfg(feature = "dev-mode")]
            Request::DevMode { set } => {
                w.u8(*set);
            }
            Request::CreateFirst { name, display, password } => {
                w.str(name).str(display);
                put_secret(&mut w, password);
            }
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let req = match r.u32()? {
            op::LIST => Request::List,
            op::PHRASE => Request::Phrase { uid: r.u64()? },
            op::AUTH => Request::Auth {
                uid: r.u64()?,
                method: Method::from_u8(r.u8()?)?,
                secret: get_secret(&mut r)?,
            },
            op::UNLOCK => Request::Unlock {
                session: r.u64()?,
                method: Method::from_u8(r.u8()?)?,
                secret: get_secret(&mut r)?,
            },
            op::LOGIN_LOCK => Request::LockSession { session: r.u64()? },
            op::WHOAMI => Request::WhoAmI,
            op::SET_PASSWORD => Request::SetPassword { old: get_secret(&mut r)?, new: get_secret(&mut r)? },
            op::SET_PIN => Request::SetPin { password: get_secret(&mut r)?, pin: get_secret(&mut r)? },
            op::ENROLL_KEY => Request::EnrollKey {
                password: get_secret(&mut r)?,
                key_pin: get_secret(&mut r)?,
                name: r.str()?,
            },
            op::REMOVE_KEY => Request::RemoveKey { password: get_secret(&mut r)?, id: r.u32()? },
            op::SET_PHRASE => Request::SetPhrase { phrase: r.str()? },
            op::ENCRYPT_HOME => Request::EncryptHome { password: get_secret(&mut r)? },
            op::LOCK => Request::Lock,
            op::LOGOUT => Request::Logout,
            op::PERM_LIST => Request::PermList,
            op::NAMES => Request::Names,
            op::PERM_REVOKE => Request::PermRevoke { app_id: r.str()?, perm: crate::perm::Perm::from_u8(r.u8()?)? },
            op::ELEVATE => Request::Elevate { action: r.u32()?, params: get_blob(&mut r)? },
            op::SET_KEY_PIN => Request::SetKeyPin { pin: get_secret(&mut r)? },
            #[cfg(feature = "dev-mode")]
            op::DEV_MODE => match r.u8()? {
                set @ (DEV_QUERY | DEV_OFF | DEV_ON) => Request::DevMode { set },
                _ => return Err(Error::InvalidArgs),
            },
            op::CREATE_FIRST => Request::CreateFirst {
                name: r.str()?,
                display: r.str()?,
                password: get_secret(&mut r)?,
            },
            _ => return Err(Error::NotSupported),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(req)
    }

    /// Patri operace na kanal login?
    pub fn is_login(&self) -> bool {
        matches!(
            self,
            Request::List | Request::Phrase { .. } | Request::Auth { .. } | Request::Unlock { .. } | Request::LockSession { .. }
        )
    }

    /// Patri operace na kanal self?
    pub fn is_self(&self) -> bool {
        (op::WHOAMI..=op::DEV_MODE).contains(&self.op())
    }
}

/// Ucet ve vypisu pro prihlasovaci obrazovku (bez tajemstvi a fraze).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSummary {
    pub uid: u64,
    pub name: String,
    pub display: String,
    /// Bity [`Method::bit`].
    pub methods: u8,
    pub flags: u32,
}

impl AccountSummary {
    pub fn put(&self, w: &mut Writer) {
        w.u64(self.uid).str(&self.name).str(&self.display).u8(self.methods).u32(self.flags);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        Ok(AccountSummary {
            uid: r.u64()?,
            name: r.str()?,
            display: r.str()?,
            methods: r.u8()?,
            flags: r.u32()?,
        })
    }

    pub fn has(&self, m: Method) -> bool {
        self.methods & m.bit() != 0
    }
}

pub fn encode_list(list: &[AccountSummary]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(list.len() as u32);
    for a in list {
        a.put(&mut w);
    }
    w.finish()
}

pub fn decode_list(b: &[u8]) -> Result<Vec<AccountSummary>> {
    let mut r = Reader::new(b);
    let n = r.u32()? as usize;
    if n > 10_000 {
        return Err(Error::InvalidArgs);
    }
    (0..n).map(|_| AccountSummary::get(&mut r)).collect()
}

/// Jmena uctu (odpoved `NAMES`): stejna, jaka ukazuje prihlasovaci
/// obrazovka.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountName {
    pub uid: u64,
    /// Prihlasovaci jmeno (zadani v shellu, `acl add bob ...`).
    pub name: String,
    /// Zobrazovane jmeno (prazdne -> `name`).
    pub display: String,
}

pub fn encode_names(list: &[AccountName]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(list.len() as u32);
    for a in list {
        w.u64(a.uid).str(&a.name).str(&a.display);
    }
    w.finish()
}

pub fn decode_names(b: &[u8]) -> Result<Vec<AccountName>> {
    let mut r = Reader::new(b);
    let n = r.u32()? as usize;
    if n > 10_000 {
        return Err(Error::InvalidArgs);
    }
    (0..n).map(|_| Ok(AccountName { uid: r.u64()?, name: r.str()?, display: r.str()? })).collect()
}

/// Vysledek overeni.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthReply {
    /// Prihlaseno: cislo relace (u `UNLOCK` cislo odemcene relace).
    Ok { session: u64 },
    /// Spatne: dalsi pokus nejdriv za `retry_after_ms` (0 = hned),
    /// `pin_left` = zbyvajici pokusy PINu (u jine metody 0).
    Denied { retry_after_ms: u64, pin_left: u8 },
    /// Pokus v dobe zpomaleni: nic se nepocitalo.
    LockedOut { retry_after_ms: u64 },
}

impl AuthReply {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            AuthReply::Ok { session } => {
                w.u8(0).u64(*session);
            }
            AuthReply::Denied { retry_after_ms, pin_left } => {
                w.u8(1).u64(*retry_after_ms).u8(*pin_left);
            }
            AuthReply::LockedOut { retry_after_ms } => {
                w.u8(2).u64(*retry_after_ms);
            }
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        Ok(match r.u8()? {
            0 => AuthReply::Ok { session: r.u64()? },
            1 => AuthReply::Denied { retry_after_ms: r.u64()?, pin_left: r.u8()? },
            2 => AuthReply::LockedOut { retry_after_ms: r.u64()? },
            _ => return Err(Error::InvalidArgs),
        })
    }
}

/// Odpoved `WHOAMI`: vlastni ucet bez tajemstvi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhoAmI {
    pub summary: AccountSummary,
    pub groups: Vec<u64>,
    pub home: String,
    pub phrase: String,
    pub session: u64,
    /// Registrovane klice: id, nazev, ma PIN klice.
    pub keys: Vec<(u32, String, bool)>,
}

impl WhoAmI {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        self.summary.put(&mut w);
        put_u64s(&mut w, &self.groups);
        w.str(&self.home).str(&self.phrase).u64(self.session).u32(self.keys.len() as u32);
        for (id, name, uv) in &self.keys {
            w.u32(*id).str(name).u8(*uv as u8);
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let summary = AccountSummary::get(&mut r)?;
        let groups = get_u64s(&mut r)?;
        let home = r.str()?;
        let phrase = r.str()?;
        let session = r.u64()?;
        let n = r.u32()? as usize;
        if n > 64 {
            return Err(Error::InvalidArgs);
        }
        let keys = (0..n)
            .map(|_| Ok((r.u32()?, r.str()?, r.u8()? != 0)))
            .collect::<Result<Vec<_>>>()?;
        Ok(WhoAmI { summary, groups, home, phrase, session, keys })
    }
}

/// Zprava accountd -> init (kanal init). U `OpenSession` je pripojeny
/// handle: kanal self teto relace (init ho preda nastaveni v relaci).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToInit {
    OpenSession { uid: u64, groups: Vec<u64>, home: String },
    CloseSession { session: u64 },
    LockSession { session: u64 },
    UnlockSession { session: u64 },
    CreateHome { uid: u64, name: String },
    /// Presune domov do kose spravy (`/home/.smazane/<uid>`).
    DeleteHome { uid: u64, name: String },
    ListSessions,
    /// Klic domova pro fsd (`LOAD_KEY`); az FS sifrovani umi.
    UnlockHome { uid: u64, key: Secret },
}

impl ToInit {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            ToInit::OpenSession { uid, groups, home } => {
                w = Writer::op(op::OPEN_SESSION);
                w.u64(*uid);
                put_u64s(&mut w, groups);
                w.str(home);
            }
            ToInit::CloseSession { session } => {
                w = Writer::op(op::CLOSE_SESSION);
                w.u64(*session);
            }
            ToInit::LockSession { session } => {
                w = Writer::op(op::LOCK_SESSION);
                w.u64(*session);
            }
            ToInit::UnlockSession { session } => {
                w = Writer::op(op::UNLOCK_SESSION);
                w.u64(*session);
            }
            ToInit::CreateHome { uid, name } => {
                w = Writer::op(op::CREATE_HOME);
                w.u64(*uid).str(name);
            }
            ToInit::DeleteHome { uid, name } => {
                w = Writer::op(op::DELETE_HOME);
                w.u64(*uid).str(name);
            }
            ToInit::ListSessions => w = Writer::op(op::LIST_SESSIONS),
            ToInit::UnlockHome { uid, key } => {
                w = Writer::op(op::UNLOCK_HOME);
                w.u64(*uid);
                put_secret(&mut w, key);
            }
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let m = match r.u32()? {
            op::OPEN_SESSION => ToInit::OpenSession { uid: r.u64()?, groups: get_u64s(&mut r)?, home: r.str()? },
            op::CLOSE_SESSION => ToInit::CloseSession { session: r.u64()? },
            op::LOCK_SESSION => ToInit::LockSession { session: r.u64()? },
            op::UNLOCK_SESSION => ToInit::UnlockSession { session: r.u64()? },
            op::CREATE_HOME => ToInit::CreateHome { uid: r.u64()?, name: r.str()? },
            op::DELETE_HOME => ToInit::DeleteHome { uid: r.u64()?, name: r.str()? },
            op::LIST_SESSIONS => ToInit::ListSessions,
            op::UNLOCK_HOME => ToInit::UnlockHome { uid: r.u64()?, key: get_secret(&mut r)? },
            _ => return Err(Error::NotSupported),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(m)
    }
}

/// Relace ve vypisu `LIST_SESSIONS`: cislo, uid, zamcena.
pub fn encode_sessions(list: &[(u64, u64, bool)]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(list.len() as u32);
    for (s, u, l) in list {
        w.u64(*s).u64(*u).u8(*l as u8);
    }
    w.finish()
}

pub fn decode_sessions(b: &[u8]) -> Result<Vec<(u64, u64, bool)>> {
    let mut r = Reader::new(b);
    let n = r.u32()? as usize;
    if n > 1024 {
        return Err(Error::InvalidArgs);
    }
    (0..n).map(|_| Ok((r.u64()?, r.u64()?, r.u8()? != 0))).collect()
}

/// Udalost init -> accountd: relace skoncila (odhlaseni, konec procesu).
pub fn encode_session_closed(session: u64) -> Vec<u8> {
    let mut w = Writer::op(op::SESSION_CLOSED);
    w.u64(session);
    w.finish()
}

pub fn decode_session_closed(b: &[u8]) -> Result<u64> {
    let mut r = Reader::new(b);
    if r.u32()? != op::SESSION_CLOSED {
        return Err(Error::InvalidArgs);
    }
    r.u64()
}
