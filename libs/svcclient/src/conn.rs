//! [`Conn`]: kanal relace ke sluzbe + listek k supervisoru.
//!
//! Volani [`Conn::call`] posle pozadavek a precte odpoved. Kdyz kanal
//! zanikne (`PEER_CLOSED` pri zapisu nebo cteni), knihovna pozada
//! supervisor pres listek o novou relaci (`RECONNECT`), pocka na ni
//! nejdel [`Conn::set_wait`] a podle [`crate::policy::after_loss`]
//! pozadavek zopakuje, nebo vrati chybu. Nikdy neceka bez terminu na
//! supervisor.
//!
//! Zamek drzi kanal po celou dobu volani (odpoved patri k pozadavku), takze
//! volani z vice vlaken se radi za sebe - jako u samotneho kanalu.

use alloc::vec::Vec;
use core::time::Duration;

use abi::{Error, Result};
use libos::{Channel, Handle, Message, time};

use crate::policy::{self, After, Event, Kind};
use crate::proto::{Reply, Request, State, op};

/// Funkce, ktera dostava udalosti obnovy (jmeno sluzby, udalost).
pub type Hook = fn(&str, &Event);

/// Stav spojeni, jak ho vidi aplikace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    Connected,
    /// Kanal zanikl, obnova probiha (nebo probehne pri dalsim volani).
    Restarting,
    /// Posledni obnova selhala touto chybou; dalsi volani to zkusi znovu.
    Unavailable(Error),
}

struct Inner {
    channel: Option<Channel>,
    lease: Option<Channel>,
    generation: u64,
    wait: Duration,
    hook: Option<Hook>,
    next_id: u32,
    state: ConnState,
}

/// Spojeni se sluzbou, ktere prezije jeji restart.
pub struct Conn {
    name: &'static str,
    inner: spin::Mutex<Inner>,
}

