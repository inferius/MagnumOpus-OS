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

/// `JOB_INFO(job, out_ptr)`: zapise `abi::job::JobInfo` (48 B).
pub fn info(a: Args) -> Result<u64> {
    let p = current_process();
    let job = p.handles().get_as::<Job>(arg_handle(a[0])?, Rights::READ)?;
    user::write_bytes(&p, a[1], &job.info().encode())?;
    Ok(0)
}
