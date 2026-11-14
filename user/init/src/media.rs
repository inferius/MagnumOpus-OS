//! Media: sluzba `mediad` pod supervisorem (`docs/cs/multimedia.md`).
//!
//! mediad dostane jen initfs (obraz dekoderu); kazdy dekoder spousti ve
//! vlastnim jobu pod svym jobem bez dalsich handlu. Bootstrap kanal je
//! ridici (jen init); klienti dostavaji relace pres `CONNECT`
//! ([`connect`], testy a aplikace). Po padu sluzbu restartuje supervisor;
//! relace padle sluzby skonci (klient otevre novou).

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::time::Duration;

use libos::abi::bootstrap::kind;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Process, Result, Startup, println, time};
use mediaproto::Request;

use crate::supervisor::{Service, Spec, StartCtx, Started};

/// Jak dlouho cekat na odpoved sluzby na `CONNECT`.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// Ridici kanal bezici instance.
static CONTROL: spin::Mutex<Option<Channel>> = spin::Mutex::new(None);

fn connect_on(control: &Channel) -> Result<Channel> {
    while control.try_read().is_ok() {}
    control.write(&Request::Connect.encode(), Vec::new())?;
    let m = control.read(Some(time::deadline_after(REPLY_TIMEOUT)))?;
    match mediaproto::decode_incoming(&m.data)? {
        mediaproto::Incoming::Reply(r) => {
            r?;
        }
        mediaproto::Incoming::Event(_) => return Err(Error::Corrupted),
    }
    m.handles.into_iter().next().map(Channel::from_handle).ok_or(Error::Corrupted)
}

/// Novy kanal relace medii (testy, aplikace).
pub fn connect() -> Result<Channel> {
    let c = CONTROL.lock();
    connect_on(c.as_ref().ok_or(Error::ServiceUnavailable)?)
}

fn spawn(startup: &Startup) -> Result<(Process, Channel)> {
    let vmo = startup.initfs.as_ref().ok_or(Error::NotFound)?;
    let handles = alloc::vec![StartHandle::new(kind::INITFS, 0, vmo.handle().duplicate(None)?)];
    let app = crate::apps::spawn(startup, "mediad", handles)?;
    Ok((app.process, app.control))
}

struct Mediad;

impl Service for Mediad {
    fn start(&mut self, ctx: &StartCtx) -> Result<Started> {
        let (process, control) = spawn(ctx.startup)?;
        let started = Started { process: process.handle().duplicate(None)?, ready: Some(control.handle().duplicate(None)?) };
        *CONTROL.lock() = Some(control);
        drop(process);
        Ok(started)
    }

    fn ready(&mut self) -> Result<bool> {
        let c = CONTROL.lock();
        let control = c.as_ref().ok_or(Error::BadState)?;
        match control.try_read() {
            Ok(m) if m.data == b"ok" => Ok(true),
            Ok(_) => Err(Error::IoError),
            Err(Error::ShouldWait) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn connect(&mut self, _recipe: &[u8]) -> Result<Channel> {
        connect()
    }

    /// Relace sluzby nemaji prava, zuzeni nic nemeni.
    fn narrow(&self, parent: &[u8], _narrowing: &[u8]) -> Result<Vec<u8>> {
        Ok(parent.to_vec())
    }

    fn stopped(&mut self) {
        *CONTROL.lock() = None;
    }
}

/// Spusti mediad a zaregistruje ho v supervisoru.
pub fn start(startup: &Startup, sup: &mut crate::supervisor::Supervisor) -> Result<()> {
    let (process, control) = spawn(startup)?;
    let msg = control.read(Some(time::deadline_after(Duration::from_secs(10))))?;
    if msg.data != b"ok" {
        let _ = process.kill();
        return Err(Error::Corrupted);
    }
    *CONTROL.lock() = Some(control);
    sup.register(Spec { name: "mediad", deps: &[], service: Some(Box::new(Mediad)) }, Some(&process));
    println!("init: sluzba medii bezi");
    Ok(())
}
