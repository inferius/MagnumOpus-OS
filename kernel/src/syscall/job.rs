//! Joby (rozpocty prostredku).

use super::handle::arg_handle;
use super::{Args, current_process, user};
use crate::error::{Error, Result};
use crate::object::job::{Job, Pool};
use crate::object::{KernelObject, Rights};

/// `JOB_CREATE(parent) -> handle`: podrizeny job bez vlastniho limitu.
pub fn create(a: Args) -> Result<u64> {
    let p = current_process();
    let parent = p.handles().get_as::<Job>(arg_handle(a[0])?, Rights::MANAGE)?;
    let job = Job::new_child(&parent)?;
    let rights = job.default_rights();
    Ok(p.handles().insert(job, rights)?.0 as u64)
}

/// `JOB_SET_LIMIT(job, pool, bytes)`: pravo `WRITE` (potomek ho na svem
/// jobu nema, takze si limit nezvedne).
pub fn set_limit(a: Args) -> Result<u64> {
    let p = current_process();
    let job = p.handles().get_as::<Job>(arg_handle(a[0])?, Rights::WRITE)?;
    let pool = Pool::from_raw(a[1]).ok_or(Error::InvalidArgs)?;
    job.set_limit(pool, a[2]);
    Ok(0)
}

/// `JOB_INFO(job, out_ptr)`: zapise `abi::job::JobInfo` (56 B).
pub fn info(a: Args) -> Result<u64> {
    let p = current_process();
    let job = p.handles().get_as::<Job>(arg_handle(a[0])?, Rights::READ)?;
    user::write_bytes(&p, a[1], &job.info().encode())?;
    Ok(0)
}

/// `JOB_KILL(job, code)`: ukonci vsechny procesy jobu a jeho potomku
/// a job uzavre (novy proces ani podjob uz nevznikne). Pravo `MANAGE`.
/// Volajici v zabijenem jobu skonci take (pozadavek na ukonceni jeho
/// vlaken se vyridi pri navratu do userspace; syscall vrati 0).
pub fn kill(a: Args) -> Result<u64> {
    let p = current_process();
    let job = p.handles().get_as::<Job>(arg_handle(a[0])?, Rights::MANAGE)?;
    // Nejdriv priznak (novy proces se uz nezaregistruje bez kontroly),
    // pak registr: proces vznikly mezi tim ukonci jeho vlastni kontrola.
    job.mark_killed();
    let code = a[1] as i64;
    let mut n = 0u64;
    for proc in crate::object::process::live_processes() {
        if proc.job().within(&job) {
            proc.terminate(code);
            n += 1;
        }
    }
    crate::log::info!("job {} ukoncen ({n} procesu)", job.id);
    Ok(0)
}

/// `JOB_PROCESSES(job, out_ptr, max) -> pocet`: procesy jobu a jeho
/// potomku (`abi::job::ProcEntry`, 64 B kazdy). Vrati celkovy pocet
/// (zapise nejvys `max`). Pravo `READ`.
pub fn processes(a: Args) -> Result<u64> {
    let p = current_process();
    let job = p.handles().get_as::<Job>(arg_handle(a[0])?, Rights::READ)?;
    let max = a[2] as usize;
    if max > 4096 {
        return Err(Error::InvalidArgs);
    }
    let mut n = 0u64;
    let mut off = 0u64;
    for proc in crate::object::process::live_processes() {
        if !proc.job().within(&job) {
            continue;
        }
        if (n as usize) < max {
            let (threads, finished) = proc.thread_stats();
            let e = abi::job::ProcEntry {
                pid: proc.id,
                job: proc.job().id,
                cpu_ns: proc.cpu_ns.load(core::sync::atomic::Ordering::Relaxed)
                    + crate::sched::running_ns(proc.id),
                threads,
                flags: if finished { abi::job::proc_flags::FINISHED } else { 0 },
                name: abi::job::ProcEntry::name_bytes(&proc.name),
            };
            user::write_bytes(&p, a[1] + off, &e.encode())?;
            off += abi::job::ProcEntry::SIZE as u64;
        }
        n += 1;
    }
    Ok(n)
}
