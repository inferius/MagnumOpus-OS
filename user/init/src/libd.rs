//! Sluzba knihoven `libd` (`docs/cs/dynamicke-linkovani.md`): init ji
//! spusti hned po case (potrebuje jen initfs) a jeji spojeni preda libos
//! (`libos::libd`). Od ted kazdy proces, ktery init (a jeho potomci)
//! spusti, dostane vlastni spojeni a muze spoustet programy se zavadecem.

use alloc::vec;

use libos::spawn::StartHandle;
use libos::{Error, Process, Result, Rights, Startup};

use crate::supervisor::{Restart, Started};

/// Spusti libd a nastavi spojeni s ni pro libos initu.
pub fn start(startup: &Startup) -> Result<Process> {
    let fs = libos::initfs::Initfs::open(startup)?;
    let image = fs.find("libd").ok_or(Error::NotFound)?;
    let initfs = startup.initfs.as_ref().ok_or(Error::NotFound)?;
    let ro = initfs.handle().duplicate(Some(Rights::READ | Rights::MAP | Rights::BASIC))?;
    let (process, channel) = libos::spawn::service("libd", image, vec![StartHandle::user(ro)])?;
    libos::libd::set_connection(channel);
    Ok(process)
}

/// Restart po padu: nova instance a nove spojeni initu. Procesy spustene
/// drive maji spojeni se starou instanci (zavrene) - programy se zavadecem
/// uz nespusti (DL-4: listek pro obnovu spojeni zatim neni).
pub const RESTART: Restart = Restart { start: restart, ready: None };

fn restart(startup: &Startup) -> Result<Started> {
    let p = start(startup)?;
    Ok(Started { process: p.handle().duplicate(None)?, ready: None })
}
