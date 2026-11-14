//! Sdileny kanal se souběznymi dotazy: kazdy dotaz dostane cislo (`id`),
//! odpovedi se paruji podle nej (viz [`crate::msg`]).
//!
//! Bez vlakna navic: vlakno, ktere ceka a nikdo zrovna necte, se stane
//! "ctenarem" - cte z kanalu, cizi odpovedi ulozi do schranky a probudi
//! ostatni (futex). Jedno vlakno na kanalu tak precte rovnou svou odpoved
//! (jeden zamek, zadny syscall navic).
//!
//! Pozdni odpoved (dotaz uz vyprsel nebo byl zrusen) se zahodi i s handly
//! (napr. kanal pozde navazaneho spojeni - jeho zavrenim spojeni zanikne).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};
use core::time::Duration;

use abi::syscall::DEADLINE_INFINITE;
use libos::sync::Mutex;
use libos::{Channel, Error, Message, Result, sys};

use crate::msg::{self, Header, NO_DEADLINE};
use crate::op;

/// Rezerva nad termin dotazu (zpracovani ve sluzbe, zatizeny system):
/// do te doby musi sluzba odpovedet, jinak klient dotaz zrusi sam.
const REPLY_MARGIN: Duration = Duration::from_secs(10);
/// Dotazy bez cekani na sit (stav, relace, socket): nejvys 30 s.
const IMMEDIATE: Duration = Duration::from_secs(30);
/// Nejdelsi cekani na rozhodnuti uzivatele ve sluzbe (`fw::ASK_WAIT_NS`);
/// cas dotazu se do terminu operace nepocita.
const ASK_WAIT: Duration = Duration::from_secs(300);

/// Jak dlouho ceka klient na odpoved (pojistka proti chybe ve sluzbe).
#[derive(Clone, Copy, Debug)]
pub(crate) enum Wait {
    /// Dotaz bez cekani na sit: termin na drate zadny, pojistka 30 s.
    Immediate,
    /// Dotaz s terminem: pojistka = termin + rezerva.
    Op,
    /// Dotaz, ktery muze cekat i na uzivatele (firewall): navic 300 s.
    Gated,
}

