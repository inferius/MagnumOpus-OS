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
    /// Listek, ze ktereho tento vznikl pres `ADOPT` nebo `NARROW`
    /// (pocitadlo zijicich; odvolani rodice odvola i jeho).
    parent: Option<u64>,
    /// Limit `ADOPT` a `NARROW` z tohoto listku.
    adopts: svcclient::policy::AdoptLimit,
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

/// Listky vydane mimo smycku supervisoru (poskytovatel relaci libd pro
/// deti initu: spawn bezi i uvnitr kola supervisoru, kdy je zamceny):
/// (sluzba, generace relace, konec supervisoru). Supervisor je prevezme na
/// zacatku kola; klient muze listek pouzit hned (zprava pocka v kanalu).
static PENDING: spin::Mutex<Vec<(&'static str, u64, Channel)>> = spin::Mutex::new(Vec::new());

/// Listek k relaci sluzby `service` vydane na instanci `issued` (prazdny
/// recept), bez cekani na supervisor: je-li volny, prevezme ho hned,
/// jinak na zacatku pristiho kola. Vraci konec listku pro klienta.
pub fn queue_lease(service: &'static str, issued: u64) -> Result<Channel> {
    let (client, server) = Channel::create()?;
    match super::SUPERVISOR.try_lock() {
        Some(mut sup) => {
            let i = sup.find(service).ok_or(Error::NotFound)?;
            sup.insert_lease(i, Vec::new(), issued, server, None)?;
        }
        None => PENDING.lock().push((service, issued, server)),
    }
    Ok(client)
}

impl Supervisor {
    /// Prevezme listky z [`queue_lease`] (zacatek kola).
    pub(super) fn adopt_pending(&mut self) {
        let list = core::mem::take(&mut *PENDING.lock());
        for (service, issued, server) in list {
            match self.find(service) {
                Some(i) => {
                    if let Err(e) = self.insert_lease(i, Vec::new(), issued, server, None) {
                        println!("init: listek {service} nelze prevzit: {e}");
                    }
                }
                None => println!("init: listek nezname sluzby {service}"),
            }
        }
    }

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
    #[cfg(feature = "tests")]
    pub fn lease_count(&self) -> usize {
        self.leases.len()
    }

    fn add_lease(&mut self, service: usize, recipe: Vec<u8>, issued: u64) -> Result<Channel> {
        let (client, server) = Channel::create()?;
        self.insert_lease(service, recipe, issued, server, None)?;
        Ok(client)
    }

    /// Odebere listek; prevzaty (`ADOPT`) vrati misto rodici.
    fn drop_lease(&mut self, key: u64) {
        if let Some(l) = self.leases.remove(&key)
            && let Some(p) = l.parent.and_then(|p| self.leases.get_mut(&p))
        {
            p.adopts.release();
        }
    }

    /// Zaregistruje listek s koncem supervisoru `server`.
    fn insert_lease(&mut self, service: usize, recipe: Vec<u8>, issued: u64, server: Channel, parent: Option<u64>) -> Result<()> {
        if self.port.is_none() {
            // Listky na portu kola supervisoru (pakety vybere `lease_packets`).
            self.ensure_waits();
            let w = self.waits.as_ref().ok_or(Error::NoMemory)?;
            self.port = Some(Port::from_handle(w.port().handle().duplicate(None)?));
        }
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
                parent,
                adopts: svcclient::policy::AdoptLimit::new(time::now()),
            },
        );
        self.arm(key)
    }

    /// Zaregistruje listek v portu (registrace je jednorazova).
    fn arm(&mut self, key: u64) -> Result<()> {
        let (Some(port), Some(l)) = (&self.port, self.leases.get(&key)) else {
            return Ok(());
        };
        let r = port.wait_async(l.channel.handle(), key, Signals::READABLE | Signals::PEER_CLOSED);
        if r.is_err() {
            self.drop_lease(key);
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
        let packets = self.waits.as_mut().map(|w| w.take_foreign()).unwrap_or_default();
        for p in packets {
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
                self.drop_lease(key);
            } else {
                let _ = self.arm(key);
            }
        }
    }

    /// Precte a zpracuje pozadavek na listku `key`.
    fn lease_message(&mut self, key: u64) {
        let Some(l) = self.leases.get(&key) else { return };
        let Ok(mut m) = l.channel.try_read() else { return };
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
            // Bez odpovedi: odmitnuti = konec noveho listku se zavre (dite
            // dostane SERVICE_UNAVAILABLE pri pouziti), duvod do logu.
            Request::Adopt { .. } if m.handles.len() == 1 => {
                let end = Channel::from_handle(m.handles.remove(0));
                if let Err(e) = self.adopt(key, end) {
                    let name = self.leases.get(&key).map_or("?", |l| self.entries[l.service].name);
                    println!("init: listek {name}: ADOPT odmitnut ({e}): dite nedostane obnovu relace");
                }
            }
            Request::Adopt { .. } => println!("init: ADOPT bez konce listku"),
        }
    }

    /// `ADOPT`: novy listek se stejnym receptem (zuzeni prazdnym zuzenim)
    /// a generaci vydani jako listek `key`; limit zijicich a rychlosti.
    fn adopt(&mut self, key: u64, end: Channel) -> Result<()> {
        let now = time::now();
        let l = self.leases.get_mut(&key).ok_or(Error::NotFound)?;
        if !l.adopts.take(now) {
            return Err(Error::NoResources);
        }
        let (i, issued) = (l.service, l.issued);
        let parent_recipe = l.recipe.clone();
        let recipe = self.entries[i]
            .service
            .as_ref()
            .ok_or(Error::NotSupported)
            .and_then(|s| s.narrow(&parent_recipe, &[]));
        let r = recipe.and_then(|r| self.insert_lease(i, r, issued, end, Some(key)));
        if r.is_err()
            && let Some(l) = self.leases.get_mut(&key)
        {
            l.adopts.release();
        }
        r
    }

    /// Zuzeny recept listku `key`: nova relace a novy listek, svazany
    /// s rodicem (limit zijicich a rychlosti jako `ADOPT`; odvolani rodice
    /// ho odvola taky, [`Supervisor::revoke_leases`]).
    fn narrow(&mut self, key: u64, narrowing: &[u8]) -> Result<(Channel, Channel)> {
        let now = time::now();
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
        if !self.leases.get_mut(&key).is_some_and(|l| l.adopts.take(now)) {
            return Err(Error::NoResources);
        }
        let r = self.connect(i, &recipe).and_then(|ch| {
            let issued = self.entries[i].generation;
            let (client, server) = Channel::create()?;
            self.insert_lease(i, recipe, issued, server, Some(key))?;
            Ok((ch, client))
        });
        if r.is_err()
            && let Some(l) = self.leases.get_mut(&key)
        {
            l.adopts.release();
        }
        r
    }

    /// Odvola listky sluzby `service`, jejichz recept splni `pred`,
    /// a vsechny z nich odvozene (`NARROW`, `ADOPT`, do libovolne hloubky).
    /// Konec supervisoru se zavre: klient dostane `PEER_CLOSED` a obnova
    /// vrati `SERVICE_UNAVAILABLE`. Vraci pocet odvolanych listku.
    #[cfg(feature = "tests")]
    pub fn revoke_leases(&mut self, service: &str, pred: impl Fn(&[u8]) -> bool) -> usize {
        let Some(i) = self.find(service) else { return 0 };
        let mut doomed: Vec<u64> = self.leases.iter().filter(|(_, l)| l.service == i && pred(&l.recipe)).map(|(k, _)| *k).collect();
        // Potomci (rodic je vzdy starsi: klice rostou).
        loop {
            let more: Vec<u64> = self
                .leases
                .iter()
                .filter(|(k, l)| !doomed.contains(k) && l.parent.is_some_and(|p| doomed.contains(&p)))
                .map(|(k, _)| *k)
                .collect();
            if more.is_empty() {
                break;
            }
            doomed.extend(more);
        }
        for k in &doomed {
            self.drop_lease(*k);
        }
        doomed.len()
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
                self.drop_lease(key);
            }
            Err(e) => self.reply(key, op::RECONNECT, p.id, Err(e), Vec::new()),
        }
    }
}
