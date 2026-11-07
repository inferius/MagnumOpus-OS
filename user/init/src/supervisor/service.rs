//! Specifikace sluzby v registru supervisoru: adapter ([`Service`]),
//! zavislosti ([`Dep`]) a drzak koncu kanalu k ovladacum ([`Kept`]).
//!
//! Protokol je popsany v `docs/cs/obnova-sluzeb.md`.

use alloc::boxed::Box;
use alloc::vec::Vec;

use libos::{Channel, Error, Handle, Result, Startup};

/// S cim se nova instance spousti.
pub struct StartCtx<'a> {
    pub startup: &'a Startup,
    /// Generace nove instance (prvni start = 1).
    pub generation: u64,
    /// `true` = restart po padu (ne start systemu). Sluzba, ktera se po
    /// restartu chova jinak (fsd setri sirotky), se to dozvi ve sve
    /// startovni zprave.
    #[allow(dead_code)] // cte adapter fsd (setreni sirotku)
    pub restart: bool,
}

/// Nove spustena instance.
pub struct Started {
    /// Proces (supervisor ceka na jeho konec).
    pub process: Handle,
    /// Kanal, na kterem sluzba potvrdi start (`None` = bezi hned).
    pub ready: Option<Handle>,
}

/// Adapter sluzby v initu: jak ji spustit a jak na ni vytvorit relaci
/// podle receptu. Metody nesmi blokovat (supervisor je jednovlaknovy);
/// cekani patri do [`Service::ready`].
pub trait Service: Send {
    /// Spusti novou instanci (neblokuje).
    fn start(&mut self, ctx: &StartCtx) -> Result<Started>;

    /// Kanal potvrzeni je citelny nebo zavreny: `Ok(true)` = instance bezi,
    /// `Ok(false)` = jeste ne (cekat dal), chyba = ukoncit a pocitat jako
    /// pad.
    fn ready(&mut self) -> Result<bool> {
        Ok(true)
    }

    /// Relace podle receptu na bezici instanci.
    fn connect(&mut self, _recipe: &[u8]) -> Result<Channel> {
        Err(Error::NotSupported)
    }

    /// Zuzeny recept. Nikdy nesmi dat vic prav nez `parent`; zuzeni, ktere
    /// by prava pridalo, = `ACCESS_DENIED`.
    fn narrow(&self, _parent: &[u8], _narrowing: &[u8]) -> Result<Vec<u8>> {
        Err(Error::NotSupported)
    }

    /// Zavislost s [`DepMode::Notify`] bezi v nove generaci.
    fn dependency_restarted(&mut self, _dep: &str, _generation: u64) {}

    /// Instance skoncila: zahodit kanaly k ni.
    fn stopped(&mut self) {}
}

/// Co se stane se sluzbou, kdyz spadne jeji zavislost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepMode {
    /// Ukoncit a spustit znovu az po novem startu zavislosti (sluzba drzi
    /// kanal primo k instanci zavislosti, bez listku).
    Restart,
    /// Bezet dal; po novem startu zavislosti zavolat
    /// [`Service::dependency_restarted`].
    Notify,
}

/// Zavislost na jine sluzbe registru.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dep {
    pub name: &'static str,
    pub mode: DepMode,
}

impl Dep {
    pub const fn restart(name: &'static str) -> Self {
        Self {
            name,
            mode: DepMode::Restart,
        }
    }

    pub const fn notify(name: &'static str) -> Self {
        Self {
            name,
            mode: DepMode::Notify,
        }
    }
}

/// Sluzba v registru.
pub struct Spec {
    pub name: &'static str,
    pub deps: &'static [Dep],
    /// `None` = jen hlidat (pad se zaloguje, restart neni).
    pub service: Option<Box<dyn Service>>,
}

/// Jednoduchy restart pres funkce (sluzby bez relaci, napr. `timed`).
#[derive(Clone, Copy)]
pub struct Restart {
    /// Spusti novy proces (neceka na nej).
    pub start: fn(&Startup) -> Result<Started>,
    /// Kanal z [`Started::ready`] je citelny nebo zavreny: dokonci start
    /// (precte potvrzeni). Chyba = sluzba se ukonci a pocita se jako pad.
    pub ready: Option<fn() -> Result<()>>,
}

impl Service for Restart {
    fn start(&mut self, ctx: &StartCtx) -> Result<Started> {
        (self.start)(ctx.startup)
    }

    fn ready(&mut self) -> Result<bool> {
        self.ready.map_or(Ok(()), |f| f()).map(|()| true)
    }
}

/// Drzak konce kanalu k ovladaci: supervisor si drzi duplikat konce, ktery
/// predal sluzbe. Kdyz sluzba spadne, konec u ovladace zustane otevreny
/// (ovladac nic nepozna) a nova instance dostane dalsi duplikat. Nova
/// instance musi nejdriv zahodit stare zpravy (synchronizace podle
/// protokolu ovladace).
#[allow(dead_code)] // pouziji adaptery fsd, inputd, netd, compd
pub struct Kept(Channel);

#[allow(dead_code)]
impl Kept {
    pub fn new(end: Channel) -> Self {
        Self(end)
    }

    /// Duplikat konce pro (novou) instanci sluzby.
    pub fn give(&self) -> Result<Channel> {
        Ok(Channel::from_handle(self.0.handle().duplicate(None)?))
    }
}
