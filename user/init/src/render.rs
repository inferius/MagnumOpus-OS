//! Sluzba render `gpud` (kostra kroku 6a-3): render uzly ovladace GPU
//! rozdava `gpud`, ne vlastnik displeje.
//!
//! - Pred predanim displeje kompozitoru si init na kanalu displeje vyzada
//!   **jednou** kanal poskytovatele (`OPEN_PROVIDER`, [`setup`]) a drzi ho
//!   ([`Kept`]); `gpud` dostane duplikat.
//! - `gpud` hlida supervisor ([`RESTART`]): po jeho padu nova instance
//!   dostane dalsi duplikat poskytovatele. Render uzly uz vydane jdou primo
//!   k ovladaci, pad gpud je nepreruse.
//! - Render uzel pro klienta otevre init pres `gpud` ([`open_render`]):
//!   kompozitor ho dostane s prikazem `backend gpu`, testovaci program
//!   Vulkanu ve startovni zprave. Aplikace s manifestem (`[gpu] render`)
//!   a spojeni pres listky prijdou s launchd (`docs/cs/render.md`).

use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use guiproto::display::{Reply as DisplayReply, Request as DisplayRequest};
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Process, Result, Startup, println, time};
use renderproto::node::Reply;
#[cfg(feature = "tests")]
use renderproto::node::Request;

use crate::supervisor::{Kept, Restart, Started};

pub const NAME: &str = "gpud";

const TIMEOUT: Duration = Duration::from_secs(5);

/// Konec poskytovatele render uzlu (duplikaty dostava gpud).
static PROVIDER: spin::Mutex<Option<Kept>> = spin::Mutex::new(None);

/// Bezici gpud: proces a kanal k nemu.
pub struct Gpud {
    pub process: Process,
    control: Channel,
}

pub static GPUD: spin::Mutex<Option<Gpud>> = spin::Mutex::new(None);

/// Vyzada si od ovladace displeje poskytovatele render uzlu a spusti
/// `gpud`. `Ok(None)` = displej render uzly nema (framebuffer, virtio-gpu
/// bez 3D).
pub fn setup(startup: &Startup, display: &Channel) -> Result<Option<Process>> {
    display.write(&DisplayRequest::OpenProvider.encode(), Vec::new())?;
    let m = display.read(Some(time::deadline_after(TIMEOUT)))?;
    let provider = match DisplayReply::decode(&m.data)? {
        DisplayReply::Provider => m.handles.into_iter().next().map(Channel::from_handle).ok_or(Error::Corrupted)?,
        DisplayReply::Error { code: Error::NotSupported, .. } => return Ok(None),
        DisplayReply::Error { code, .. } => return Err(code),
        _ => return Err(Error::Corrupted),
    };
    *PROVIDER.lock() = Some(Kept::new(provider));
    let g = spawn(startup)?;
    let ready = g.control.read(Some(time::deadline_after(TIMEOUT))).and_then(|m| check_ready(&m.data));
    if let Err(e) = ready {
        let _ = g.process.kill();
        return Err(e);
    }
    let process = Process::from_handle(g.process.handle().duplicate(None)?);
    *GPUD.lock() = Some(g);
    Ok(Some(process))
}

fn spawn(startup: &Startup) -> Result<Gpud> {
    let provider = PROVIDER.lock().as_ref().ok_or(Error::BadState)?.give()?;
    let fs = Initfs::open(startup)?;
    let image = fs.find(NAME).ok_or(Error::NotFound)?;
    let (process, control) = libos::spawn::service(NAME, image, vec![StartHandle::user(provider.into_handle())])?;
    Ok(Gpud { process, control })
}

fn check_ready(data: &[u8]) -> Result<()> {
    match Reply::decode(data)? {
        Reply::Opened => Ok(()),
        Reply::Error { code } => Err(code),
    }
}

/// Restart gpud po padu (supervisor).
pub const RESTART: Restart = Restart { start: restart, ready: Some(ready) };

fn restart(startup: &Startup) -> Result<Started> {
    let g = spawn(startup)?;
    let started = Started {
        process: g.process.handle().duplicate(None)?,
        ready: Some(g.control.handle().duplicate(None)?),
    };
    *GPUD.lock() = Some(g);
    Ok(started)
}

fn ready() -> Result<()> {
    let guard = GPUD.lock();
    let g = guard.as_ref().ok_or(Error::BadState)?;
    let m = g.control.try_read()?;
    check_ready(&m.data)?;
    println!("init: sluzba render (gpud) znovu bezi");
    Ok(())
}

/// Novy render uzel od gpud. `NOT_SUPPORTED` = gpud nebezi (displej bez
/// render uzlu).
#[cfg(feature = "tests")]
pub fn open_render() -> Result<Channel> {
    let guard = GPUD.lock();
    let g = guard.as_ref().ok_or(Error::NotSupported)?;
    g.control.write(&Request::Open.encode(), Vec::new())?;
    let m = g.control.read(Some(time::deadline_after(TIMEOUT)))?;
    match Reply::decode(&m.data)? {
        Reply::Opened => m.handles.into_iter().next().map(Channel::from_handle).ok_or(Error::Corrupted),
        Reply::Error { code } => Err(code),
    }
}
