//! Plot (fence): casova osa `u64` pro explicitni synchronizaci (GPU,
//! video, kompozitor). Stavovy automat je `abi::fence::Timeline`.
//!
//! Cekani na hodnotu ma vlastni seznam cekajicich (vlakna `FENCE_WAIT`
//! a registrace portu `FENCE_WAIT_ASYNC`), kontrola a registrace probihaji
//! pod stejnym zamkem jako posun, takze probuzeni neutece. Binarni pohled
//! (`SIGNALED`) jde pres `ObjectCore` jako u ostatnich objektu.
//!
//! Opusteni: kazdy handle s pravem `SIGNAL` drzi vlastni [`FenceSignaler`]
//! (obalka, kterou handle tabulka dosadi pri vlozeni, viz [`bind`]). Obalka
//! zije i ve zprave kanalu, takze predani producenta jinemu procesu plot
//! neopusti; zanik posledni obalky (zavreni, pad procesu, zahozena zprava)
//! plot opusti a probudi vsechny, kdo cekaji na nedosazenou hodnotu.

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::any::Any;
use core::sync::atomic::{AtomicUsize, Ordering};

use abi::fence::Timeline;

use super::{Error, KernelObject, ObjectCore, ObjectType, Rights, Signals, port};
use crate::arch;
use crate::sched::{self, Thread};
use crate::sync::SpinLock;

/// Kdo ceka na hodnotu.
enum Waiter {
    Thread { thread: Arc<Thread>, seq: u64 },
    /// Jednorazova registrace portu (ma v nem rezervovane misto).
    Port { port: Weak<port::Port>, key: u64 },
}

struct ValueWaiter {
    target: u64,
    waiter: Waiter,
}

impl Drop for ValueWaiter {
    /// Nedorucena registrace portu vrati portu rezervovane misto.
    fn drop(&mut self) {
        if let Waiter::Port { port, .. } = &self.waiter
            && let Some(p) = port.upgrade()
        {
            p.unreserve();
        }
    }
}

struct Inner {
    timeline: Timeline,
    waiters: Vec<ValueWaiter>,
}

pub struct Fence {
    core: ObjectCore,
    inner: SpinLock<Inner>,
    /// Pocet zivych [`FenceSignaler`].
    signalers: AtomicUsize,
}

fn signals_of(t: &Timeline) -> Signals {
    if t.signaled() { Signals::SIGNALED } else { Signals::NONE }
}

impl Fence {
    pub fn new(initial: u64) -> Arc<Self> {
        let t = Timeline::new(initial);
        Arc::new(Self {
            core: ObjectCore::new(signals_of(&t)),
            inner: SpinLock::new(Inner {
                timeline: t,
                waiters: Vec::new(),
            }),
            signalers: AtomicUsize::new(0),
        })
    }

    pub fn info(&self) -> abi::fence::Info {
        self.inner.lock().timeline.info()
    }

    /// `FENCE_SIGNAL`: posun nebo trvala chyba.
    pub fn signal(&self, value: u64, failed: bool) -> Result<(), Error> {
        let mut inner = self.inner.lock();
        inner.timeline.signal(value, failed)?;
        self.wake(&mut inner);
        Ok(())
    }

    /// Zanikl posledni handle s pravem `SIGNAL`.
    fn abandon(&self) {
        let mut inner = self.inner.lock();
        inner.timeline.abandon();
        self.wake(&mut inner);
    }

    /// Probudi cekajici, jejichz cil uz je rozhodnuty, a nastavi
    /// `SIGNALED`. Vola se pod zamkem `inner`.
    fn wake(&self, inner: &mut Inner) {
        let t = inner.timeline;
        let signals = signals_of(&t);
        inner.waiters.retain_mut(|w| {
            if t.check(w.target).is_none() {
                // Registrace zavreneho portu uz nikdo nevybere.
                return !matches!(&w.waiter, Waiter::Port { port, .. } if port.strong_count() == 0);
            }
            match &mut w.waiter {
                Waiter::Thread { thread, seq } => sched::wake(thread, *seq),
                Waiter::Port { port, key } => {
                    // Doruceno: rezervace se spotrebuje (Drop ji uz nevraci).
                    let port = core::mem::take(port);
                    if let Some(p) = port.upgrade() {
                        p.deliver(*key, signals, None);
                    }
                }
            }
            false
        });
        if signals != Signals::NONE {
            self.core.update(Signals::NONE, signals);
        }
    }

    /// `FENCE_WAIT_ASYNC`: az bude `target` rozhodnuty, port dostane paket
    /// s `key` (hned, pokud uz je). Misto v portu se rezervuje predem.
    pub fn observe_async(&self, port: &Arc<port::Port>, key: u64, target: u64) -> Result<(), Error> {
        port.reserve()?;
        let mut inner = self.inner.lock();
        if inner.timeline.check(target).is_some() {
            port.deliver(key, signals_of(&inner.timeline), None);
            return Ok(());
        }
        if inner.waiters.try_reserve(1).is_err() {
            drop(inner);
            port.unreserve();
            return Err(Error::NoMemory);
        }
        inner.waiters.push(ValueWaiter {
            target,
            waiter: Waiter::Port {
                port: Arc::downgrade(port),
                key,
            },
        });
        Ok(())
    }

    /// Rozhodnuty vysledek, nebo registrace vlakna (atomicky vuci posunu).
    fn check_or_observe(&self, thread: &Arc<Thread>, seq: u64, target: u64) -> Option<Result<(), Error>> {
        let mut inner = self.inner.lock();
        if let Some(r) = inner.timeline.check(target) {
            return Some(r);
        }
        if inner.waiters.try_reserve(1).is_err() {
            return Some(Err(Error::NoMemory));
        }
        inner.waiters.push(ValueWaiter {
            target,
            waiter: Waiter::Thread {
                thread: thread.clone(),
                seq,
            },
        });
        None
    }

