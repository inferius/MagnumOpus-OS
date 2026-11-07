//! Sluzba souboroveho systemu (`fsd`) pod supervisorem: adapter obnovy po
//! padu (protokol `docs/cs/obnova-sluzeb.md`, recept v
//! `docs/cs/souborovy-system.md`, kapitola "Obnova po padu fsd").
//!
//! - **Start**: proces `fsd` dostane duplikat konce kanalu k ovladaci disku
//!   (konec drzi supervisor v [`Kept`], ovladac pad sluzby nepozna)
//!   a vlastni ridici kanal. Konfigurace na bootstrap: `start`, `debug`
//!   (testovaci beh) a po padu `restart <generace>` - nova instance pak
//!   nejdriv zahodi pozdni odpovedi disku a 30 s neuklidi sirotky. Start
//!   potvrdi zpravou `ok`.
//! - **Recept** relace ([`Recipe`]): ridici relace systemu, nebo uzivatel
//!   se skupinami a pripojenymi cestami (`CONNECT_AS` z ridici relace).
//! - **Zuzeni**: podmnozina pripojenych cest (kazda cesta musi lezet
//!   v nektere puvodni a zachovat jen pro cteni), stejny uzivatel.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use fsproto::perm::{Mount, mount_flags};
use fsproto::{FsClient, Reader, Writer};
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Process, Result};

use crate::supervisor::{Kept, Service, StartCtx, Started};

/// Recept relace `fsd`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recipe {
    /// Ridici relace systemu (init): vsechna prava, `CONNECT_AS`.
    System,
    /// Relace uzivatele (`CONNECT_AS`).
    User {
        user: u64,
        groups: Vec<u64>,
        mounts: Vec<Mount>,
    },
}

const TAG_SYSTEM: u8 = 0;
const TAG_USER: u8 = 1;
/// Nejvic skupin a pripojenych cest v receptu (ochrana pameti supervisoru).
const MAX_ITEMS: usize = 64;

impl Recipe {
    /// Kodovani (`libs/wire`): `u8 druh`; uzivatel: `u64 uzivatel`,
    /// `u32 pocet` + `u64` skupiny, `u32 pocet` + `Mount` (jmeno, cil,
    /// priznaky).
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Recipe::System => {
                w.u8(TAG_SYSTEM);
            }
            Recipe::User { user, groups, mounts } => {
                w.u8(TAG_USER).u64(*user).u32(groups.len() as u32);
                for g in groups {
                    w.u64(*g);
                }
                put_mounts(&mut w, mounts);
            }
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        match r.u8()? {
            TAG_SYSTEM => Ok(Recipe::System),
            TAG_USER => {
                let user = r.u64()?;
                let n = r.u32()? as usize;
                if n > MAX_ITEMS {
                    return Err(Error::InvalidArgs);
                }
                let groups = (0..n).map(|_| r.u64()).collect::<Result<_>>()?;
                let mounts = get_mounts(&mut r)?;
                Ok(Recipe::User { user, groups, mounts })
            }
            _ => Err(Error::InvalidArgs),
        }
    }
}

fn put_mounts(w: &mut Writer, mounts: &[Mount]) {
    w.u32(mounts.len() as u32);
    for m in mounts {
        m.put(w);
    }
}

fn get_mounts(r: &mut Reader) -> Result<Vec<Mount>> {
    let n = r.u32()? as usize;
    if n > MAX_ITEMS {
        return Err(Error::InvalidArgs);
    }
    (0..n).map(|_| Mount::get(r)).collect()
}

/// Zuzeni receptu pro dite: seznam pripojenych cest (kodovani jako cesty
/// v receptu: `u32 pocet` + `Mount`).
pub fn narrowing(mounts: &[Mount]) -> Vec<u8> {
    let mut w = Writer::new();
    put_mounts(&mut w, mounts);
    w.finish()
}

/// Lezi cesta `t` v `base` (stejna nebo pod ni)?
fn under(t: &str, base: &str) -> bool {
    let t = t.trim_end_matches('/');
    let b = base.trim_end_matches('/');
    b.is_empty() || t == b || t.strip_prefix(b).is_some_and(|r| r.starts_with('/'))
}

