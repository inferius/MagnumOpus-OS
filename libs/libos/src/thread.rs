//! Vlakna ve vlastnim procesu: `spawn(f)` s vlastnim zasobnikem.
//!
//! Zasobnik je objekt pameti namapovany do vlastniho procesu s ochrannou
//! oblasti pod sebou (`vm::map_stack`); po `join` se uvolni oboje. Heap je sdileny (alokator se zamkem), takze vlakno muze
//! alokovat. Panika ve vlakne ukonci cely proces (stejne jako v hlavnim).

use alloc::boxed::Box;

use abi::Result;

use crate::{Process, Thread, Vmo, rt, sys};

/// Velikost zasobniku vlakna (interpret shellu smi pouzit ~160 KB).
pub const STACK_SIZE: u64 = 256 * 1024;

type Job = Box<dyn FnOnce() + Send + 'static>;

/// Spustene vlakno; `join` pocka na konec a uvolni zasobnik.
pub struct JoinHandle {
    thread: Thread,
    stack_addr: u64,
    stack_len: u64,
    /// Ukazatel vlakna z bloku TLS zavadece (0 = program bez zavadece).
    tp: u64,
}

impl JoinHandle {
    /// Pocka na konec vlakna (`deadline` v ns od startu, `None` = navzdy).
    pub fn join(self, deadline: Option<u64>) -> Result<()> {
        self.thread.wait_exit(deadline)?;
        if self.tp != 0 {
            crate::loader::thread_tls_free(self.tp);
        }
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

extern "C" fn trampoline(job: u64, tp: u64) -> ! {
    // Program pod zavadecem: TLS modulu (blok od zavadece) driv nez kod.
    if tp != 0 && sys::thread_set_pointer(tp).is_err() {
        rt::fail("libos: nelze nastavit ukazatel vlakna (TLS)");
    }
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

/// Jako [`spawn`], ale se zasobnikem `stack` bajtu (napr. kryptografie TLS
/// ve vlakne potrebuje vic nez [`STACK_SIZE`]).
pub fn spawn_sized(name: &str, stack: u64, f: impl FnOnce() + Send + 'static) -> Result<JoinHandle> {
    let me = Process::from_handle(crate::Handle::from_raw(rt::self_process()));
    let r = spawn_stack(&me, name, stack, f);
    core::mem::forget(me);
    r
}

fn spawn_in(me: &Process, name: &str, f: impl FnOnce() + Send + 'static) -> Result<JoinHandle> {
    spawn_stack(me, name, STACK_SIZE, f)
}

fn spawn_stack(me: &Process, name: &str, stack: u64, f: impl FnOnce() + Send + 'static) -> Result<JoinHandle> {
    let len = stack.max(STACK_SIZE).next_multiple_of(crate::vm::page_size());
    let stack = Vmo::create(len)?;
    let (addr, total, stack_top) = crate::vm::map_stack(me, &stack, len)?;
    let len = total;
    let thread = match Thread::create(me, name) {
        Ok(t) => t,
        Err(e) => {
            let _ = me.unmap(addr, len);
            return Err(e);
        }
    };
    // x86_64: rsp % 16 == 8 jako po instrukci call; aarch64: sp % 16 == 0.
    let top = if cfg!(target_arch = "x86_64") { stack_top - 8 } else { stack_top };
    // Pod zavadecem dostane vlakno blok TLS vsech modulu (abi::loader).
    let tp = match crate::loader::thread_tls_new() {
        Some(0) => {
            let _ = me.unmap(addr, len);
            return Err(abi::Error::NoMemory);
        }
        Some(tp) => tp,
        None => 0,
    };
    let job: Box<Job> = Box::new(Box::new(f));
    let raw = Box::into_raw(job);
    if let Err(e) = thread.start(trampoline as *const () as u64, top, raw as u64, tp) {
        // SAFETY: vlakno nebezi, ukazatel jsme nikomu nepredali.
        drop(unsafe { Box::from_raw(raw) });
        if tp != 0 {
            crate::loader::thread_tls_free(tp);
        }
        let _ = me.unmap(addr, len);
        return Err(e);
    }
    Ok(JoinHandle { thread, stack_addr: addr, stack_len: len, tp })
}
