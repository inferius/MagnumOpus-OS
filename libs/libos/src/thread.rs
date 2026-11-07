//! Vlakna ve vlastnim procesu: `spawn(f)` s vlastnim zasobnikem.
//!
//! Zasobnik je objekt pameti namapovany do vlastniho procesu; po `join`
//! se odmapuje. Heap je sdileny (alokator se zamkem), takze vlakno muze
//! alokovat. Panika ve vlakne ukonci cely proces (stejne jako v hlavnim).

use alloc::boxed::Box;

use abi::Result;
use abi::vm::{MAP_READ, MAP_WRITE};

use crate::{Process, Thread, Vmo, rt, sys};

/// Velikost zasobniku vlakna (interpret shellu smi pouzit ~160 KB).
pub const STACK_SIZE: u64 = 256 * 1024;

type Job = Box<dyn FnOnce() + Send + 'static>;

/// Spustene vlakno; `join` pocka na konec a uvolni zasobnik.
pub struct JoinHandle {
    thread: Thread,
    stack_addr: u64,
    stack_len: u64,
}

impl JoinHandle {
    /// Pocka na konec vlakna (`deadline` v ns od startu, `None` = navzdy).
    pub fn join(self, deadline: Option<u64>) -> Result<()> {
        self.thread.wait_exit(deadline)?;
        let me = Process::from_handle(crate::Handle::from_raw(rt::self_process()));
        let r = me.unmap(self.stack_addr, self.stack_len);
        // Handle na vlastni proces patri behovemu prostredi: nezavirat.
        core::mem::forget(me);
        r
    }

    pub fn thread(&self) -> &Thread {
        &self.thread
    }
}

extern "C" fn trampoline(job: u64, _unused: u64) -> ! {
    // SAFETY: `job` je ukazatel z `Box::into_raw` ve `spawn`, predany
    // prave jednou tomuto vlaknu.
    let f = unsafe { Box::from_raw(job as *mut Job) };
    f();
    sys::thread_exit()
}

/// Spusti `f` ve novem vlakne vlastniho procesu.
pub fn spawn(name: &str, f: impl FnOnce() + Send + 'static) -> Result<JoinHandle> {
    let me = Process::from_handle(crate::Handle::from_raw(rt::self_process()));
    let r = spawn_in(&me, name, f);
    core::mem::forget(me);
    r
}

fn spawn_in(me: &Process, name: &str, f: impl FnOnce() + Send + 'static) -> Result<JoinHandle> {
    let len = STACK_SIZE.next_multiple_of(crate::vm::page_size());
    let stack = Vmo::create(len)?;
    let addr = me.map(&stack, 0, len, MAP_READ | MAP_WRITE, 0)?;
    let thread = match Thread::create(me, name) {
        Ok(t) => t,
        Err(e) => {
            let _ = me.unmap(addr, len);
            return Err(e);
        }
    };
    // x86_64: rsp % 16 == 8 jako po instrukci call; aarch64: sp % 16 == 0.
    let top = if cfg!(target_arch = "x86_64") { addr + len - 8 } else { addr + len };
    let job: Box<Job> = Box::new(Box::new(f));
    let raw = Box::into_raw(job);
    if let Err(e) = thread.start(trampoline as *const () as u64, top, raw as u64, 0) {
        // SAFETY: vlakno nebezi, ukazatel jsme nikomu nepredali.
        drop(unsafe { Box::from_raw(raw) });
        let _ = me.unmap(addr, len);
        return Err(e);
    }
    Ok(JoinHandle { thread, stack_addr: addr, stack_len: len })
}
