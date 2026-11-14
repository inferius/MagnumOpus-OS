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

/// `EXCEPTION_CHANNEL(task, flags) -> channel`: kanal vyjimek procesu nebo
/// vlakna (pravo `MANAGE`); `HANDLER_THREAD` na vlakne ho jen oznaci.
pub fn exception_channel(a: Args) -> Result<u64> {
    use abi::exception::channel::{DEBUGGER, HANDLER_THREAD};
    let p = current_process();
    let flags = u32::try_from(a[1]).map_err(|_| Error::InvalidArgs)?;
    if flags & !(DEBUGGER | HANDLER_THREAD) != 0 || flags == DEBUGGER | HANDLER_THREAD {
        return Err(Error::InvalidArgs);
    }
    let object = p.handles().get(arg_handle(a[0])?, Rights::MANAGE)?;
    let any: alloc::sync::Arc<dyn core::any::Any + Send + Sync> = object;
    let (kernel_end, user_end) = crate::object::channel::Channel::create();
    match any.downcast::<Process>() {
        Ok(target) => {
            if flags & HANDLER_THREAD != 0 {
                return Err(Error::InvalidArgs);
            }
            target.set_exception_channel(flags & DEBUGGER != 0, kernel_end)?;
        }
        Err(any) => {
            let thread = any.downcast::<Thread>().map_err(|_| Error::WrongType)?;
            if flags & DEBUGGER != 0 {
                return Err(Error::InvalidArgs);
            }
            if flags & HANDLER_THREAD != 0 {
                sched::suspend::mark_handler(&thread)?;
                return Ok(0);
            }
            sched::suspend::set_exception_channel(&thread, kernel_end)?;
        }
    }
    let rights = user_end.default_rights();
    Ok(p.handles().insert(user_end, rights)?.0 as u64)
}

/// `EXCEPTION_HANDLER(op, a0, a1)`: doruceni vyjimek primo ve vlaknu.
pub fn exception_handler(a: Args) -> Result<u64> {
    use abi::exception::handler::{RETURN, SET_ENTRY, SET_STACK};
    let op = u32::try_from(a[0]).map_err(|_| Error::InvalidArgs)?;
    match op {
        SET_ENTRY => {
            let p = current_process();
            let target = p.handles().get_as::<Process>(arg_handle(a[1])?, Rights::WRITE)?;
            target.set_exception_entry(a[2])?;
            Ok(0)
        }
        SET_STACK => {
            let me = sched::current();
            if me.process.is_none() {
                return Err(Error::WrongType);
            }
            let (base, size) = (a[1], a[2]);
            if size != 0 {
                let end = base.checked_add(size).ok_or(Error::InvalidArgs)?;
                let min = crate::arch::ustate::FRAME_SIZE as u64 + 256;
                if base < abi::vm::USER_MIN || end > abi::vm::USER_MAX || size < min {
                    return Err(Error::InvalidArgs);
                }
            }
            sched::suspend::set_alt_stack(&me, base, size);
            Ok(0)
        }
        RETURN => crate::object::exception::handler_return(a[1], u32::try_from(a[2]).map_err(|_| Error::InvalidArgs)?),
        _ => Err(Error::InvalidArgs),
    }
}

/// `EXCEPTION_RESOLVE(token, action)`.
pub fn exception_resolve(a: Args) -> Result<u64> {
    let p = current_process();
    let token = p
        .handles()
        .get_as::<crate::object::exception::ExceptionToken>(arg_handle(a[0])?, Rights::NONE)?;
    token.resolve(u32::try_from(a[1]).map_err(|_| Error::InvalidArgs)?)?;
    Ok(0)
}

/// `PROCESS_THREADS(process, out_ptr, max) -> pocet`: handly zivych vlaken
/// (pravo `READ`; s `WRITE` i zapis kontextu a pozastaveni).
pub fn process_threads(a: Args) -> Result<u64> {
    let p = current_process();
    let (target, rights) = {
        let t = p.handles();
        let h = arg_handle(a[0])?;
        (t.get_as::<Process>(h, Rights::READ)?, t.rights(h)?)
    };
    let max = usize::try_from(a[2]).map_err(|_| Error::InvalidArgs)?;
    if max > 4096 {
        return Err(Error::InvalidArgs);
    }
    let thread_rights = if rights.contains(Rights::WRITE) {
        Rights::BASIC | Rights::READ | Rights::WRITE | Rights::MANAGE
    } else {
        Rights::BASIC | Rights::READ
    };
    let threads = target.threads();
    let mut out = alloc::vec::Vec::new();
    for t in threads.iter().take(max) {
        let h = p.handles().insert(t.clone(), thread_rights)?;
        out.extend_from_slice(&h.0.to_le_bytes());
    }
    if let Err(e) = user::write_bytes(&p, a[1], &out) {
        // Handly, ktere program neuvidi, zavrit.
        for c in out.chunks(4) {
            let _ = p.handles().close(crate::object::Handle(u32::from_le_bytes(c.try_into().unwrap())));
        }
        return Err(e);
    }
    Ok(threads.len() as u64)
}

/// `THREAD_EXIT()`: nevraci se.
pub fn thread_exit() -> Result<u64> {
    sched::exit()
}
