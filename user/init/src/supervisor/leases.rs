//! Listky: klient pozada pres svuj listek o novou relaci se stejnym
//! receptem (protokol `svcclient::proto`, popis v
//! `docs/cs/obnova-sluzeb.md`, kapitola "Protokol listku").
//!
//! Listky se hlidaji pres jeden port (jich muze byt vic nez polozek
//! `OBJECT_WAIT_MANY`). Klic portu je cislo listku, ktere se nikdy
//! neopakuje: paket zruseneho listku se jen zahodi.

use alloc::vec;
use alloc::vec::Vec;

use libos::{Channel, Error, Port, Result, Signals, println, time};
use svcclient::policy::CLOSE_GRACE;
use svcclient::proto::{Reply, Request, State, op};

use super::{Happened, Phase, Supervisor};

/// Listek: recept relace sluzby a stav posledniho pozadavku.
pub(super) struct Lease {
    /// Index sluzby v registru.
    service: usize,
    recipe: Vec<u8>,
    channel: Channel,
    /// Generace, na ktere byla vydana posledni relace listku.
    issued: u64,
    pending: Option<Pending>,
}

/// `RECONNECT`, na ktery se jeste neodpovedelo.
#[derive(Clone, Copy)]
struct Pending {
    id: u32,
    deadline: u64,
    /// Instance s generaci `issued` jeste bezi: kdyz bude bezet i po tomto
    /// case, sluzba relaci zavrela umyslne.
    grace_until: u64,
}

impl Supervisor {
    /// Listek k relaci, kterou uz init vytvoril na bezici instanci sluzby
    /// `service` (start systemu). Vraci konec listku pro klienta.
    #[allow(dead_code)] // pouzije init pri predani relaci sluzeb
    pub fn lease(&mut self, service: &str, recipe: Vec<u8>) -> Result<Channel> {
        let i = self.find(service).ok_or(Error::NotFound)?;
        let issued = self.entries[i].generation;
        self.add_lease(i, recipe, issued)
    }

    /// Nova relace podle receptu na bezici instanci a listek k ni:
    /// `(kanal relace, listek)`.
    pub fn session(&mut self, service: &str, recipe: Vec<u8>) -> Result<(Channel, Channel)> {
        let i = self.find(service).ok_or(Error::NotFound)?;
        let ch = self.connect(i, &recipe)?;
        let issued = self.entries[i].generation;
        let lease = self.add_lease(i, recipe, issued)?;
        Ok((ch, lease))
    }

    /// Pocet zivych listku (testy).
    pub fn lease_count(&self) -> usize {
        self.leases.len()
    }

    fn add_lease(&mut self, service: usize, recipe: Vec<u8>, issued: u64) -> Result<Channel> {
        if self.port.is_none() {
            self.port = Some(Port::create()?);
        }
        let (client, server) = Channel::create()?;
        let key = self.next_lease;
        self.next_lease += 1;
        self.leases.insert(
            key,
            Lease {
                service,
                recipe,
                channel: server,
                issued,
                pending: None,
            },
        );
        self.arm(key)?;
        Ok(client)
    }

    /// Zaregistruje listek v portu (registrace je jednorazova).
    fn arm(&mut self, key: u64) -> Result<()> {
        let (Some(port), Some(l)) = (&self.port, self.leases.get(&key)) else {
            return Ok(());
        };
        let r = port.wait_async(l.channel.handle(), key, Signals::READABLE | Signals::PEER_CLOSED);
        if r.is_err() {
            self.leases.remove(&key);
        }
        r
    }

    /// Relace na bezici instanci sluzby `i`.
    fn connect(&mut self, i: usize, recipe: &[u8]) -> Result<Channel> {
        let e = &mut self.entries[i];
        if e.phase != Phase::Running {
            return Err(Error::ServiceUnavailable);
        }
        e.service.as_mut().ok_or(Error::NotSupported)?.connect(recipe)
    }

    /// Nejblizsi lhuta cekajiciho `RECONNECT`.
    pub(super) fn leases_due(&self) -> Option<u64> {
        let now = time::now();
        self.leases
            .values()
            .filter_map(|l| l.pending)
            // Ubehla lhuta pro umyslne zavreni uz nebudi (jinak by smycka
            // tocila naprazdno, dokud sluzba ceka na prodlevu).
            .map(|p| if p.grace_until > now { p.deadline.min(p.grace_until) } else { p.deadline })
            .min()
    }

    /// Pakety portu: listky s novou zpravou nebo zavrene klientem.
    pub(super) fn lease_packets(&mut self) {
        loop {
            let Some(port) = &self.port else { return };
            let Ok(p) = port.wait(Some(0)) else { return };
            let key = p.key;
            let sig = Signals(p.signals);
            if !self.leases.contains_key(&key) {
                continue;
            }
            if sig.intersects(Signals::READABLE) {
                self.lease_message(key);
            }
            let closed = sig.intersects(Signals::PEER_CLOSED)
                && self.leases.get(&key).is_some_and(|l| l.channel.try_read().is_err());
            if closed {
                // Klient listek zavrel: recept zanikne.
                self.leases.remove(&key);
            } else {
                let _ = self.arm(key);
            }
        }
    }

