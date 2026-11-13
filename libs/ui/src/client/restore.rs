//! Obnova okna po padu kompozitoru (viz `docs/cs/obnova-sluzeb.md`,
//! pravidlo 5 "Proudy a push kanaly").
//!
//! 1. `conn.recover()`: supervisor pres listek vyda novou relaci na nove
//!    instanci `compd` se stejnou identitou (recept = `ConnectInfo`
//!    z manifestu; identitu dava init, ne aplikace).
//! 2. `HELLO`, pak posledni potvrzeny stav ze zaznamu (`guiproto::replay`):
//!    okno, buffery (s handly VMO, ktere drzi aplikace), vrstvy,
//!    titulek, oblasti a strom pristupnosti.
//! 3. Prvni `CONFIGURE`: stejna velikost = `COMMIT` s potvrzenim (okno je
//!    hned videt), jina = udalost `Resized` (aplikace nakresli a potvrdi).
//! 4. Fronta dostane `Restored`. Rozpracovana transakce se zahodi a volajici
//!    dostane `SERVICE_RESTARTED`.
//!
//! Nikdy nevisi: cekani na listek ma termin (`Conn`, vychozi 10 s), cekani
//! na novou instanci pri pozdravu taky ([`HANDSHAKE`]). Kdyz nova instance
//! spadne behem obnovy, zkusi se to znovu (nejvic [`ATTEMPTS`]x).

use alloc::vec::Vec;
use core::time::Duration;

use guiproto::window::{ClientMsg, commit};
use libos::{Error, Handle, Result};
use svcclient::ConnState;

use super::{Event, Window, handshake};

/// Jak dlouho ceka pozdrav na novou instanci (bezi uz, odpovida hned).
const HANDSHAKE: Duration = Duration::from_secs(5);
/// Pokusu o obnovu za sebou (nova instance muze spadnout taky).
const ATTEMPTS: u32 = 3;

impl Window {
    /// Kanal zanikl: obnovit okno na nove instanci. `Err(SERVICE_RESTARTED)`
    /// = okno je obnovene, ale rozpracovana transakce se ztratila (aplikace
    /// at okno nakresli znovu). Bez listku (nebo kdyz to supervisor vzdal)
    /// `PEER_CLOSED` / `SERVICE_UNAVAILABLE`.
    pub(super) fn restore(&mut self) -> Result<()> {
        let lost = self.replay.lost_staged();
        self.replay.drop_staged();
        let mut last = Error::ServiceUnavailable;
        for _ in 0..ATTEMPTS {
            let generation = match self.conn.recover() {
                Ok(g) => g,
                // Bez listku: jako drive, konec okna.
                Err(e) if !self.conn.has_lease() => return Err(if e == Error::ServiceUnavailable { Error::PeerClosed } else { e }),
                Err(e) => return Err(e),
            };
            match self.replay_on_new() {
                Ok(()) => {
                    libos::println!("{}: okno obnoveno po restartu kompozitoru (generace {generation})", self.app);
                    self.queue.push(Event::Restored { generation });
                    return if lost { Err(Error::ServiceRestarted) } else { Ok(()) };
                }
                // Nova instance spadla behem obnovy: znovu pres listek.
                Err(Error::PeerClosed) => last = Error::PeerClosed,
                Err(e) => return Err(e),
            }
        }
        Err(if last == Error::PeerClosed { Error::ServiceUnavailable } else { last })
    }

    /// Kanal relace chybi (predchozi obnova selhala): obnovit cele okno,
    /// ne jen kanal (nova instance o okne nic nevi).
    pub(super) fn ensure(&mut self) -> Result<()> {
        match self.conn.state() {
            ConnState::Connected => Ok(()),
            _ => self.restore(),
        }
    }

    /// Posle zaznam na novou instanci a dokonci pozdrav.
    fn replay_on_new(&mut self) -> Result<()> {
        let mut msgs: Vec<(ClientMsg, Vec<Handle>)> = Vec::new();
        for m in self.replay.replay() {
            let handles = match &m {
                ClientMsg::BufferCreate { buffer, .. } => match self.buffer_handle(*buffer) {
                    Some(h) => alloc::vec![h?],
                    // Buffer uz aplikace nema (zaznam o nem nevi): preskocit.
                    None => continue,
                },
                _ => Vec::new(),
            };
            msgs.push((m, handles));
        }
        let deadline = Some(libos::time::deadline_after(HANDSHAKE));
        let g = self.conn.with_channel(|ch| handshake(ch, &msgs, deadline))??;
        let (w, h, serial) = g.size;
        self.screen = g.screen;
        self.app = g.app;
        if g.context.is_some() {
            self.context = g.context;
            self.queue.push(super::Event::Context);
        }
        self.frame_pending = false;
        self.ack = None;
        self.reset_buffers();
        if (w, h) == (self.w, self.h) {
            // Obsah sedi s velikosti: potvrdit hned (okno se ukaze).
            self.txn += 1;
            let c = ClientMsg::Commit { txn: self.txn, flags: commit::ACK_CONFIGURE, window: 1, serial, token: 0 };
            let data = c.encode();
            self.conn.with_channel(|ch| super::write(ch, &data, Vec::new()))??;
        } else {
            self.w = w;
            self.h = h;
            self.ack = Some(serial);
            self.replay.configured(1, w, h);
            self.queue.push(Event::Resized { w, h });
        }
        Ok(())
    }

    /// Handle VMO bufferu `id` pro novou instanci (jen cteni).
    fn buffer_handle(&self, id: u32) -> Option<Result<Handle>> {
        self.pairs
            .values()
            .flat_map(|p| p.bufs.iter().flatten())
            .find(|b| b.id == id)
            .map(|b| b.readonly())
    }

    /// Po obnove: buffer, ktery vrstva ukazuje, drzi kompozitor (`busy`,
    /// uvolni ho `RELEASED` po dalsim commitu) a je predni; ostatni jsou
    /// volne. Dalsi kresleni tak jde do jineho bufferu nez toho na
    /// obrazovce.
    fn reset_buffers(&mut self) {
        for (&layer, p) in self.pairs.iter_mut() {
            let shown = self.replay.layer_buffer(layer);
            for (i, b) in p.bufs.iter_mut().enumerate() {
                if let Some(b) = b {
                    b.busy = Some(b.id) == shown;
                    if b.busy {
                        p.front = i;
                    }
                }
            }
        }
    }
}
