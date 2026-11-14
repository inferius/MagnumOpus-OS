//! Doruceni vyjimek: zprava z kanalu vyjimek a rozhodnuti obsluhy.
//!
//! ```ignore
//! let ch = process.exception_channel(false)?;
//! let e = libos::exception::receive(&ch, None)?;
//! if e.report.kind == abi::exception::kind::PAGE_FAULT { /* opravit */ }
//! e.resume()?; // nebo e.next() / e.kill(); zahozeni = dalsi obsluha
//! ```
//!
//! **Obsluha primo ve vlaknu** ([`set_handler`]): vlakno skoci na obsluhu
//! s ramcem vyjimky (`abi::exception::frame`) na svem zasobniku
//! (nebo alternativnim, [`set_alt_stack`]); obsluha konci
//! [`handler_return`] (RESUME = pokracovat s kontextem z ramce, NEXT =
//! dalsi obsluha v poradi). Podklad pro SEH a signaly.

use abi::exception::{Report, action};
use abi::{Error, Result};

use crate::{Channel, Handle, Process, Thread, rt, sys};

/// Vstup obsluhy ve vlaknu: dostane ukazatel na ramec vyjimky, nevraci se
/// (konci [`handler_return`]).
pub type Handler = extern "C" fn(frame: *mut u8) -> !;

/// Nastavi obsluhu vyjimek primo ve vlaknu pro vlastni proces (musi lezet
/// ve spustitelne pameti - kod programu).
pub fn set_handler(entry: Handler) -> Result<()> {
    sys::exception_handler(abi::exception::handler::SET_ENTRY, rt::self_process() as u64, entry as *const () as u64)
        .map(|_| ())
}

/// Nastavi obsluhu pro jiny proces (pravo `WRITE`; zavadec, debugger).
pub fn set_handler_for(process: &Process, entry: u64) -> Result<()> {
    sys::exception_handler(abi::exception::handler::SET_ENTRY, process.handle().raw() as u64, entry).map(|_| ())
}

/// Zrusi obsluhu ve vlaknu vlastniho procesu.
pub fn clear_handler() -> Result<()> {
    sys::exception_handler(abi::exception::handler::SET_ENTRY, rt::self_process() as u64, 0).map(|_| ())
}

/// Alternativni zasobnik aktualniho vlakna pro obsluhu (jako
/// `sigaltstack`; nutny pro obsluhu preteceni zasobniku). `size` 0 = zadny.
pub fn set_alt_stack(base: u64, size: u64) -> Result<()> {
    sys::exception_handler(abi::exception::handler::SET_STACK, base, size).map(|_| ())
}

/// Zprava o vyjimce z ramce obsluhy ve vlaknu.
///
/// # Safety
///
/// `frame` je ramec, ktery obsluha dostala.
pub unsafe fn frame_report(frame: *const u8) -> Report {
    // SAFETY: predano volajicim (ramec ma aspon `Report::SIZE` B).
    let b = unsafe { core::slice::from_raw_parts(frame, Report::SIZE) };
    Report::decode(b).unwrap_or_default()
}

/// Navrat z obsluhy ve vlaknu (`action::RESUME` nebo `action::NEXT`). Pri
/// uspechu se nevraci (vlakno pokracuje v kontextu z ramce); vrati jen
/// chybu (neplatny ramec = `INVALID_ARGS`, zadna obsluha = `BAD_STATE`).
///
/// # Safety
///
/// `frame` je ramec, ktery obsluha dostala (pripadne upraveny).
pub unsafe fn handler_return(frame: *const u8, disposition: u32) -> Error {
    match sys::exception_handler(abi::exception::handler::RETURN, frame as u64, disposition as u64) {
        Err(e) => e,
        Ok(_) => Error::BadState,
    }
}

/// Obecne registry teto architektury (format ramce i `THREAD_STATE`).
#[cfg(target_arch = "x86_64")]
pub type General = abi::thread::X64General;
/// Obecne registry teto architektury (format ramce i `THREAD_STATE`).
#[cfg(target_arch = "aarch64")]
pub type General = abi::thread::A64General;

/// Offset FP casti (hlavicka `abi::thread::fp::Header`, pak data) v ramci.
#[cfg(target_arch = "x86_64")]
pub const FP_OFF: usize = abi::exception::frame::X64_FP;
/// Offset FP casti (hlavicka `abi::thread::fp::Header`, pak data) v ramci.
#[cfg(target_arch = "aarch64")]
pub const FP_OFF: usize = abi::exception::frame::A64_FP;

