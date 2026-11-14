//! Doruceni vyjimek: zprava z kanalu vyjimek a rozhodnuti obsluhy.
//!
//! ```ignore
//! let ch = process.exception_channel(false)?;
//! let e = libos::exception::receive(&ch, None)?;
//! if e.report.kind == abi::exception::kind::PAGE_FAULT { /* opravit */ }
//! e.resume()?; // nebo e.next() / e.kill(); zahozeni = dalsi obsluha
//! ```
//!
//! **Obsluha primo ve vlaknu** ([`set_handler`]): vlakno skoci na obsluhu
//! s ramcem vyjimky (`abi::exception::frame`) na svem zasobniku
//! (nebo alternativnim, [`set_alt_stack`]); obsluha konci
//! [`handler_return`] (RESUME = pokracovat s kontextem z ramce, NEXT =
//! dalsi obsluha v poradi). Podklad pro SEH a signaly.

use abi::exception::{Report, action};
use abi::{Error, Result};

use crate::{Channel, Handle, Process, Thread, rt, sys};

/// Vstup obsluhy ve vlaknu: dostane ukazatel na ramec vyjimky, nevraci se
/// (konci [`handler_return`]).
pub type Handler = extern "C" fn(frame: *mut u8) -> !;

/// Nastavi obsluhu vyjimek primo ve vlaknu pro vlastni proces (musi lezet
/// ve spustitelne pameti - kod programu).
pub fn set_handler(entry: Handler) -> Result<()> {
    sys::exception_handler(abi::exception::handler::SET_ENTRY, rt::self_process() as u64, entry as *const () as u64)
        .map(|_| ())
}

/// Nastavi obsluhu pro jiny proces (pravo `WRITE`; zavadec, debugger).
pub fn set_handler_for(process: &Process, entry: u64) -> Result<()> {
    sys::exception_handler(abi::exception::handler::SET_ENTRY, process.handle().raw() as u64, entry).map(|_| ())
}

/// Zrusi obsluhu ve vlaknu vlastniho procesu.
pub fn clear_handler() -> Result<()> {
    sys::exception_handler(abi::exception::handler::SET_ENTRY, rt::self_process() as u64, 0).map(|_| ())
}

/// Alternativni zasobnik aktualniho vlakna pro obsluhu (jako
/// `sigaltstack`; nutny pro obsluhu preteceni zasobniku). `size` 0 = zadny.
pub fn set_alt_stack(base: u64, size: u64) -> Result<()> {
    sys::exception_handler(abi::exception::handler::SET_STACK, base, size).map(|_| ())
}

/// Zprava o vyjimce z ramce obsluhy ve vlaknu.
///
/// # Safety
///
/// `frame` je ramec, ktery obsluha dostala.
pub unsafe fn frame_report(frame: *const u8) -> Report {
    // SAFETY: predano volajicim (ramec ma aspon `Report::SIZE` B).
    let b = unsafe { core::slice::from_raw_parts(frame, Report::SIZE) };
    Report::decode(b).unwrap_or_default()
}

/// Navrat z obsluhy ve vlaknu (`action::RESUME` nebo `action::NEXT`). Pri
/// uspechu se nevraci (vlakno pokracuje v kontextu z ramce); vrati jen
/// chybu (neplatny ramec = `INVALID_ARGS`, zadna obsluha = `BAD_STATE`).
///
/// # Safety
///
/// `frame` je ramec, ktery obsluha dostala (pripadne upraveny).
pub unsafe fn handler_return(frame: *const u8, disposition: u32) -> Error {
    match sys::exception_handler(abi::exception::handler::RETURN, frame as u64, disposition as u64) {
        Err(e) => e,
        Ok(_) => Error::BadState,
    }
}

/// Vyjimka doruzena obsluze: zprava, vlakno (kontext pres
/// [`Thread::read_state`] / [`Thread::write_state`]) a token.
pub struct Exception {
    pub report: Report,
    pub thread: Thread,
    token: Handle,
}

impl Exception {
    fn resolve(&self, a: u32) -> Result<()> {
        sys::exception_resolve(self.token.raw(), a)
    }

    /// Vlakno pokracuje (se zmenenym kontextem).
    pub fn resume(&self) -> Result<()> {
        self.resolve(action::RESUME)
    }

    /// Dalsi obsluha v poradi.
    pub fn next(&self) -> Result<()> {
        self.resolve(action::NEXT)
    }

    /// Ukoncit proces (jako neosetrena vyjimka).
    pub fn kill(&self) -> Result<()> {
        self.resolve(action::KILL)
    }
}

/// Precte dalsi vyjimku z kanalu vyjimek (ceka do `deadline`).
pub fn receive(ch: &Channel, deadline: Option<u64>) -> Result<Exception> {
    let m = ch.read(deadline)?;
    let report = Report::decode(&m.data).ok_or(Error::Corrupted)?;
    let mut handles = m.handles.into_iter();
    let (Some(thread), Some(token)) = (handles.next(), handles.next()) else {
        return Err(Error::Corrupted);
    };
    Ok(Exception { report, thread: Thread::from_handle(thread), token })
}
