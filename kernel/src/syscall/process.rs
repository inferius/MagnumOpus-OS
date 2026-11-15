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

/// Objekt typu `T` za handlem, ktery ma aspon jedno z prav `any` (uzke
/// pravo jako `KILL` nebo siroke `MANAGE`).
fn get_with_any<T: KernelObject>(p: &Process, handle: u64, any: Rights) -> Result<alloc::sync::Arc<T>> {
    let h = arg_handle(handle)?;
    let table = p.handles();
    if table.rights(h)?.0 & any.0 == 0 {
        return Err(Error::AccessDenied);
    }
    table.get_as::<T>(h, Rights::NONE)
}

/// `THREAD_SELF(rights) -> handle`: handle volajiciho vlakna s pravy
/// `rights` (podmnozina vychozich, 0 = vychozi).
pub fn thread_self(a: Args) -> Result<u64> {
    let p = current_process();
    let me = sched::current();
    let default = me.default_rights();
    let rights = if a[0] == 0 { default } else { Rights(u32::try_from(a[0]).map_err(|_| Error::InvalidArgs)?) };
    if !default.contains(rights) {
        return Err(Error::AccessDenied);
    }
    Ok(p.handles().insert(me, rights)?.0 as u64)
}

/// `PROCESS_KILL(process)`: pravo `KILL` nebo `MANAGE`.
pub fn kill(a: Args) -> Result<u64> {
    let p = current_process();
    let target = get_with_any::<Process>(&p, a[0], Rights::KILL | Rights::MANAGE)?;
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

/// `THREAD_INTERRUPT(thread, code)`: cekajici preruseni vlakna (pravo
/// `MANAGE`); doruceni na obsluhu ve vlakne pri navratu do userspace.
pub fn thread_interrupt(a: Args) -> Result<u64> {
    let p = current_process();
    let thread = get_with_any::<Thread>(&p, a[0], Rights::INTERRUPT | Rights::MANAGE)?;
    if a[1] >= abi::syscall::INTERRUPT_CODES {
        return Err(Error::InvalidArgs);
    }
    // Bez obsluhy ve vlakne by preruseni viselo a cekani vlakna by
    // porad vracela INTERRUPTED.
    let Some(target) = thread.process.as_ref() else { return Err(Error::WrongType) };
    if target.exception_entry() == 0 {
        return Err(Error::BadState);
    }
    sched::interrupt(&thread, a[1] as u32);
    Ok(0)
}

/// Velikost bloku kopie `PROCESS_MEMORY` (buffer jadra).
const MEMORY_CHUNK: usize = 64 * 1024;

/// `PROCESS_MEMORY(process, op, addr, buf, len) -> bajty`: kopie mezi
/// pameti volajiciho a jineho procesu pres buffer jadra po blocich (zadny
/// zamek dvou adresnich prostoru najednou). Chyba v bufferu volajiciho =
/// chyba; chyba v cili po prvnim bloku = castecna kopie.
pub fn process_memory(a: Args) -> Result<u64> {
    use abi::process::memory;
    let p = current_process();
    let need = match a[1] {
        memory::READ => Rights::READ_MEMORY,
        memory::WRITE => Rights::WRITE_MEMORY,
        _ => return Err(Error::InvalidArgs),
    };
    let target = p.handles().get_as::<Process>(arg_handle(a[0])?, need)?;
    let (addr, buf) = (a[2], a[3]);
    let len = usize::try_from(a[4]).map_err(|_| Error::InvalidArgs)?;
    if addr.checked_add(a[4]).is_none() || buf.checked_add(a[4]).is_none() {
        return Err(Error::BadAddress);
    }
    if target.address_space().root().is_none() {
        // Skonceny proces (prostor zrusen).
        return Err(Error::BadState);
    }
    let mut chunk = alloc::vec::Vec::new();
    chunk.try_reserve_exact(len.min(MEMORY_CHUNK)).map_err(|_| Error::NoMemory)?;
    chunk.resize(len.min(MEMORY_CHUNK), 0);
    let mut done = 0usize;
    while done < len {
        let n = (len - done).min(MEMORY_CHUNK);
        let k = &mut chunk[..n];
        let at = done as u64;
        let r = if a[1] == memory::READ {
            match target.address_space().copy_from_user(addr + at, k) {
                Ok(()) => p.address_space().copy_to_user(buf + at, k).map_err(|e| (e, true)),
                Err(e) => Err((e, false)),
            }
        } else {
            match p.address_space().copy_from_user(buf + at, k) {
                Ok(()) => target.address_space().copy_to_user(addr + at, k).map_err(|e| (e, false)),
                Err(e) => Err((e, true)),
            }
        };
        match r {
            Ok(()) => done += n,
            // Buffer volajiciho je vzdy chyba; cil po prvnim bloku = castecne.
            Err((e, true)) => return Err(e),
            Err((e, false)) if done == 0 => return Err(e),
            Err(_) => break,
        }
    }
    Ok(done as u64)
}

/// `THREAD_STATE(thread, op, kind, ptr, len)`: cteni (`READ`) nebo zapis
/// (`WRITE`) kontextu pozastaveneho vlakna (`abi::thread`).
pub fn thread_state(a: Args) -> Result<u64> {
    let p = current_process();
    let (op, kind) = (
        u32::try_from(a[1]).map_err(|_| Error::InvalidArgs)?,
        u32::try_from(a[2]).map_err(|_| Error::InvalidArgs)?,
    );
    // Velikost stavu FPU zavisi na CPU (hlavicka `abi::thread::fp`):
    // vetsi buffer se pouzije jen do velikosti stavu, mensi = chyba.
    let size = sched::suspend::state_size(kind)?;
    if a[4] < size as u64 {
        // Cteni FPU s bufferem aspon na hlavicku: jadro do ni zapise
        // potrebnou delku (program si pak alokuje spravny buffer).
        let hdr = abi::thread::fp::HEADER;
        if op == abi::thread::op::READ && kind == abi::thread::kind::FP && a[4] >= hdr as u64 {
            p.handles().get_as::<Thread>(arg_handle(a[0])?, Rights::READ)?;
            user::write_bytes(&p, a[3], &crate::arch::ustate::fp_header().encode())?;
        }
        return Err(Error::BufferTooSmall);
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
                let min = crate::arch::ustate::frame_size() as u64 + 256;
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
