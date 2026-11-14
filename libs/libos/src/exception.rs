//! Doruceni vyjimek: zprava z kanalu vyjimek a rozhodnuti obsluhy.
//!
//! ```ignore
//! let ch = process.exception_channel(false)?;
//! let e = libos::exception::receive(&ch, None)?;
//! if e.report.kind == abi::exception::kind::PAGE_FAULT { /* opravit */ }
//! e.resume()?; // nebo e.next() / e.kill(); zahozeni = dalsi obsluha
//! ```

use abi::exception::{Report, action};
use abi::{Error, Result};

use crate::{Channel, Handle, Thread, sys};

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
