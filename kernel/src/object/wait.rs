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
        let seq = me.prepare_wait();
        let mut registered = 0;
        let mut result = None;
        for (i, (object, mask)) in items.iter().enumerate() {
            if let Some(signals) = object.core().check_or_observe(&me, seq, *mask) {
                result = Some(Ok((i, signals)));
                break;
            }
            registered = i + 1;
        }
        if result.is_none() {
            match deadline {
                Some(d) if arch::now_ns() >= d => result = Some(Err(Error::TimedOut)),
                Some(d) => sched::add_timer(d, &me, seq),
                None => {}
            }
        }
        if result.is_some() {
            me.cancel_wait();
        } else {
            sched::block();
        }
        for (object, _) in &items[..registered] {
            object.core().remove_observer(&me, seq);
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
