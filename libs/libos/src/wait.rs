//! Cekani na signaly objektu.

use alloc::vec::Vec;

use abi::object::WaitItem;
use abi::syscall::DEADLINE_INFINITE;
use abi::{Result, Signals};

use crate::{Handle, sys};

/// Ceka na signaly objektu; `deadline` v ns od startu (viz [`crate::time`]).
/// Vraci signaly objektu.
pub fn wait_one(h: &Handle, mask: Signals, deadline: Option<u64>) -> Result<Signals> {
    let (r, observed) =
        sys::object_wait_one(h.raw(), mask.0, deadline.unwrap_or(DEADLINE_INFINITE));
    r.map(|_| Signals(observed))
}

/// Ceka na vice objektu; vraci index prvniho splneneho a signaly vsech.
pub fn wait_many(
    items: &[(&Handle, Signals)],
    deadline: Option<u64>,
) -> Result<(usize, Vec<Signals>)> {
    let mut raw: Vec<WaitItem> = items
        .iter()
        .map(|(h, m)| WaitItem {
            handle: h.raw(),
            mask: m.0,
            observed: 0,
        })
        .collect();
    let index = sys::object_wait_many(&mut raw, deadline.unwrap_or(DEADLINE_INFINITE))?;
    Ok((index, raw.iter().map(|w| Signals(w.observed)).collect()))
}
