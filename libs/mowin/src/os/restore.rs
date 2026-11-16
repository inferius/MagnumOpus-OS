//! Obnova oken po padu kompozitoru (`docs/cs/obnova-sluzeb.md`, pravidlo 5
//! "Proudy a push kanaly"; jako `ui::client::restore`).
//!
//! 1. `osrt::gui::reconnect`: supervisor pres listek role Gui vyda novou
//!    relaci na nove instanci `compd` se stejnou identitou.
//! 2. `HELLO`, pak posledni potvrzeny stav ze zaznamu (`guiproto::replay`):
//!    okna, vlastnici (`WINDOW_OWNER`), buffery (handly VMO drzi `mowin`),
//!    vrstvy (obsah oken i popupy), titulky.
//! 3. `COMMIT` kazdeho ukazaneho okna (okno je hned videt se starym
//!    obsahem), buffery: ten na obrazovce drzi kompozitor, ostatni volne.
//! 4. Ovladac dostane `MOWIN_EV_EXPOSE` pro kazde okno (Wine prekresli);
//!    velikosti potvrdi dalsi `CONFIGURE`.
//!
//! Nikdy nevisi: obnova listkem ma termin; kdyz nova instance spadne behem
//! obnovy, zkusi se to znovu (nejvic `ATTEMPTS`x).

use std::os::magnumopus::runtime as rt;
use std::time::Duration;

use guiproto::window::ClientMsg;
use libos::{Channel, Error, Handle, Result};

use super::State;
use crate::event::kind;

/// Jak dlouho cekat na novou instanci kompozitoru.
const RECONNECT: Duration = Duration::from_secs(10);
const ATTEMPTS: u32 = 3;

impl State {
    /// Kanal zanikl: okna na nove instanci kompozitoru. Bez listku (proces
    /// bez role Gui) nebo po odvolani GUI `SERVICE_UNAVAILABLE`.
    pub(super) fn recover(&mut self) -> Result<()> {
        self.replay.drop_staged();
        let mut last = Error::ServiceUnavailable;
        for _ in 0..ATTEMPTS {
            let raw = rt::gui::reconnect_raw(RECONNECT).map_err(|c| Error::from_code(c as i64).unwrap_or(Error::ServiceUnavailable))?;
            self.ch = Some(Channel::from_handle(Handle::from_raw(raw)));
            self.hello = false;
            match self.replay_on_new() {
                Ok(()) => {
                    println!("mowin: okna obnovena po restartu kompozitoru");
                    let ids: Vec<u32> = self.windows.keys().copied().collect();
                    for w in ids {
                        self.push(kind::EXPOSE, w, [0; 4]);
                    }
                    return Ok(());
                }
                // Nova instance spadla behem obnovy: znovu pres listek.
                Err(Error::PeerClosed | Error::ServiceUnavailable) => last = Error::ServiceUnavailable,
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }

    /// Posle zaznam na novou instanci (zpravy uz v zaznamu jsou, znovu se
    /// nezaznamenavaji).
    fn replay_on_new(&mut self) -> Result<()> {
        self.ensure_hello()?;
        for m in self.replay.replay() {
            let handles = match &m {
                ClientMsg::BufferCreate { buffer, .. } => match self.buffer_handle(*buffer) {
                    Some(h) => vec![h?],
                    None => continue,
                },
                _ => Vec::new(),
            };
            self.raw_send(&m, handles)?;
        }
        // Ukazana okna hned videt (vrstvy se starymi buffery).
        let shown: Vec<u32> = self.windows.iter().filter(|(_, w)| w.shown).map(|(k, _)| *k).collect();
        for window in shown {
            self.txn += 1;
            let txn = self.txn;
            self.raw_send(&ClientMsg::Commit { txn, flags: 0, window, serial: 0, token: 0 }, Vec::new())?;
        }
        // Buffer, ktery vrstva ukazuje, drzi nova instance (uvolni ho
        // RELEASED po dalsim commitu); ostatni jsou volne.
        let replay = &self.replay;
        for s in self.windows.values_mut().map(|w| &mut w.content).chain(self.popups.values_mut().map(|p| &mut p.surface)) {
            let shown = replay.layer_buffer(s.layer);
            for (i, b) in s.bufs.iter_mut().enumerate() {
                if let Some(b) = b {
                    b.busy = Some(b.id) == shown;
                    if b.busy {
                        s.front = i;
                    }
                }
            }
        }
        for w in self.windows.values_mut() {
            w.ack = None;
        }
        Ok(())
    }

    fn raw_send(&self, m: &ClientMsg, handles: Vec<Handle>) -> Result<()> {
        self.ch()?.write(&m.encode(), handles)
    }

    /// Handle VMO bufferu `id` pro novou instanci (jen cteni).
    fn buffer_handle(&self, id: u32) -> Option<Result<Handle>> {
        self.windows
            .values()
            .map(|w| &w.content)
            .chain(self.popups.values().map(|p| &p.surface))
            .flat_map(|s| s.bufs.iter().flatten())
            .find(|b| b.id == id)
            .map(|b| b.readonly())
    }
}