/// Velikost dat stavu FPU/SIMD na tomto CPU (bez hlavicky): x86_64 obraz
/// XSAVE pro casti zapnute jadrem (CPUID 0xD.0 EBX, kdyz je OSXSAVE),
/// jinak obraz FXSAVE (512 B); aarch64 528 B. Stejnou hodnotu ma jadro
/// v hlavicce stavu.
pub fn fp_data_len() -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        use core::arch::x86_64::__cpuid_count;
        // OSXSAVE (CPUID.1:ECX bit 27): jadro zaplo XSAVE a XCR0.
        if __cpuid_count(1, 0).ecx & (1 << 27) != 0 {
            (__cpuid_count(0xD, 0).ebx as usize).max(576)
        } else {
            abi::thread::X64_FP_SIZE
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        abi::thread::A64_FP_SIZE
    }
}

/// Velikost stavu FPU pro `THREAD_STATE` (`kind::FP`): hlavicka a data.
pub fn fp_state_size() -> usize {
    abi::thread::fp::HEADER + fp_data_len()
}

/// Velikost ramce obsluhy ve vlaknu na tomto CPU.
pub fn frame_size() -> usize {
    abi::exception::frame::size(FP_OFF, fp_data_len())
}

/// Nejmensi alternativni zasobnik, ktery jadro prijme (ramec a rezerva).
pub fn min_alt_stack() -> usize {
    frame_size() + 256
}

/// Ramec obsluhy ve vlaknu s typovym pristupem (podklad pro `ucontext_t`
/// signalu v `cstd` a `CONTEXT` SEH ve Windows vrstve). Zmeny se projevi
/// az navratem [`Frame::resume`] (jadro je overi jako `THREAD_STATE`).
pub struct Frame {
    ptr: *mut u8,
}

impl Frame {
    /// # Safety
    ///
    /// `ptr` je ramec, ktery obsluha dostala od jadra (a zatim se nevratila).
    pub unsafe fn from_raw(ptr: *mut u8) -> Self {
        Self { ptr }
    }

    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr
    }

    fn bytes(&self, off: usize, len: usize) -> &[u8] {
        // SAFETY: ramec od jadra ma `frame_size()` B (`from_raw`).
        unsafe { core::slice::from_raw_parts(self.ptr.add(off), len) }
    }

    fn bytes_mut(&mut self, off: usize, len: usize) -> &mut [u8] {
        // SAFETY: jako `bytes`; `&mut self` = jediny pristup.
        unsafe { core::slice::from_raw_parts_mut(self.ptr.add(off), len) }
    }

    /// Zprava o vyjimce (druh, adresa, PC ...).
    pub fn report(&self) -> Report {
        Report::decode(self.bytes(abi::exception::frame::REPORT, Report::SIZE)).unwrap_or_default()
    }

    /// Obecne registry v okamziku vyjimky.
    pub fn general(&self) -> General {
        let b = self.bytes(abi::exception::frame::GENERAL, General::SIZE);
        General::decode(b.try_into().unwrap_or(&[0; General::SIZE]))
    }

    /// Prepise obecne registry (pouziji se pri navratu).
    pub fn set_general(&mut self, g: &General) {
        self.bytes_mut(abi::exception::frame::GENERAL, General::SIZE).copy_from_slice(&g.encode());
    }

    /// Hlavicka FP casti (format, delka dat, x86_64 XCR0).
    pub fn fp_header(&self) -> abi::thread::fp::Header {
        abi::thread::fp::Header::decode(self.bytes(FP_OFF, abi::thread::fp::HEADER)).unwrap_or_default()
    }

    /// Data stavu FPU/SIMD (x86_64 obraz XSAVE - prvnich 512 B je obraz
    /// FXSAVE: x87, XMM0-15, MXCSR; aarch64 V0-V31 + FPCR + FPSR).
    pub fn fp(&self) -> &[u8] {
        self.bytes(FP_OFF + abi::thread::fp::HEADER, fp_data_len())
    }

    pub fn fp_mut(&mut self) -> &mut [u8] {
        self.bytes_mut(FP_OFF + abi::thread::fp::HEADER, fp_data_len())
    }

    /// Adresa instrukce (x86_64 po `int3` uz za ni, aarch64 na `brk`).
    pub fn pc(&self) -> u64 {
        let g = self.general();
        #[cfg(target_arch = "x86_64")]
        return g.rip;
        #[cfg(target_arch = "aarch64")]
        return g.pc;
    }

    pub fn set_pc(&mut self, pc: u64) {
        let mut g = self.general();
        #[cfg(target_arch = "x86_64")]
        {
            g.rip = pc;
        }
        #[cfg(target_arch = "aarch64")]
        {
            g.pc = pc;
        }
        self.set_general(&g);
    }

    pub fn sp(&self) -> u64 {
        let g = self.general();
        #[cfg(target_arch = "x86_64")]
        return g.rsp;
        #[cfg(target_arch = "aarch64")]
        return g.sp;
    }

    pub fn set_sp(&mut self, sp: u64) {
        let mut g = self.general();
        #[cfg(target_arch = "x86_64")]
        {
            g.rsp = sp;
        }
        #[cfg(target_arch = "aarch64")]
        {
            g.sp = sp;
        }
        self.set_general(&g);
    }

    /// Pokracovat s kontextem z ramce. Vraci jen chybu (neplatny kontext).
    pub fn resume(self) -> Error {
        // SAFETY: ramec od jadra (`from_raw`).
        unsafe { handler_return(self.ptr, action::RESUME) }
    }

    /// Nabidnout vyjimku dalsim obsluham (kanaly, debugger, konec procesu).
    pub fn next(self) -> Error {
        // SAFETY: jako `resume`.
        unsafe { handler_return(self.ptr, action::NEXT) }
    }
}

