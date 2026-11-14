//! Diagnosticky vypis stavu planovace: fronty a casovace vsech CPU a stav
//! vybranych vlaken. Pro selhane testy (vlakno, na ktere se ceka, nebezi)
//! a ladeni uvaznuti.
//!
//! Cte cizi CPU bez zastaveni, takze vypis neni atomicky snimek: hodnoty
//! z ruznych CPU se mohou o par us lisit. Zamky front a casovacu bere jen
//! pres `try_lock` (vypis nesmi uvaznout na zamku, ktery drzi zaseknute CPU).

use alloc::string::String;
use alloc::sync::Arc;
use core::fmt::Write;
use core::sync::atomic::Ordering;

use super::percpu::all_cpus;
use super::thread::{Thread, cpu_flag, state};
use crate::arch;
use crate::log;

/// Pokusy o zamek cizi fronty, nez ho vypis vzda.
const LOCK_TRIES: usize = 100_000;

/// Vypise stav planovace do logu (uroven error) a stav vlaken `watch`.
pub fn dump_state(watch: &[Arc<Thread>]) {
    let now = arch::now_ns();
    log::error!("planovac: stav v {} us", now / 1000);
    for cpu in all_cpus() {
        let mut line = String::new();
        let _ = write!(
            line,
            "  cpu{}: bezi #{} qos {} idle {} need_resched {} prepnuti {} od posledniho {} us",
            cpu.index,
            cpu.current_tid.load(Ordering::Relaxed),
            cpu.current_qos.load(Ordering::Relaxed),
            cpu.is_idle.load(Ordering::Relaxed),
            cpu.need_resched.load(Ordering::Relaxed),
            cpu.switches.load(Ordering::Relaxed),
            now.saturating_sub(cpu.run_start.load(Ordering::Relaxed)) / 1000,
        );
        let slice_end = cpu.slice_end.load(Ordering::Relaxed);
        if slice_end != u64::MAX {
            let _ = write!(line, " kvantum do {} us", slice_end as i64 / 1000 - now as i64 / 1000);
        }
        let _ = write!(line, " | fronta:");
        match try_lock(|| cpu.rq.try_lock()) {
            Some(rq) => {
                if rq.is_empty() {
                    let _ = write!(line, " prazdna");
                }
                for t in rq.iter() {
                    let _ = write!(line, " {}#{}", t.name, t.id);
                }
            }
            None => {
                let _ = write!(line, " (zamcena)");
            }
        }
        let _ = write!(line, " | casovace:");
        match try_lock(|| cpu.timers.try_lock()) {
            Some(timers) => {
                let _ = write!(line, " {}", timers.len());
                if let Some(e) = timers.peek() {
                    let _ = write!(
                        line,
                        ", nejblizsi {}#{} za {} us",
                        e.thread.name,
                        e.thread.id,
                        e.deadline as i64 / 1000 - now as i64 / 1000
                    );
                }
            }
            None => {
                let _ = write!(line, " (zamcene)");
            }
        }
        log::error!("{line}");
    }
    for t in watch {
        log::error!("  {}", describe(t));
    }
}

/// Jednoradkovy popis vlakna pro vypis.
fn describe(t: &Thread) -> String {
    let st = match t.state() {
        state::RUNNING => "RUNNING",
        state::RUNNABLE => "RUNNABLE",
        state::BLOCKING => "BLOCKING",
        state::BLOCKED => "BLOCKED",
        state::DEAD => "DEAD",
        _ => "?",
    };
    let mut s = String::new();
    let _ = write!(
        s,
        "vlakno {}#{}: {st} on_cpu {} wake_pending {} last_cpu {} wait_seq {}",
        t.name,
        t.id,
        t.on_cpu(),
        t.cpu_flags.load(Ordering::Relaxed) & cpu_flag::WAKE_PENDING != 0,
        t.last_cpu.load(Ordering::Relaxed),
        t.wait_seq(),
    );
    s
}

fn try_lock<G>(mut f: impl FnMut() -> Option<G>) -> Option<G> {
    for _ in 0..LOCK_TRIES {
        if let Some(g) = f() {
            return Some(g);
        }
        core::hint::spin_loop();
    }
    None
}
