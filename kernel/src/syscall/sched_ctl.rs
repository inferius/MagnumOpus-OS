//! `SCHED_CONTROL`: tridy QoS, strop jobu, Realtime rozpocet, udaje o CPU.
//! Popis: `docs/cs/syscally.md`, chovani: `docs/cs/planovac.md`.

use alloc::sync::Arc;

use abi::sched::op;
use schedcore::Class;

use super::handle::arg_handle;
use super::{Args, current_process, user};
use crate::error::{Error, Result};
use crate::object::Rights;
use crate::object::job::Job;
use crate::object::process::Process;
use crate::object::resource::Resource;
use crate::sched::{self, Thread};

pub fn control(a: Args) -> Result<u64> {
    match a[0] {
        op::THREAD_SET => thread_set(a),
        op::PROCESS_SET => process_set(a),
        op::JOB_SET_MAX => job_set_max(a),
        op::THREAD_GET => thread_get(a),
        op::CPU_INFO => cpu_info(a),
        op::CPU_HINT => cpu_hint(a),
        _ => Err(Error::InvalidArgs),
    }
}

fn class_arg(v: u64) -> Result<Class> {
    u8::try_from(v).ok().and_then(Class::from_raw).ok_or(Error::InvalidArgs)
}

/// Trida a Realtime parametry (us -> ns) z argumentu `a[2..5]`.
fn qos_args(a: &Args) -> Result<(Class, Option<(u64, u64)>)> {
    let class = class_arg(a[2])?;
    if class != Class::Realtime {
        return Ok((class, None));
    }
    let budget = u32::try_from(a[3]).map_err(|_| Error::InvalidArgs)?;
    let period = u32::try_from(a[4]).map_err(|_| Error::InvalidArgs)?;
    let rt = schedcore::rt::params_from_us(budget, period).ok_or(Error::InvalidArgs)?;
    Ok((class, Some(rt)))
}

/// Vlakno z handle (0 = volajici vlakno).
fn thread_arg(p: &Process, h: u64, rights: Rights) -> Result<Arc<Thread>> {
    if h == 0 {
        return Ok(sched::current());
    }
    p.handles().get_as::<Thread>(arg_handle(h)?, rights)
}

fn thread_set(a: Args) -> Result<u64> {
    let p = current_process();
    let t = thread_arg(&p, a[1], Rights::MANAGE)?;
    // Jen vlakna procesu (vlakna jadra se do userspace nedostanou, ale
    // pro jistotu).
    if t.process.is_none() {
        return Err(Error::AccessDenied);
    }
    let (class, rt) = qos_args(&a)?;
    sched::set_thread_qos(&t, class, rt).map(|_| 0)
}

fn process_set(a: Args) -> Result<u64> {
    let p = current_process();
    let target = p.handles().get_as::<Process>(arg_handle(a[1])?, Rights::MANAGE)?;
    let (class, rt) = qos_args(&a)?;
    if class.above(target.job().qos_cap()) {
        return Err(Error::AccessDenied);
    }
    target.set_default_qos(class, rt);
    // Kazde vlakno ma vlastni Realtime rozpocet; prvni chyba (kapacita)
    // se vrati, ostatni vlakna uz nastavena zustanou.
    let mut result = Ok(0);
    for t in target.threads() {
        if let Err(e) = sched::set_thread_qos(&t, class, rt)
            && result.is_ok()
        {
            result = Err(e);
        }
    }
    result
}

fn job_set_max(a: Args) -> Result<u64> {
    let p = current_process();
    let job = p.handles().get_as::<Job>(arg_handle(a[1])?, Rights::WRITE)?;
    job.set_qos_max(class_arg(a[2])?)?;
    Ok(0)
}

fn thread_get(a: Args) -> Result<u64> {
    let p = current_process();
    let t = thread_arg(&p, a[1], Rights::READ)?;
    user::write_bytes(&p, a[2], &sched::thread_info(&t).encode())?;
    Ok(0)
}

fn cpu_info(a: Args) -> Result<u64> {
    let p = current_process();
    let index = usize::try_from(a[1]).map_err(|_| Error::InvalidArgs)?;
    let info = sched::topo::cpu_info(index).ok_or(Error::InvalidArgs)?;
    user::write_bytes(&p, a[2], &info.encode())?;
    Ok(sched::cpu_count() as u64)
}

fn cpu_hint(a: Args) -> Result<u64> {
    let p = current_process();
    p.handles().get_as::<Resource>(arg_handle(a[1])?, Rights::POWER)?;
    let index = usize::try_from(a[2]).map_err(|_| Error::InvalidArgs)?;
    let cap = u32::try_from(a[3]).map_err(|_| Error::InvalidArgs)?;
    sched::topo::set_hint(index, cap).map(|_| 0)
}