    /// Precte a zpracuje pozadavek na listku `key`.
    fn lease_message(&mut self, key: u64) {
        let Some(l) = self.leases.get(&key) else { return };
        let Ok(m) = l.channel.try_read() else { return };
        let req = match Request::decode(&m.data) {
            Ok(r) => r,
            Err(_) => {
                println!("init: neplatny pozadavek na listku");
                return;
            }
        };
        match req {
            Request::Reconnect { id, wait_ms } => {
                let now = time::now();
                let deadline = now.saturating_add(wait_ms as u64 * 1_000_000);
                let grace_until = now.saturating_add(CLOSE_GRACE.as_nanos() as u64);
                if let Some(l) = self.leases.get_mut(&key) {
                    l.pending = Some(Pending {
                        id,
                        deadline,
                        grace_until,
                    });
                }
                // Odpoved hned, kdyz uz jde (nova instance bezi, vzdano).
                self.serve_lease(key, now, &mut Vec::new());
            }
            Request::Narrow { id, narrowing } => {
                let r = self.narrow(key, &narrowing);
                let (status, handles) = match r {
                    Ok((ch, lease)) => (Ok(()), vec![ch.into_handle(), lease.into_handle()]),
                    Err(e) => (Err(e), Vec::new()),
                };
                self.reply(key, op::NARROW, id, status, handles);
            }
            Request::Status { id } => self.reply(key, op::STATUS, id, Ok(()), Vec::new()),
        }
    }

    /// Zuzeny recept listku `key`: nova relace a novy listek.
    fn narrow(&mut self, key: u64, narrowing: &[u8]) -> Result<(Channel, Channel)> {
        let l = self.leases.get(&key).ok_or(Error::NotFound)?;
        let i = l.service;
        let e = &self.entries[i];
        if e.phase != Phase::Running {
            return Err(Error::ServiceUnavailable);
        }
        let recipe = e
            .service
            .as_ref()
            .ok_or(Error::NotSupported)?
            .narrow(&l.recipe, narrowing)?;
        let ch = self.connect(i, &recipe)?;
        let issued = self.entries[i].generation;
        let lease = self.add_lease(i, recipe, issued)?;
        Ok((ch, lease))
    }

    /// Odpovi na listku `key`; `handles` jen pri uspechu.
    fn reply(&self, key: u64, op: u32, id: u32, status: Result<()>, handles: Vec<libos::Handle>) {
        let Some(l) = self.leases.get(&key) else { return };
        let e = &self.entries[l.service];
        let r = Reply {
            op,
            id,
            status,
            generation: e.generation,
            state: e.state(),
        };
        // Klient mezitim listek zavrel: odpoved nema kdo cist.
        let _ = l.channel.write(&r.encode(), handles);
    }

    /// Projde cekajici `RECONNECT` (po kazdem kole smycky).
    pub(super) fn serve_pending(&mut self, out: &mut Vec<Happened>) {
        let now = time::now();
        let keys: Vec<u64> = self
            .leases
            .iter()
            .filter(|(_, l)| l.pending.is_some())
            .map(|(k, _)| *k)
            .collect();
        for key in keys {
            self.serve_lease(key, now, out);
        }
    }

    /// Odpovi na cekajici `RECONNECT` listku `key`, kdyz uz je co rict.
    fn serve_lease(&mut self, key: u64, now: u64, out: &mut Vec<Happened>) {
        let Some(l) = self.leases.get(&key) else { return };
        let Some(p) = l.pending else { return };
        let e = &self.entries[l.service];
        let issued = l.issued;
        let decision: Option<Result<()>> = match e.state() {
            State::GaveUp | State::Stopped => Some(Err(Error::ServiceUnavailable)),
            State::Running if e.generation > issued => Some(Ok(())),
            // Instance, na ktere byla relace vydana, porad bezi: pad, ktery
            // supervisor jeste nezpracoval, nebo umyslne zavreni.
            State::Running if p.grace_until <= now && Self::alive(e) => Some(Err(Error::PeerClosed)),
            _ if p.deadline <= now => Some(Err(Error::ServiceUnavailable)),
            _ => None,
        };
        let Some(decision) = decision else { return };
        let i = l.service;
        let recipe = l.recipe.clone();
        let result = decision.and_then(|()| self.connect(i, &recipe));
        let generation = self.entries[i].generation;
        let name = self.entries[i].name;
        if let Some(l) = self.leases.get_mut(&key) {
            l.pending = None;
            if result.is_ok() {
                l.issued = generation;
            }
        }
        match result {
            Ok(ch) => {
                self.reply(key, op::RECONNECT, p.id, Ok(()), vec![ch.into_handle()]);
                out.push(Happened::Reconnected { name, generation });
            }
            Err(Error::PeerClosed) => {
                // Sluzba relaci zavrela umyslne (odebrani prav): listek
                // zanikne, aby ho neslo obejit pres NARROW nebo pozdejsi
                // restart.
                self.reply(key, op::RECONNECT, p.id, Err(Error::PeerClosed), Vec::new());
                self.leases.remove(&key);
            }
            Err(e) => self.reply(key, op::RECONNECT, p.id, Err(e), Vec::new()),
        }
    }
}