fn add(t: u64, d: Duration) -> u64 {
    t.saturating_add(u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
}

impl Wait {
    /// Termin na drate a pojistka klienta (`None` = cekat bez limitu).
    fn limits(self, deadline: u64) -> (u64, Option<u64>) {
        let now = libos::time::now();
        match self {
            Wait::Immediate => (NO_DEADLINE, Some(add(now, IMMEDIATE))),
            Wait::Op if deadline == NO_DEADLINE => (deadline, None),
            Wait::Gated if deadline == NO_DEADLINE => (deadline, None),
            Wait::Op => (deadline, Some(add(deadline.max(now), REPLY_MARGIN))),
            Wait::Gated => (deadline, Some(add(add(deadline.max(now), ASK_WAIT), REPLY_MARGIN))),
        }
    }
}

struct State {
    next_id: u32,
    /// Nektere vlakno prave cte z kanalu.
    reading: bool,
    /// Sluzba kanal zavrela.
    closed: bool,
    /// Rozpracovane dotazy (odpovedi na jine `id` se zahodi).
    waiting: BTreeSet<u32>,
    /// Dorucene odpovedi, ktere si jeste nevyzvedlo jejich vlakno.
    done: BTreeMap<u32, Message>,
}

/// Kanal ke sluzbe site sdileny vlakny.
pub(crate) struct Mux {
    ch: Channel,
    state: Mutex<State>,
    /// Pocitadlo zmen schranky; na nem cekaji vlakna, ktera nectou.
    epoch: AtomicU32,
}

impl Mux {
    pub fn new(ch: Channel) -> Arc<Self> {
        Arc::new(Self {
            ch,
            state: Mutex::new(State {
                next_id: 1,
                reading: false,
                closed: false,
                waiting: BTreeSet::new(),
                done: BTreeMap::new(),
            }),
            epoch: AtomicU32::new(0),
        })
    }

    pub fn channel(&self) -> &Channel {
        &self.ch
    }

    /// Kanal zpet, kdyz ho nikdo jiny nedrzi a nic neni rozpracovane.
    pub fn into_channel(this: Arc<Self>) -> Result<Channel> {
        let m = Arc::try_unwrap(this).map_err(|_| Error::BadState)?;
        let st = m.state.lock();
        if !st.waiting.is_empty() {
            return Err(Error::BadState);
        }
        drop(st);
        Ok(m.ch)
    }

    /// Probudi vlakna cekajici na schranku.
    fn bump(&self) {
        self.epoch.fetch_add(1, Ordering::Release);
        sys::futex_wake(&self.epoch, u32::MAX);
    }

    /// Posle dotaz a pocka na odpoved (cela zprava vcetne hlavicky).
    pub fn call(
        self: &Arc<Self>,
        code: u32,
        deadline: u64,
        body: &[u8],
        wait: Wait,
        cancel: Option<&CancelToken>,
    ) -> Result<Message> {
        let (wire, limit) = wait.limits(deadline);
        let id = {
            let mut st = self.state.lock();
            if st.closed {
                return Err(Error::PeerClosed);
            }
            let mut id = st.next_id;
            while st.waiting.contains(&id) {
                id = id.wrapping_add(1).max(1);
            }
            st.next_id = id.wrapping_add(1).max(1);
            st.waiting.insert(id);
            id
        };
        if let Some(t) = cancel
            && !t.register(self, id)
        {
            self.forget(id);
            return Err(Error::Canceled);
        }
        let req = msg::request(
            Header {
                op: code,
                id,
                deadline: wire,
            },
            body,
        );
        let res = match self.ch.write(&req, Vec::new()) {
            Ok(()) => self.wait_reply(id, limit),
            Err(e) => {
                self.forget(id);
                Err(e)
            }
        };
        if let Some(t) = cancel {
            t.unregister(self, id);
        }
        if matches!(res, Err(Error::TimedOut)) {
            // Sluzba neodpovedela ani s rezervou: dotaz zrusit (pozdni
            // odpoved se zahodi).
            self.cancel(id);
            libos::println!(
                "netproto: sluzba site neodpovedela na dotaz {code} (id {id}) vcas, rusim ho"
            );
        }
        res
    }

    fn forget(&self, id: u32) {
        let mut st = self.state.lock();
        st.waiting.remove(&id);
        st.done.remove(&id);
    }

    /// Posle `CANCEL` pro dotaz `id` (chyba zapisu se ignoruje: kanal je
    /// zavreny, nebo plna fronta - dotaz pak skonci terminem).
    pub fn cancel(&self, id: u32) {
        let req = msg::request(
            Header {
                op: op::CANCEL,
                id,
                deadline: NO_DEADLINE,
            },
            &[],
        );
        let _ = self.ch.write(&req, Vec::new());
    }

    fn wait_reply(&self, id: u32, limit: Option<u64>) -> Result<Message> {
        loop {
            let mut st = self.state.lock();
            if let Some(m) = st.done.remove(&id) {
                st.waiting.remove(&id);
                return Ok(m);
            }
            if st.closed {
                st.waiting.remove(&id);
                return Err(Error::PeerClosed);
            }
            if !st.reading {
                st.reading = true;
                drop(st);
                let r = self.ch.read(limit);
                let mut st = self.state.lock();
                st.reading = false;
                let out = match r {
                    Ok(m) => match msg::reply_id(&m.data) {
                        Some(rid) if rid == id => {
                            st.waiting.remove(&id);
                            Some(Ok(m))
                        }
                        Some(rid) if st.waiting.contains(&rid) => {
                            st.done.insert(rid, m);
                            None
                        }
                        // Pozdni nebo neplatna odpoved: zahodit.
                        _ => None,
                    },
                    Err(Error::PeerClosed) => {
                        st.closed = true;
                        None
                    }
                    Err(e) => {
                        st.waiting.remove(&id);
                        Some(Err(e))
                    }
                };
                // Jine vlakno se muze stat ctenarem nebo si vyzvednout
                // odpoved.
                self.bump();
                drop(st);
                if let Some(o) = out {
                    return o;
                }
            } else {
                let e = self.epoch.load(Ordering::Acquire);
                drop(st);
                let r = sys::futex_wait(&self.epoch, e, limit.unwrap_or(DEADLINE_INFINITE));
                if r == Err(Error::TimedOut) {
                    let mut st = self.state.lock();
                    if let Some(m) = st.done.remove(&id) {
                        st.waiting.remove(&id);
                        return Ok(m);
                    }
                    st.waiting.remove(&id);
                    return Err(Error::TimedOut);
                }
            }
        }
    }
}

/// Zruseni rozpracovanych dotazu z jineho vlakna.
///
/// Token se preda volitelne dotazum (connect, resolve, accept, recv);
/// [`CancelToken::cancel`] posle `CANCEL` na vsechny, ktere s nim prave
/// cekaji, a dalsi dotazy s nim hned vrati `CANCELED`. Jeden token muze
/// slouzit vic dotazum (napr. vsem operacim jednoho stazeni).
#[derive(Clone, Default)]
pub struct CancelToken(Arc<Mutex<TokenState>>);

#[derive(Default)]
struct TokenState {
    canceled: bool,
    active: Vec<(Weak<Mux>, u32)>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// Zrusi rozpracovane i budouci dotazy s timto tokenem.
    pub fn cancel(&self) {
        let active = {
            let mut st = self.0.lock();
            st.canceled = true;
            core::mem::take(&mut st.active)
        };
        for (m, id) in active {
            if let Some(m) = m.upgrade() {
                m.cancel(id);
            }
        }
    }

    pub fn is_canceled(&self) -> bool {
        self.0.lock().canceled
    }

    /// Zapamatuje dotaz; `false` = token uz je zruseny.
    fn register(&self, m: &Arc<Mux>, id: u32) -> bool {
        let mut st = self.0.lock();
        if st.canceled {
            return false;
        }
        st.active.push((Arc::downgrade(m), id));
        true
    }

    fn unregister(&self, m: &Arc<Mux>, id: u32) {
        let mut st = self.0.lock();
        st.active.retain(|(w, i)| !(*i == id && core::ptr::eq(w.as_ptr(), Arc::as_ptr(m))));
    }
}