/// Alokuje alternativni zasobnik `size` B (zaokrouhleno na stranky) a
/// nastavi ho aktualnimu vlaknu; vraci jeho zacatek (uvolnit
/// `vm::release` po `set_alt_stack(0, 0)`). Nutny pro obsluhu preteceni
/// zasobniku (`SIGSEGV` s `SA_ONSTACK`, SEH `EXCEPTION_STACK_OVERFLOW`).
pub fn alloc_alt_stack(size: u64) -> Result<u64> {
    let size = size.next_multiple_of(crate::vm::page_size());
    let base = crate::vm::alloc(size, abi::vm::MAP_READ | abi::vm::MAP_WRITE)?;
    if let Err(e) = set_alt_stack(base, size) {
        let _ = crate::vm::release(base, size);
        return Err(e);
    }
    Ok(base)
}

/// Trida vyjimky podle POSIX signalu (cisla signalu si urci `cstd`; Wine
/// mapuje na kody `EXCEPTION_*` samo podle `Report::kind` a `access`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalClass {
    /// Neplatny pristup do pameti (`SIGSEGV`).
    Segv,
    /// Nezarovnany pristup (`SIGBUS`).
    Bus,
    /// Aritmeticka chyba (`SIGFPE`; aarch64 deleni nulou nehlasi).
    Fpe,
    /// Neplatna instrukce (`SIGILL`).
    Ill,
    /// Breakpoint (`SIGTRAP`).
    Trap,
}

/// Trida signalu pro druh vyjimky (`abi::exception::kind`).
pub fn signal_class(kind: u32) -> SignalClass {
    use abi::exception::kind as k;
    match kind {
        k::DIVIDE => SignalClass::Fpe,
        k::BREAKPOINT => SignalClass::Trap,
        k::INVALID_OPCODE => SignalClass::Ill,
        k::ALIGNMENT => SignalClass::Bus,
        _ => SignalClass::Segv,
    }
}

/// Vyjimka doruzena obsluze: zprava, vlakno (kontext pres
/// [`Thread::read_state`] / [`Thread::write_state`]) a token.
pub struct Exception {
    pub report: Report,
    pub thread: Thread,
    token: Handle,
}

impl Exception {
    fn resolve(&self, a: u32) -> Result<()> {
        sys::exception_resolve(self.token.raw(), a)
    }

    /// Vlakno pokracuje (se zmenenym kontextem).
    pub fn resume(&self) -> Result<()> {
        self.resolve(action::RESUME)
    }

    /// Dalsi obsluha v poradi.
    pub fn next(&self) -> Result<()> {
        self.resolve(action::NEXT)
    }

    /// Ukoncit proces (jako neosetrena vyjimka).
    pub fn kill(&self) -> Result<()> {
        self.resolve(action::KILL)
    }
}

/// Precte dalsi vyjimku z kanalu vyjimek (ceka do `deadline`).
pub fn receive(ch: &Channel, deadline: Option<u64>) -> Result<Exception> {
    let m = ch.read(deadline)?;
    let report = Report::decode(&m.data).ok_or(Error::Corrupted)?;
    let mut handles = m.handles.into_iter();
    let (Some(thread), Some(token)) = (handles.next(), handles.next()) else {
        return Err(Error::Corrupted);
    };
    Ok(Exception { report, thread: Thread::from_handle(thread), token })
}
