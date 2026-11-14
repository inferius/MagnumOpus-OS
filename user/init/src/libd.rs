//! Sluzba knihoven `libd` (`docs/cs/dynamicke-linkovani.md`): init ji
//! spusti hned po case (potrebuje jen initfs) a ridi ji supervisor
//! s adapterem ([`LibdService`], `docs/cs/obnova-sluzeb.md`):
//!
//! - korenove spojeni (bootstrap kanal libd) drzi libos initu; z nej
//!   adapter vytvari relace (`Connect` bez odpovedi - adapter nikdy
//!   neceka),
//! - recept je prazdny (relace libd nema identitu ani prava navic),
//! - deti initu dostanou relaci primo z korene a listek bez IPC na sebe
//!   ([`provider`], `supervisor::queue_lease`); ostatni procesy `NARROW`
//!   na svem listku,
//! - po padu libd supervisor spusti novou instanci a klienti se pripoji
//!   pres listky (`RECONNECT`).

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use libos::spawn::StartHandle;
use libos::{Channel, Error, Process, Result, Rights, Startup};

use crate::supervisor::{Service, StartCtx, Started};

pub const NAME: &str = "libd";

/// Generace bezici instance (pro listky z [`provider`]).
static GENERATION: AtomicU64 = AtomicU64::new(1);

/// Spusti libd a nastavi korenove spojeni pro libos initu.
pub fn start(startup: &Startup) -> Result<Process> {
    let fs = libos::initfs::Initfs::open(startup)?;
    let image = fs.find(NAME).ok_or(Error::NotFound)?;
    let initfs = startup.initfs.as_ref().ok_or(Error::NotFound)?;
    let ro = initfs.handle().duplicate(Some(Rights::READ | Rights::MAP | Rights::BASIC))?;
    let (process, channel) = libos::spawn::service(NAME, image, vec![StartHandle::user(ro)])?;
    libos::libd::set_connection(channel, None);
    libos::libd::set_provider(provider);
    Ok(process)
}

/// Relace a listek pro dite initu: relace z korene libd, listek u
/// supervisoru (bez cekani, i uvnitr jeho kola).
fn provider() -> Result<(Channel, Channel)> {
    let session = libos::libd::connect_root()?;
    let lease = crate::supervisor::queue_lease(NAME, GENERATION.load(Ordering::Acquire))?;
    Ok((session, lease))
}

/// Adapter libd v supervisoru.
pub struct LibdService;

impl Service for LibdService {
    fn start(&mut self, ctx: &StartCtx) -> Result<Started> {
        let p = start(ctx.startup)?;
        GENERATION.store(ctx.generation, Ordering::Release);
        Ok(Started { process: p.handle().duplicate(None)?, ready: None })
    }

    fn connect(&mut self, recipe: &[u8]) -> Result<Channel> {
        if !recipe.is_empty() {
            return Err(Error::InvalidArgs);
        }
        libos::libd::connect_root()
    }

    fn narrow(&self, parent: &[u8], narrowing: &[u8]) -> Result<Vec<u8>> {
        // Vsechny relace libd jsou stejne: zuzit neni co.
        if !parent.is_empty() || !narrowing.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(Vec::new())
    }
}
