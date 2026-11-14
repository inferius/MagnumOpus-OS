//! Cekani na signaly objektu.

use core::time::Duration;

use super::{Error, KernelObject, Signals};
use crate::{arch, sched};

/// Ceka, dokud signaly nektereho objektu neprotnou jeho masku, nebo do
/// `deadline` (ns od startu, `None` = bez limitu). Vraci index objektu
/// a jeho signaly.
///
/// Vlakno se zaregistruje u vsech objektu najednou; probuzeni muze byt
/// i planne, proto se podminky po kazdem probuzeni overi znovu.
pub fn wait_many(
    items: &[(&dyn KernelObject, Signals)],
    deadline: Option<u64>,
) -> Result<(usize, Signals), Error> {
    let me = sched::current();
    loop {
        // Ukoncovane vlakno nesmi dal cekat (napr. kill procesu).
        if me.kill_requested() {
            return Err(Error::Canceled);
        }
        let seq = me.prepare_wait();
        let mut registered = 0;
        let mut result = None;
        for (i, (object, mask)) in items.iter().enumerate() {
            if let Some(signals) = object.core().check_or_observe(&me, seq, *mask) {
                result = Some(Ok((i, signals)));
                break;
            }
            registered = i + 1;
            test_hook::run(&me, test_hook::Point::Registered(i));
        }
        let mut timer = None;
        if result.is_none() {
            match deadline {
                Some(d) if arch::now_ns() >= d => result = Some(Err(Error::TimedOut)),
                Some(d) => timer = Some(sched::add_timer(d, &me, seq)),
                None => {}
            }
        }
        if result.is_some() {
            me.cancel_wait();
        } else {
            test_hook::run(&me, test_hook::Point::BeforeBlock);
            sched::block();
        }
        // Uklid registraci: pozorovatele u objektu i casovac (kdyz cekani
        // skoncilo driv, zaznam by jinak drzel vlakno do terminu).
        for (object, _) in &items[..registered] {
            object.core().remove_observer(&me, seq);
        }
        if let Some(timer) = timer {
            sched::cancel_timer(timer);
        }
        if let Some(result) = result {
            return result;
        }
    }
}

pub fn wait_one(
    object: &dyn KernelObject,
    mask: Signals,
    deadline: Option<u64>,
) -> Result<Signals, Error> {
    wait_many(&[(object, mask)], deadline).map(|(_, s)| s)
}

/// Uspi aktualni vlakno alespon na `duration`.
pub fn sleep(duration: Duration) {
    let deadline = arch::now_ns() + duration.as_nanos() as u64;
    let _ = wait_many(&[], Some(deadline));
}

/// Deadline za `duration` od ted.
pub fn deadline_after(duration: Duration) -> u64 {
    arch::now_ns() + duration.as_nanos() as u64
}

/// Hacek pro self-testy jadra (bez nastaveni stoji jedno atomicke cteni):
/// zavola se v urcenych mistech `wait_many` jen pro
/// jedno vybrane vlakno, takze test muze deterministicky poslat signal
/// "mezi kontrolou a usnutim" (i z jineho CPU).
pub mod test_hook {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicU64, Ordering};

    use crate::sched::Thread;
    use crate::sync::SpinLock;

    /// Misto v `wait_many`.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Point {
        /// Polozka `i` je zaregistrovana (dalsi jeste ne).
        Registered(usize),
        /// Vse zaregistrovane (i casovac), vlakno je BLOCKING a jde usnout.
        BeforeBlock,
    }

    type Hook = Arc<dyn Fn(Point) + Send + Sync>;

    /// Id vlakna, pro ktere hacek plati (0 = zadne).
    static TARGET: AtomicU64 = AtomicU64::new(0);
    static HOOK: SpinLock<Option<Hook>> = SpinLock::new(None);

    /// Nastavi hacek pro vlakno `thread_id` (`None` = zrusit).
    pub fn set(thread_id: u64, hook: Option<Hook>) {
        TARGET.store(0, Ordering::SeqCst);
        let on = hook.is_some();
        *HOOK.lock() = hook;
        if on {
            TARGET.store(thread_id, Ordering::SeqCst);
        }
    }

    pub(in crate::object) fn run(me: &Thread, point: Point) {
        if TARGET.load(Ordering::Relaxed) != me.id {
            return;
        }
        // Hacek se vola bez zamku (muze cekat na jine vlakno).
        let hook = HOOK.lock().clone();
        if let Some(h) = hook {
            h(point);
        }
    }
}
