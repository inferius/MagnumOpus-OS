//! Sluzba "otevri cestu" (init, `libs/openwith`): `start` a hola cesta
//! k souboru. Kanal sluzby prijde od termd zpravou `SessMsg::Open`; kazda
//! zadost ma vlastni kanal pro odpoved.

use core::time::Duration;

use libos::{Channel, Error, time};
use openwith::msg::{Reply, request, request_app};
use shlang::host::Opened;

/// Kanal sluzby relace (`None` = relace ho nedostala: konzole, bez GUI).
pub static SERVICE: spin::Mutex<Option<Channel>> = spin::Mutex::new(None);

/// Sluzba ceka na aplikaci 20 s; vlastni termin je pojistka, kdyby
/// odpoved neprisla vubec.
const REPLY_TIMEOUT: Duration = Duration::from_secs(25);

/// Rozpracovana zadost: kanal pro odpoved a termin.
#[derive(Default)]
pub struct Pending(Option<(Channel, u64)>);

impl Pending {
    pub fn send(&mut self, path: &str, app_only: bool) -> Result<(), Error> {
        let guard = SERVICE.lock();
        let svc = guard.as_ref().ok_or(Error::NotSupported)?;
        let (mine, theirs) = Channel::create()?;
        let req = if app_only { request_app(path) } else { request(path) };
        svc.write(&req, alloc::vec![theirs.into_handle()])?;
        self.0 = Some((mine, time::deadline_after(REPLY_TIMEOUT)));
        Ok(())
    }

    pub fn recv(&mut self, wait_ms: u64) -> Result<Option<Opened>, Error> {
        let (ch, deadline) = self.0.as_ref().ok_or(Error::BadState)?;
        let wait = time::deadline_after(Duration::from_millis(wait_ms)).min(*deadline);
        let m = match ch.read(Some(wait)) {
            Ok(m) => m,
            Err(Error::TimedOut) if time::now() < *deadline => return Ok(None),
            Err(e) => {
                self.0 = None;
                return Err(e);
            }
        };
        self.0 = None;
        Ok(Some(match Reply::decode(&m.data).ok_or(Error::Corrupted)? {
            Reply::Opened(a) => Opened::App(a),
            Reply::Revealed(a) => Opened::Revealed(a),
            Reply::NoApp(_) => Opened::NoApp,
            Reply::Failed(why) => Opened::Failed(why),
        }))
    }
}