impl Conn {
    /// `channel` = kanal relace, `lease` = listek (`None` = bez obnovy:
    /// ztrata kanalu = `SERVICE_UNAVAILABLE`).
    pub fn new(name: &'static str, channel: Channel, lease: Option<Channel>) -> Self {
        Self {
            name,
            inner: spin::Mutex::new(Inner {
                channel: Some(channel),
                lease,
                generation: 0,
                wait: policy::DEFAULT_WAIT,
                hook: None,
                next_id: 1,
                state: ConnState::Connected,
            }),
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Jak dlouho cekat na novou instanci (vychozi [`policy::DEFAULT_WAIT`]).
    pub fn set_wait(&self, wait: Duration) {
        self.inner.lock().wait = wait;
    }

    /// Udalosti obnovy (napr. pro hlasku "sluzba se restartuje").
    pub fn on_event(&self, hook: Hook) {
        self.inner.lock().hook = Some(hook);
    }

    pub fn state(&self) -> ConnState {
        self.inner.lock().state
    }

    /// Generace instance, se kterou spojeni naposledy obnovilo relaci
    /// (0 = zatim zadna obnova).
    pub fn generation(&self) -> u64 {
        self.inner.lock().generation
    }

    pub fn has_lease(&self) -> bool {
        self.inner.lock().lease.is_some()
    }

    /// Posle pozadavek a vrati odpoved. Po padu sluzby obnovi relaci;
    /// `Idempotent` pozadavek zopakuje, `Effect` (a kazdy s handly) vrati
    /// `SERVICE_RESTARTED`.
    pub fn call(&self, kind: Kind, data: &[u8], handles: Vec<Handle>) -> Result<Message> {
        let mut g = self.inner.lock();
        let had_handles = !handles.is_empty();
        let mut handles = Some(handles);
        for _ in 0..policy::MAX_ATTEMPTS {
            if g.channel.is_none() {
                self.recover_locked(&mut g)?;
            }
            let ch = g.channel.as_ref().ok_or(Error::ServiceUnavailable)?;
            let (delivered, res) = match ch.write(data, handles.take().unwrap_or_default()) {
                Ok(()) => (true, ch.read(None)),
                Err(e) => (false, Err(e)),
            };
            match res {
                Err(Error::PeerClosed) => {
                    g.channel = None;
                    self.recover_locked(&mut g)?;
                    if let After::Fail(e) = policy::after_loss(kind, delivered, had_handles) {
                        return Err(e);
                    }
                }
                other => return other,
            }
        }
        Err(Error::ServiceUnavailable)
    }

    /// Kanal relace pro vlastni protokol (proudy, push zpravy). Kdyz zatim
    /// neni (predchozi obnova selhala), zkusi obnovu.
    pub fn with_channel<R>(&self, f: impl FnOnce(&Channel) -> R) -> Result<R> {
        let mut g = self.inner.lock();
        if g.channel.is_none() {
            self.recover_locked(&mut g)?;
        }
        g.channel.as_ref().map(f).ok_or(Error::ServiceUnavailable)
    }

    /// Kanal zanikl (knihovna to zjistila sama, napr. na push kanalu):
    /// zahodi ho a obnovi relaci. Vraci generaci nove instance.
    pub fn recover(&self) -> Result<u64> {
        let mut g = self.inner.lock();
        g.channel = None;
        self.recover_locked(&mut g)?;
        Ok(g.generation)
    }

    /// Zuzena relace pro dite: `(kanal relace, listek)`. Zuzeni (format
    /// sluzby) smi prava jen odebrat. Sluzba musi bezet.
    pub fn narrow(&self, narrowing: &[u8]) -> Result<(Channel, Channel)> {
        let mut g = self.inner.lock();
        let id = Self::next_id(&mut g);
        let req = Request::Narrow {
            id,
            narrowing: narrowing.to_vec(),
        };
        let deadline = time::deadline_after(policy::REPLY_MARGIN);
        let (_, mut h) = self.exchange(&mut g, &req, deadline)?;
        let lease = Channel::from_handle(h.pop().ok_or(Error::Corrupted)?);
        let channel = Channel::from_handle(h.pop().ok_or(Error::Corrupted)?);
        Ok((channel, lease))
    }

    /// Stav sluzby podle supervisoru: (stav, generace bezici instance).
    pub fn status(&self) -> Result<(State, u64)> {
        let mut g = self.inner.lock();
        let id = Self::next_id(&mut g);
        let deadline = time::deadline_after(policy::REPLY_MARGIN);
        let (r, _) = self.exchange(&mut g, &Request::Status { id }, deadline)?;
        Ok((r.state, r.generation))
    }

    /// Rozlozi spojeni na kanal relace a listek (predani jinemu procesu).
    pub fn into_parts(self) -> (Option<Channel>, Option<Channel>) {
        let i = self.inner.into_inner();
        (i.channel, i.lease)
    }

    fn next_id(g: &mut Inner) -> u32 {
        let id = g.next_id;
        g.next_id = g.next_id.wrapping_add(1).max(1);
        id
    }

    fn emit(&self, g: &Inner, e: Event) {
        if let Some(h) = g.hook {
            h(self.name, &e);
        }
    }

    /// Obnovi relaci pres listek (zamek uz drzi volajici).
    fn recover_locked(&self, g: &mut Inner) -> Result<()> {
        if g.lease.is_none() {
            g.state = ConnState::Unavailable(Error::ServiceUnavailable);
            return Err(Error::ServiceUnavailable);
        }
        g.state = ConnState::Restarting;
        self.emit(g, Event::Restarting);
        // Pozdni uspesna odpoved z drivejska = relace, kterou supervisor
        // vydal naposledy: prevzit ji.
        if self.adopt_queued(g) {
            return Ok(());
        }
        let id = Self::next_id(g);
        let req = Request::Reconnect {
            id,
            wait_ms: policy::wait_ms(g.wait),
        };
        let deadline = time::deadline_after(g.wait + policy::REPLY_MARGIN);
        match self.exchange(g, &req, deadline) {
            Ok((r, mut h)) => {
                let ch = Channel::from_handle(h.pop().ok_or(Error::Corrupted)?);
                self.adopt(g, ch, r.generation);
                Ok(())
            }
            Err(e) => {
                g.state = ConnState::Unavailable(e);
                self.emit(g, Event::Unavailable(e));
                Err(e)
            }
        }
    }

    fn adopt(&self, g: &mut Inner, ch: Channel, generation: u64) {
        g.channel = Some(ch);
        g.generation = generation;
        g.state = ConnState::Connected;
        self.emit(g, Event::Restored { generation });
    }

    /// Zpravy, ktere uz cekaji na listku: uspesna odpoved `RECONNECT` =
    /// prevzit relaci (posledni vyhrava), ostatni zahodit.
    fn adopt_queued(&self, g: &mut Inner) -> bool {
        let mut found = None;
        while let Some(m) = g.lease.as_ref().and_then(|l| l.try_read().ok()) {
            if let Some((ch, generation)) = reconnected(m) {
                found = Some((ch, generation));
            }
        }
        match found {
            Some((ch, generation)) => {
                self.adopt(g, ch, generation);
                true
            }
            None => false,
        }
    }

    /// Posle pozadavek na listek a pocka na odpoved se stejnym `id` (nejdel
    /// do `deadline`). Pozdni uspesne `RECONNECT` po ceste prevezme.
    fn exchange(&self, g: &mut Inner, req: &Request, deadline: u64) -> Result<(Reply, Vec<Handle>)> {
        let lease = g.lease.as_ref().ok_or(Error::ServiceUnavailable)?;
        match lease.write(&req.encode(), Vec::new()) {
            Ok(()) => {}
            Err(Error::PeerClosed) => return Err(Error::ServiceUnavailable),
            Err(e) => return Err(e),
        }
        loop {
            let lease = g.lease.as_ref().ok_or(Error::ServiceUnavailable)?;
            let m = match lease.read(Some(deadline)) {
                Ok(m) => m,
                // Supervisor nebezi nebo neodpovedel: sluzba neni k dispozici.
                Err(Error::PeerClosed | Error::TimedOut) => return Err(Error::ServiceUnavailable),
                Err(e) => return Err(e),
            };
            let Ok(r) = Reply::decode(&m.data) else { continue };
            if r.id != req.id() || r.op != req.op() {
                // Pozdni odpoved na drivejsi pozadavek: uspesne `RECONNECT`
                // nese relaci, kterou supervisor vydal naposledy - prevzit.
                if let Some((ch, generation)) = reconnected(m) {
                    self.adopt(g, ch, generation);
                }
                continue;
            }
            r.status?;
            if m.handles.len() != Reply::handles_for(r.op) {
                return Err(Error::Corrupted);
            }
            return Ok((r, m.handles));
        }
    }
}

/// Uspesna odpoved `RECONNECT` -> (kanal relace, generace).
fn reconnected(mut m: Message) -> Option<(Channel, u64)> {
    let r = Reply::decode(&m.data).ok()?;
    if r.op != op::RECONNECT || r.status.is_err() || m.handles.len() != 1 {
        return None;
    }
    Some((Channel::from_handle(m.handles.pop()?), r.generation))
}
