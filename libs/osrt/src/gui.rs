//! Relace okna zdedena od rodice (role `Gui`, `docs/cs/gui.md`, kapitola
//! Relace okna pro potomky).
//!
//! Proces spusteny s roli `Gui` dostane relaci okna u kompozitoru
//! (`abi::params::tag::GUI`) a listek k ni (`GUI_LEASE`). Runtime:
//!
//! - nastavi `MO_GUI=1` do prostredi (jako `WAYLAND_DISPLAY`),
//! - relaci preda knihovne okna ([`take_session`], `libs/mowin`), listek si
//!   necha: obnova po padu kompozitoru ([`reconnect`]) a relace pro dalsi
//!   potomky ([`child`]),
//! - potomek dostane relaci jen, kdyz jeho prostredi ma `MO_GUI` neprazdne
//!   ([`wanted`]); odebrani promenne = potomek bez GUI.
//!
//! Relace potomka vznika `NARROW` s prazdnym zuzenim: supervisor vyda
//! novou relaci se **stejnou** identitou aplikace (nikdy sirsi) a novy
//! listek svazany s rodicovskym (odvolani rodice odvola i potomky).

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use core::time::Duration;

use leaseproto::{Reply, Request};
use libos::sync::Mutex;
use libos::{Channel, Error, Result, time};

/// Kanal relace okna (pro programy, ktere libos primo nepouzivaji).
pub type Session = Channel;

/// Promenna prostredi: neprazdna = potomek dostane relaci okna.
pub const ENV: &[u8] = b"MO_GUI";

/// Relace okna od rodice, dokud si ji nevezme knihovna okna.
static SESSION: Mutex<Option<Channel>> = Mutex::new(None);
/// Listek k relaci (zustava runtime po celou dobu behu).
static LEASE: Mutex<Option<Channel>> = Mutex::new(None);
static HAS_LEASE: AtomicBool = AtomicBool::new(false);
static NEXT_ID: AtomicU32 = AtomicU32::new(1);

/// Odpoved na `NARROW` (relace u bezici instance) chodi hned; `RECONNECT`
/// po padu muze cekat na novou instanci.
const NARROW_TIMEOUT: Duration = Duration::from_secs(5);

/// Pri startu: relace a listek ze startovni zpravy (jen oba spolu).
pub(crate) fn init(session: Option<Channel>, lease: Option<Channel>) {
    let (Some(session), Some(lease)) = (session, lease) else { return };
    *SESSION.lock() = Some(session);
    *LEASE.lock() = Some(lease);
    HAS_LEASE.store(true, Ordering::Release);
    if crate::env::var(ENV).is_none_or(|v| v.is_empty()) {
        let _ = crate::env::set_var(ENV, b"1");
    }
}

/// Vezme zdedenou relaci okna (jen jednou; volajici ji vlastni).
pub fn take_session() -> Option<Channel> {
    SESSION.lock().take()
}

/// Jako [`take_session`], syrove cislo handlu (pro knihovnu s vlastni
/// kopii libos, `libs/mowin`; volajici handle vlastni a zavira).
pub fn take_session_raw() -> Option<u32> {
    take_session().map(|c| c.into_handle().into_raw())
}

/// Ma proces listek k relaci okna (spusten s roli `Gui`)?
pub fn has_lease() -> bool {
    HAS_LEASE.load(Ordering::Acquire)
}

/// Ma potomek s prostredim `env` (`JMENO=HODNOTA`) dostat relaci okna?
pub fn wanted(env: &[Vec<u8>]) -> bool {
    has_lease()
        && env.iter().any(|e| e.len() > ENV.len() + 1 && e.starts_with(ENV) && e[ENV.len()] == b'=')
}

/// Pozadavek na listku a odpoved (handly odpovedi pri uspechu).
fn call(req: Request, timeout: Duration) -> Result<Vec<libos::Handle>> {
    let lease = LEASE.lock();
    let lease = lease.as_ref().ok_or(Error::NotSupported)?;
    let id = req.id();
    let op = req.op();
    lease.write(&req.encode(), Vec::new())?;
    let end = time::deadline_after(timeout);
    loop {
        let m = lease.read(Some(end))?;
        let r = Reply::decode(&m.data)?;
        // Pozdni odpoved na drivejsi pozadavek (vyprsel cas) se zahodi.
        if r.id != id || r.op != op {
            continue;
        }
        r.status?;
        if m.handles.len() != Reply::handles_for(op) {
            return Err(Error::Corrupted);
        }
        return Ok(m.handles);
    }
}

/// Relace a listek pro potomka: `NARROW` s prazdnym zuzenim (stejna
/// identita, limit zijicich a rychlosti jako `ADOPT`).
pub fn child() -> Result<(Channel, Channel)> {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let mut h = call(Request::Narrow { id, narrowing: Vec::new() }, NARROW_TIMEOUT)?;
    let lease = Channel::from_handle(h.pop().ok_or(Error::Corrupted)?);
    let session = Channel::from_handle(h.pop().ok_or(Error::Corrupted)?);
    Ok((session, lease))
}

/// Nova relace po padu kompozitoru (`RECONNECT`, ceka nejvys `wait`).
/// Odvolane GUI = `PEER_CLOSED` nebo `SERVICE_UNAVAILABLE`.
pub fn reconnect(wait: Duration) -> Result<Channel> {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let wait_ms = u32::try_from(wait.as_millis()).unwrap_or(u32::MAX);
    let mut h = call(Request::Reconnect { id, wait_ms }, wait + Duration::from_secs(5))?;
    Ok(Channel::from_handle(h.pop().ok_or(Error::Corrupted)?))
}
