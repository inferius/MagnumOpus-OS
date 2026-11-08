//! Syscally: rozhrani userspace -> jadro.
//!
//! Konvence volani a cisla: crate `abi::syscall`, popis kazdeho volani:
//! `docs/cs/syscally.md`. Kazdy handler vraci `Result<u64>`; chyba se
//! zakoduje jako zaporny kod `abi::Error`. Pameti procesu se tyka jen pres
//! [`user`] (kopie s kontrolou mapovani), nikdy primym ukazatelem.

mod channel;
mod handle;
mod hw;
mod job;
mod misc;
mod object;
mod process;
mod user;
mod vm;

use alloc::sync::Arc;

use abi::syscall as nr;

use crate::error::{Error, Result};
use crate::log;
use crate::object::process::Process;
use crate::sched;

/// Argumenty syscallu (rdi, rsi, rdx, r10, r8, r9).
pub type Args = [u64; 6];

/// Proces aktualniho vlakna (syscall prichazi vzdy z vlakna procesu).
fn current_process() -> Arc<Process> {
    sched::current()
        .process
        .clone()
        .expect("syscall z vlakna jadra")
}

/// Zpracuje syscall `number` a vrati hodnotu pro `rax`.
pub fn dispatch(number: u64, a: Args) -> u64 {
    let result = call(number, a);
    if let Err(e) = result {
        // Ocekavane "chyby" rizeni toku nelogovat (byly by vsude).
        if !matches!(e, Error::ShouldWait | Error::TimedOut) {
            log::debug!("syscall {} ({number}) selhal: {e}", name(number));
        }
    }
    abi::error::encode(result)
}

fn call(number: u64, a: Args) -> Result<u64> {
    match number {
        nr::DEBUG_LOG => misc::debug_log(a),
        nr::CLOCK_MONOTONIC => misc::clock_monotonic(),
        nr::CLOCK_REALTIME => misc::clock_realtime(a),
        nr::HANDLE_CLOSE => handle::close(a),
        nr::HANDLE_DUPLICATE => handle::duplicate(a),
        nr::OBJECT_WAIT_ONE => object::wait_one(a),
        nr::OBJECT_WAIT_MANY => object::wait_many(a),
        nr::OBJECT_SIGNAL => object::signal(a),
        nr::EVENT_CREATE => object::event_create(),
        nr::CHANNEL_CREATE => channel::create(a),
        nr::CHANNEL_WRITE => channel::write(a),
        nr::CHANNEL_READ => channel::read(a),
        nr::VMO_CREATE => vm::vmo_create(a),
        nr::VMO_READ => vm::vmo_read(a),
        nr::VMO_WRITE => vm::vmo_write(a),
        nr::VM_MAP => vm::map(a),
        nr::VM_UNMAP => vm::unmap(a),
        nr::PROCESS_CREATE => process::create(a),
        nr::PROCESS_START => process::start(a),
        nr::PROCESS_EXIT => process::exit(a),
        nr::PROCESS_KILL => process::kill(a),
        nr::PROCESS_EXIT_CODE => process::exit_code(a),
        nr::THREAD_CREATE => process::thread_create(a),
        nr::THREAD_START => process::thread_start(a),
        nr::THREAD_EXIT => process::thread_exit(),
        nr::VMO_CREATE_CONTIGUOUS => hw::vmo_create_contiguous(a),
        nr::VMO_CREATE_PHYSICAL => hw::vmo_create_physical(a),
        nr::VMO_PHYS_ADDR => hw::vmo_phys_addr(a),
        nr::INTERRUPT_CREATE => hw::interrupt_create(a),
        nr::INTERRUPT_ACK => hw::interrupt_ack(a),
        nr::RESOURCE_CREATE => hw::resource_create(a),
        nr::VM_PAGE_SIZE => vm::page_size(a),
        nr::CLOCK_SET => hw::clock_set(a),
        nr::IOPORT_READ => hw::ioport_read(a),
        nr::IOPORT_WRITE => hw::ioport_write(a),
        nr::INTERRUPT_CREATE_LINE => hw::interrupt_create_line(a),
        nr::RANDOM_GET => misc::random_get(a),
        nr::RANDOM_ADD => hw::random_add(a),
        nr::PORT_CREATE => object::port_create(),
        nr::OBJECT_WAIT_ASYNC => object::wait_async(a),
        nr::PORT_WAIT => object::port_wait(a),
        nr::JOB_CREATE => job::create(a),
        nr::JOB_SET_LIMIT => job::set_limit(a),
        nr::JOB_INFO => job::info(a),
        nr::CLOCK_ADJUST => hw::clock_adjust(a),
        nr::FB_CONTROL => hw::fb_control(a),
        nr::SYSTEM_POWER => hw::system_power(a),
        nr::THREAD_YIELD => {
            sched::yield_now();
            Ok(0)
        }
        _ => Err(Error::BadSyscall),
    }
}

/// Jmeno syscallu pro logy.
pub fn name(number: u64) -> &'static str {
    const NAMES: [&str; nr::COUNT as usize] = [
        "DEBUG_LOG",
        "CLOCK_MONOTONIC",
        "HANDLE_CLOSE",
        "HANDLE_DUPLICATE",
        "OBJECT_WAIT_ONE",
        "OBJECT_WAIT_MANY",
        "OBJECT_SIGNAL",
        "EVENT_CREATE",
        "CHANNEL_CREATE",
        "CHANNEL_WRITE",
        "CHANNEL_READ",
        "VMO_CREATE",
        "VMO_READ",
        "VMO_WRITE",
        "VM_MAP",
        "VM_UNMAP",
        "PROCESS_CREATE",
        "PROCESS_START",
        "PROCESS_EXIT",
        "PROCESS_KILL",
        "PROCESS_EXIT_CODE",
        "THREAD_CREATE",
        "THREAD_START",
        "THREAD_EXIT",
        "THREAD_YIELD",
        "VMO_CREATE_CONTIGUOUS",
        "VMO_CREATE_PHYSICAL",
        "VMO_PHYS_ADDR",
        "INTERRUPT_CREATE",
        "INTERRUPT_ACK",
        "RESOURCE_CREATE",
        "VM_PAGE_SIZE",
        "CLOCK_REALTIME",
        "CLOCK_SET",
        "IOPORT_READ",
        "IOPORT_WRITE",
        "INTERRUPT_CREATE_LINE",
        "RANDOM_GET",
        "RANDOM_ADD",
        "PORT_CREATE",
        "OBJECT_WAIT_ASYNC",
        "PORT_WAIT",
        "JOB_CREATE",
        "JOB_SET_LIMIT",
        "JOB_INFO",
        "CLOCK_ADJUST",
        "FB_CONTROL",
        "?",
        "SYSTEM_POWER",
    ];
    NAMES.get(number as usize).copied().unwrap_or("?")
}