    fn remove_thread(&self, thread: &Thread, seq: u64) {
        self.inner.lock().waiters.retain(|w| match &w.waiter {
            Waiter::Thread { thread: t, seq: s } => !(core::ptr::eq(&**t, thread) && *s == seq),
            Waiter::Port { .. } => true,
        });
    }

    /// Pocet cekajicich vlaken (pro testy: po skonceni nesmi zadne zustat).
    #[cfg_attr(not(feature = "qemu-test"), allow(dead_code))]
    pub fn thread_waiters(&self) -> usize {
        self.inner
            .lock()
            .waiters
            .iter()
            .filter(|w| matches!(w.waiter, Waiter::Thread { .. }))
            .count()
    }
}

impl KernelObject for Fence {
    fn core(&self) -> &ObjectCore {
        &self.core
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Fence
    }

    fn default_rights(&self) -> Rights {
        Rights::BASIC | Rights::SIGNAL
    }
}

/// Handle s pravem `SIGNAL` na plot (producent). Signaly i typ jsou
/// plotu; zanik posledni obalky plot opusti.
pub struct FenceSignaler {
    fence: Arc<Fence>,
}

impl FenceSignaler {
    pub fn new(fence: Arc<Fence>) -> Arc<Self> {
        fence.signalers.fetch_add(1, Ordering::AcqRel);
        Arc::new(Self { fence })
    }
}

impl Drop for FenceSignaler {
    fn drop(&mut self) {
        if self.fence.signalers.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.fence.abandon();
        }
    }
}

impl KernelObject for FenceSignaler {
    fn core(&self) -> &ObjectCore {
        &self.fence.core
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Fence
    }

    fn default_rights(&self) -> Rights {
        self.fence.default_rights()
    }
}

/// Objekt, ktery ulozit do handle tabulky pro handle s pravy `rights`:
/// plot s `SIGNAL` dostane obalku producenta, bez `SIGNAL` holy plot
/// (duplikat nebo predani s ubranym pravem producenta nedrzi). Ostatni
/// objekty beze zmeny.
pub fn bind(object: Arc<dyn KernelObject>, rights: Rights) -> Arc<dyn KernelObject> {
    if object.object_type() != ObjectType::Fence {
        return object;
    }
    let any: Arc<dyn Any + Send + Sync> = object.clone();
    if rights.contains(Rights::SIGNAL) {
        if let Ok(f) = any.downcast::<Fence>() {
            return FenceSignaler::new(f);
        }
    } else if let Ok(s) = any.downcast::<FenceSignaler>() {
        return s.fence.clone();
    }
    object
}

/// Plot za objektem handlu (holy plot i obalka producenta).
pub fn fence_of(object: Arc<dyn KernelObject>) -> Result<Arc<Fence>, Error> {
    let any: Arc<dyn Any + Send + Sync> = object;
    match any.downcast::<Fence>() {
        Ok(f) => Ok(f),
        Err(any) => any
            .downcast::<FenceSignaler>()
            .map(|s| s.fence.clone())
            .map_err(|_| Error::WrongType),
    }
}

/// `FENCE_WAIT`: ceka, az bude rozhodnuta jedna polozka (`all` = vsechny),
/// nebo do `deadline` (ns od startu, `None` = bez limitu). Polozky se
/// kontroluji v poradi; chyba plotu cekani ukonci. Vraci index rozhodujici
/// polozky (u `all` pri uspechu 0) a pri chybe i jeji index.
pub fn wait(
    items: &[(Arc<Fence>, u64)],
    all: bool,
    deadline: Option<u64>,
) -> Result<usize, (Error, Option<usize>)> {
    debug_assert!(items.len() <= 64);
    let me = sched::current();
    loop {
        if me.kill_requested() {
            return Err((Error::Canceled, None));
        }
        if sched::wait_interrupted(&me) {
            return Err((Error::Interrupted, None));
        }
        let seq = me.prepare_wait();
        // Bitova mapa polozek s registraci (nejvys 64 polozek).
        let mut registered: u64 = 0;
        let mut result = None;
        for (i, (fence, target)) in items.iter().enumerate() {
            match fence.check_or_observe(&me, seq, *target) {
                Some(Ok(())) if !all => {
                    result = Some(Ok(i));
                    break;
                }
                Some(Ok(())) => {}
                Some(Err(e)) => {
                    result = Some(Err((e, Some(i))));
                    break;
                }
                None => registered |= 1 << i,
            }
            super::wait::test_hook::run(&me, super::wait::test_hook::Point::Registered(i));
        }
        if result.is_none() && all && registered == 0 {
            result = Some(Ok(0));
        }
        let mut timer = None;
        if result.is_none() {
            match deadline {
                Some(d) if arch::now_ns() >= d => result = Some(Err((Error::TimedOut, None))),
                Some(d) => timer = Some(sched::add_timer(d, &me, seq)),
                None => {}
            }
        }
        if result.is_some() {
            me.cancel_wait();
        } else {
            super::wait::test_hook::run(&me, super::wait::test_hook::Point::BeforeBlock);
            sched::block();
        }
        for (i, (fence, _)) in items.iter().enumerate() {
            if registered & (1 << i) != 0 {
                fence.remove_thread(&me, seq);
            }
        }
        if let Some(timer) = timer {
            sched::cancel_timer(timer);
        }
        if let Some(result) = result {
            return result;
        }
    }
}
