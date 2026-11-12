//! Futex a ukazatel vlakna (TLS) pro knihovny v userspace (`std`, pthread).
//!
//! Futex je fronta vlaken cekajicich na zmenu u32 v pameti procesu. Klic je
//! virtualni adresa ve vlastnim procesu (sdileni mezi procesy zatim neni).
//! Kontrola hodnoty a zarazeni do fronty probehnou pod zamkem fronty, ktery
//! bere i `FUTEX_WAKE`: probuzeni mezi kontrolou a usnutim se neztrati.

use alloc::collections::VecDeque;

use super::{Args, current_process};
use crate::error::{Error, Result};
use crate::{arch, sched};

/// Adresa futexu: zarovnana na 4 B, v userspace.
fn futex_addr(addr: u64) -> Result<u64> {
    if addr % 4 != 0 || addr == 0 || addr >= abi::vm::USER_MAX {
        return Err(Error::InvalidArgs);
    }
    Ok(addr)
}

/// `FUTEX_WAIT(addr, expected, deadline)`.
pub fn wait(a: Args) -> Result<u64> {
    let addr = futex_addr(a[0])?;
    let expected = a[1] as u32;
    let deadline = (a[2] != abi::syscall::DEADLINE_INFINITE).then_some(a[2]);
    let p = current_process();
    let me = sched::current();
    if me.kill_requested() {
        return Err(Error::Canceled);
    }
    let seq = {
        let mut table = p.futex.lock();
        let mut value = [0u8; 4];
        p.address_space().copy_from_user(addr, &mut value)?;
        if u32::from_le_bytes(value) != expected {
            return Ok(0);
        }
        if deadline.is_some_and(|d| arch::now_ns() >= d) {
            return Err(Error::TimedOut);
        }
        let seq = me.prepare_wait();
        table.entry(addr).or_insert_with(VecDeque::new).push_back((me.clone(), seq));
        seq
    };
    let timer = deadline.map(|d| sched::add_timer(d, &me, seq));
    sched::block();
    if let Some(timer) = timer {
        sched::cancel_timer(timer);
    }
    // Zaznam ve fronte zustal = nikdo nas neprobudil pres FUTEX_WAKE
    // (vyprseni, kill nebo planne probuzeni): uklidit.
    let still_queued = {
        let mut table = p.futex.lock();
        let mut found = false;
        if let Some(queue) = table.get_mut(&addr) {
            if let Some(i) = queue.iter().position(|(t, s)| alloc::sync::Arc::ptr_eq(t, &me) && *s == seq) {
                queue.remove(i);
                found = true;
            }
            if queue.is_empty() {
                table.remove(&addr);
            }
        }
        found
    };
    if !still_queued {
        return Ok(0);
    }
    if me.kill_requested() {
        return Err(Error::Canceled);
    }
    if deadline.is_some_and(|d| arch::now_ns() >= d) {
        return Err(Error::TimedOut);
    }
    Ok(0)
}

/// `FUTEX_WAKE(addr, count) -> pocet probuzenych`.
pub fn wake(a: Args) -> Result<u64> {
    let addr = futex_addr(a[0])?;
    let count = a[1].min(u64::from(u32::MAX)) as usize;
    let p = current_process();
    let mut woken = 0usize;
    let mut table = p.futex.lock();
    if let Some(queue) = table.get_mut(&addr) {
        while woken < count {
            let Some((t, seq)) = queue.pop_front() else { break };
            sched::wake(&t, seq);
            woken += 1;
        }
        if queue.is_empty() {
            table.remove(&addr);
        }
    }
    Ok(woken as u64)
}

/// `THREAD_SET_POINTER(value)`: registr ukazatele vlakna (TLS).
pub fn set_thread_pointer(a: Args) -> Result<u64> {
    let value = a[0];
    // Na x86_64 zpusobi zapis nekanonicke adresy do FS_BASE #GP v jadre;
    // mimo userspace ho nepovolime ani na aarch64 (jednotne chovani).
    if value >= abi::vm::USER_MAX {
        return Err(Error::InvalidArgs);
    }
    sched::set_user_thread_pointer(value);
    Ok(0)
}