/// Zuzeny recept: stejny uzivatel a skupiny, kazda pripojena cesta lezi
/// v nektere puvodni (a puvodni jen pro cteni zustane jen pro cteni).
/// Cokoli jineho = `ACCESS_DENIED` (zuzeni nesmi prava pridat).
pub fn narrow_recipe(parent: &[u8], narrowing: &[u8]) -> Result<Vec<u8>> {
    let Recipe::User { user, groups, mounts } = Recipe::decode(parent)? else {
        // Z ridici relace se dite nedela (jina identita, ne podmnozina).
        return Err(Error::AccessDenied);
    };
    let wanted = get_mounts(&mut Reader::new(narrowing))?;
    let ok = wanted.iter().all(|m| {
        mounts.iter().any(|p| {
            under(&m.target, &p.target) && (!p.read_only() || m.flags & mount_flags::READ_ONLY != 0)
        })
    });
    if !ok {
        return Err(Error::AccessDenied);
    }
    Ok(Recipe::User { user, groups, mounts: wanted }.encode())
}

/// Adapter `fsd`.
pub struct Fsd {
    /// Konec kanalu k ovladaci disku (supervisor ho drzi pres pady).
    disk: Kept,
    debug: bool,
    process: Option<Process>,
    /// Bootstrap kanal instance (potvrzeni startu).
    control: Option<Channel>,
    /// Ridici relace instance (relace podle receptu).
    session: Option<FsClient>,
}

impl Fsd {
    /// Adapter pro uz bezici instanci (start systemu, generace 1).
    pub fn running(disk: Kept, debug: bool, control: Channel, session: FsClient) -> Self {
        Self {
            disk,
            debug,
            process: None,
            control: Some(control),
            session: Some(session),
        }
    }

    /// Spusti `fsd` nad koncem disku; vraci proces, bootstrap a ridici
    /// relaci. Konfigurace jde hned, potvrzeni (`ok`) cte volajici.
    pub fn spawn(startup: &libos::Startup, disk: Channel, debug: bool, restart: Option<u64>) -> Result<(Process, Channel, FsClient)> {
        let image = Initfs::open(startup)?.find("fsd").ok_or(Error::NotFound)?;
        let (client, server) = Channel::create()?;
        let handles = vec![StartHandle::user(disk.into_handle()), StartHandle::user(server.into_handle())];
        let (process, control) = libos::spawn("fsd", image, handles)?;
        let mut config = String::from("start");
        // Simulace vypadku proudu jen v testovacim behu.
        if debug {
            config.push_str(" debug");
        }
        if let Some(g) = restart {
            config.push_str(&alloc::format!(" restart {g}"));
        }
        if let Err(e) = control.write(config.as_bytes(), Vec::new()) {
            let _ = process.kill();
            return Err(e);
        }
        Ok((process, control, FsClient::new(client)))
    }
}

impl Service for Fsd {
    fn start(&mut self, ctx: &StartCtx) -> Result<Started> {
        let restart = ctx.restart.then_some(ctx.generation);
        let (process, control, session) = Self::spawn(ctx.startup, self.disk.give()?, self.debug, restart)?;
        let started = Started {
            process: process.handle().duplicate(None)?,
            ready: Some(control.handle().duplicate(None)?),
        };
        self.process = Some(process);
        self.control = Some(control);
        self.session = Some(session);
        Ok(started)
    }

    fn ready(&mut self) -> Result<bool> {
        let control = self.control.as_ref().ok_or(Error::BadState)?;
        match control.try_read() {
            Ok(m) if m.data == b"ok" => Ok(true),
            Ok(_) => Err(Error::IoError),
            Err(Error::ShouldWait) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn connect(&mut self, recipe: &[u8]) -> Result<Channel> {
        let session = self.session.as_ref().ok_or(Error::ServiceUnavailable)?;
        let client = match Recipe::decode(recipe)? {
            Recipe::System => session.connect_control()?,
            Recipe::User { user, groups, mounts } => session.connect_as(user, &groups, &mounts)?.1,
        };
        Ok(client.into_channel())
    }

    fn narrow(&self, parent: &[u8], narrowing: &[u8]) -> Result<Vec<u8>> {
        narrow_recipe(parent, narrowing)
    }

    fn stopped(&mut self) {
        self.process = None;
        self.control = None;
        self.session = None;
    }
}
