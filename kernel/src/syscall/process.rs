//! Procesy a vlakna.

use super::handle::arg_handle;
use super::{Args, current_process, user};
use crate::error::{Error, Result};
use crate::object::job::Job;
use crate::object::process::Process;
use crate::object::{KernelObject, Rights};
use crate::sched::{self, Thread};

/// `PROCESS_CREATE(job, name_ptr, name_len) -> handle`: prazdny proces
/// v jobu `job` (pravo `MANAGE`).
pub fn create(a: Args) -> Result<u64> {
    let p = current_process();
    let job = p.handles().get_as::<Job>(arg_handle(a[0])?, Rights::MANAGE)?;
    let name = user::read_name(&p, a[1], a[2])?;
    if !crate::mm::phys::user_room(8) {
        return Err(Error::NoMemory);
    }
    let child = Process::new(&name, job)?;
    let rights = child.default_rights();
    Ok(p.handles().insert(child, rights)?.0 as u64)
}

/// `PROCESS_START(process, thread, entry, stack, bootstrap_handle)`.
///
/// Spusti prvni vlakno procesu. `bootstrap_handle` (handle volajiciho,
/// typicky konec kanalu) se presune do tabulky noveho procesu a jeho nova
/// hodnota prijde vlaknu v `rdi`.
pub fn start(a: Args) -> Result<u64> {
    let p = current_process();
    let (child, thread, bootstrap) = {
        let mut table = p.handles();
        let child = table.get_as::<Process>(arg_handle(a[0])?, Rights::MANAGE)?;
        let thread = table.get_as::<Thread>(arg_handle(a[1])?, Rights::MANAGE)?;
        if !thread
            .process
            .as_ref()
            .is_some_and(|tp| core::ptr::eq(&**tp, &*child))
        {
            return Err(Error::InvalidArgs);
        }
        if child.is_started() {
            return Err(Error::BadState);
        }
        table.check(arg_handle(a[4])?, Rights::TRANSFER)?;
        let bootstrap = table.take(arg_handle(a[4])?)?;
        (child, thread, bootstrap)
    };
    let child_handle = child.handles().insert(bootstrap.0, bootstrap.1)?;
    sched::start_user_thread(&thread, true, a[2], a[3], child_handle.0 as u64, 0)?;
    Ok(0)
}

/// `PROCESS_EXIT(code)`: ukonci aktualni proces; nevraci se.
pub fn exit(a: Args) -> Result<u64> {
    current_process().terminate(a[0] as i64);
    // Vlakno skonci pri navratu do userspace (pozadavek na ukonceni).
    Ok(0)
}

/// `PROCESS_KILL(process)`.
pub fn kill(a: Args) -> Result<u64> {
    let p = current_process();
    let target = p
        .handles()
        .get_as::<Process>(arg_handle(a[0])?, Rights::MANAGE)?;
    target.terminate(abi::process::EXIT_KILLED);
    Ok(0)
}

/// `PROCESS_EXIT_CODE(process, out_code_ptr)`: BAD_STATE, dokud proces bezi.
pub fn exit_code(a: Args) -> Result<u64> {
    let p = current_process();
    let target = p
        .handles()
        .get_as::<Process>(arg_handle(a[0])?, Rights::READ)?;
    let code = target.exit_code().ok_or(Error::BadState)?;
    user::write_u64(&p, a[1], code as u64)?;
    Ok(0)
}

/// `THREAD_CREATE(process, name_ptr, name_len) -> handle`.
pub fn thread_create(a: Args) -> Result<u64> {
    let p = current_process();
    let name = user::read_name(&p, a[1], a[2])?;
    let target = p
        .handles()
        .get_as::<Process>(arg_handle(a[0])?, Rights::MANAGE)?;
    let thread = sched::create_user_thread(&target, &name)?;
    let rights = thread.default_rights();
    Ok(p.handles().insert(thread, rights)?.0 as u64)
}

/// `THREAD_START(thread, entry, stack, arg0, arg1)`: dalsi vlakno bezici
/// procesu.
pub fn thread_start(a: Args) -> Result<u64> {
    let p = current_process();
    let thread = p
        .handles()
        .get_as::<Thread>(arg_handle(a[0])?, Rights::MANAGE)?;
    sched::start_user_thread(&thread, false, a[1], a[2], a[3], a[4])?;
    Ok(0)
}

/// `THREAD_SUSPEND(thread) -> token`: pozastavi vlakno (pravo `MANAGE`);
/// zavreni tokenu ho obnovi.
pub fn thread_suspend(a: Args) -> Result<u64> {
    let p = current_process();
    let thread = p
        .handles()
        .get_as::<Thread>(arg_handle(a[0])?, Rights::MANAGE)?;
    let token = sched::suspend::SuspendToken::new(thread)?;
    let rights = token.default_rights();
    Ok(p.handles().insert(token, rights)?.0 as u64)
}

/// `THREAD_STATE(thread, op, kind, ptr, len)`: cteni (`READ`) nebo zapis
/// (`WRITE`) kontextu pozastaveneho vlakna (`abi::thread`).
pub fn thread_state(a: Args) -> Result<u64> {
    let p = current_process();
    let (op, kind) = (
        u32::try_from(a[1]).map_err(|_| Error::InvalidArgs)?,
        u32::try_from(a[2]).map_err(|_| Error::InvalidArgs)?,
    );
    let size = sched::suspend::state_size(kind)?;
    if a[4] != size as u64 {
        return Err(Error::InvalidArgs);
    }
    match op {
        abi::thread::op::READ => {
            let thread = p.handles().get_as::<Thread>(arg_handle(a[0])?, Rights::READ)?;
            let mut buf = alloc::vec![0u8; size];
            sched::suspend::read_state(&thread, kind, &mut buf)?;
            user::write_bytes(&p, a[3], &buf)?;
        }
        abi::thread::op::WRITE => {
            let thread = p.handles().get_as::<Thread>(arg_handle(a[0])?, Rights::WRITE)?;
            let data = user::read_bytes(&p, a[3], size)?;
            sched::suspend::write_state(&thread, kind, &data)?;
        }
        _ => return Err(Error::InvalidArgs),
    }
    Ok(0)
}

/// `THREAD_EXIT()`: nevraci se.
pub fn thread_exit() -> Result<u64> {
    sched::exit()
}
