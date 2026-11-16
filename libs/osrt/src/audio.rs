//! Relace zvuku zdedena od rodice (role `Audio`, `docs/cs/zvuk.md`,
//! kapitola Relace zvuku pro potomky).
//!
//! Proces spusteny s roli `Audio` (nebo aplikace, ktere init dal relaci
//! zvuku) dostane relaci `audiod` s pravy aplikace
//! (`abi::params::tag::AUDIO`, `audioproto::session`) a listek k ni
//! (`AUDIO_LEASE`). Runtime:
//!
//! - nastavi `MO_AUDIO=1` do prostredi (jako `MO_GUI`),
//! - relaci i listek preda knihovne zvuku ([`take_client_raw`]; mosdl,
//!   moaudio v `libs/mowin`): obnovu po restartu audiod dela knihovna
//!   (`svcclient::Conn`), listek pak drzi ona; relace pro potomky jde pres
//!   jeji `NARROW` (hacek [`set_child_hook`]),
//! - potomek dostane relaci jen, kdyz jeho prostredi ma `MO_AUDIO`
//!   neprazdne ([`wanted`]); odebrani promenne = potomek bez zvuku.
//!
//! Relace potomka vznika `NARROW` s prazdnym zuzenim: stejna identita
//! a stejna pravidla nahravani (nikdy sirsi: nahravani i potom jen pres
//! dialog povoleni mikrofonu), listek svazany s rodicovskym (odvolani
//! rodice odvola i potomky, limit jako `ADOPT`).

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use core::time::Duration;

use alloc::vec::Vec;

use leaseproto::Request;
use libos::sync::Mutex;
use libos::{Channel, Error, Handle, Result};

/// Promenna prostredi: neprazdna = potomek dostane relaci zvuku.
pub const ENV: &[u8] = b"MO_AUDIO";

/// Relace a listek od rodice, dokud si je nevezme knihovna zvuku.
static SESSION: Mutex<Option<Channel>> = Mutex::new(None);
static LEASE: Mutex<Option<Channel>> = Mutex::new(None);
static HAS_LEASE: AtomicBool = AtomicBool::new(false);
static NEXT_ID: AtomicU32 = AtomicU32::new(1);
/// Hacek knihovny, ktera si vzala listek: relace a listek pro potomka
/// (`fn() -> Result<(relace, listek), kod>`; 0 = zadny).
static CHILD_HOOK: AtomicUsize = AtomicUsize::new(0);

/// Hacek knihovny zvuku: `NARROW` na jejim listku, syrova cisla handlu.
pub type ChildHook = fn() -> core::result::Result<(u32, u32), i32>;

const NARROW_TIMEOUT: Duration = Duration::from_secs(5);

/// Pri startu: relace a listek ze startovni zpravy. Relace bez listku
/// (testy) se preda knihovne, ale bez obnovy a bez relaci pro potomky.
pub(crate) fn init(session: Option<Channel>, lease: Option<Channel>) {
    let Some(session) = session else { return };
    *SESSION.lock() = Some(session);
    let Some(lease) = lease else { return };
    *LEASE.lock() = Some(lease);
    HAS_LEASE.store(true, Ordering::Release);
    if crate::env::var(ENV).is_none_or(|v| v.is_empty()) {
        let _ = crate::env::set_var(ENV, b"1");
    }
}

/// Vezme relaci a listek (je-li) pro knihovnu zvuku (jen jednou; volajici
/// handly vlastni). S listkem knihovna zaregistruje [`set_child_hook`],
/// jinak potomci relaci nedostanou.
pub fn take_client_raw() -> Option<(u32, Option<u32>)> {
    let s = SESSION.lock().take()?;
    let l = LEASE.lock().take();
    Some((s.into_handle().into_raw(), l.map(|l| l.into_handle().into_raw())))
}

/// Hacek pro relace potomku (knihovna, ktera si vzala listek).
pub fn set_child_hook(f: ChildHook) {
    CHILD_HOOK.store(f as usize, Ordering::Release);
}

/// Ma proces relaci zvuku od rodice nebo initu?
pub fn has_lease() -> bool {
    HAS_LEASE.load(Ordering::Acquire)
}

/// Ma potomek s prostredim `env` (`JMENO=HODNOTA`) dostat relaci zvuku?
pub fn wanted(env: &[Vec<u8>]) -> bool {
    has_lease() && env.iter().any(|e| e.len() > ENV.len() + 1 && e.starts_with(ENV) && e[ENV.len()] == b'=')
}

/// Relace a listek pro potomka: `NARROW` s prazdnym zuzenim na listku
/// procesu (dokud ho ma runtime), jinak pres knihovnu zvuku.
pub fn child() -> Result<(Channel, Channel)> {
    {
        let lease = LEASE.lock();
        if let Some(l) = lease.as_ref() {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let mut h = crate::gui::lease_call(l, Request::Narrow { id, narrowing: Vec::new() }, NARROW_TIMEOUT)?;
            let lease = Channel::from_handle(h.pop().ok_or(Error::Corrupted)?);
            let session = Channel::from_handle(h.pop().ok_or(Error::Corrupted)?);
            return Ok((session, lease));
        }
    }
    let hook = CHILD_HOOK.load(Ordering::Acquire);
    if hook == 0 {
        return Err(Error::NotSupported);
    }
    // SAFETY: nenulova hodnota vznikla v `set_child_hook` z `ChildHook`.
    let f: ChildHook = unsafe { core::mem::transmute::<usize, ChildHook>(hook) };
    let (s, l) = f().map_err(|c| Error::from_code(c as i64).unwrap_or(Error::Corrupted))?;
    Ok((Channel::from_handle(Handle::from_raw(s)), Channel::from_handle(Handle::from_raw(l))))
}
