//! Prace s handly.

use super::{Args, current_process};
use crate::error::{Error, Result};
use crate::object::{Handle, Rights};

/// Handle z argumentu (horni bity musi byt nulove).
pub fn arg_handle(v: u64) -> Result<Handle> {
    u32::try_from(v).map(Handle).map_err(|_| Error::BadHandle)
}

/// `HANDLE_CLOSE(handle)`.
pub fn close(a: Args) -> Result<u64> {
    let h = arg_handle(a[0])?;
    let p = current_process();
    // Objekt se uvolni az mimo zamek tabulky (Drop muze budit vlakna).
    let taken = p.handles().take(h)?;
    drop(taken);
    Ok(0)
}

/// `HANDLE_DUPLICATE(handle, rights) -> handle`. `rights` = u32::MAX zachova
/// prava puvodniho handlu, jinak musi byt jejich podmnozinou.
pub fn duplicate(a: Args) -> Result<u64> {
    let h = arg_handle(a[0])?;
    let p = current_process();
    let mut table = p.handles();
    let rights = if a[1] == u32::MAX as u64 {
        table.rights(h)?
    } else {
        Rights(u32::try_from(a[1]).map_err(|_| Error::InvalidArgs)?)
    };
    table.duplicate(h, rights).map(|h| h.0 as u64)
}
