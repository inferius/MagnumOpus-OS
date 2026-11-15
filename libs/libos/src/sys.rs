//! Syrova volani jadra. Programy maji pouzivat typy z libos; tyto funkce
//! jsou verejne kvuli testum rozhrani a nizkourovnovym nastrojum.

use core::arch::asm;

use abi::syscall as nr;
use abi::{Error, Result};

/// Syscall se sesti argumenty (x86_64: instrukce `syscall`, cislo v `rax`,
/// argumenty `rdi, rsi, rdx, r10, r8, r9`). Jadro zachova vsechny registry
/// krome `rax` (vysledek) a `rcx`, `r11` (prepise je instrukce `syscall`).
///
/// # Safety
///
/// Ukazatele v argumentech musi odpovidat popisu daneho syscallu.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub unsafe fn raw(n: u64, a0: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> u64 {
    let ret;
    // SAFETY: predano volajicim; instrukce syscall prepisuje jen rcx a r11.
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") n => ret,
            in("rdi") a0,
            in("rsi") a1,
            in("rdx") a2,
            in("r10") a3,
            in("r8") a4,
            in("r9") a5,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }
    ret
}

/// Syscall se sesti argumenty (aarch64: `svc #0`, cislo v `x8`, argumenty
/// `x0`-`x5`, vysledek v `x0`). Jadro zachova vsechny ostatni registry.
///
/// # Safety
///
/// Ukazatele v argumentech musi odpovidat popisu daneho syscallu.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub unsafe fn raw(n: u64, a0: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> u64 {
    let ret;
    // SAFETY: predano volajicim; jadro meni jen x0 (vysledek).
    unsafe {
        asm!(
            "svc #0",
            in("x8") n,
            inlateout("x0") a0 => ret,
            in("x1") a1,
            in("x2") a2,
            in("x3") a3,
            in("x4") a4,
            in("x5") a5,
            options(nostack)
        );
    }
    ret
}

#[inline(always)]
fn call(n: u64, a: [u64; 6]) -> Result<u64> {
    // SAFETY: vsechny obalky nize predavaji ukazatele na platne buffery
    // odpovidajici velikosti.
    abi::error::decode(unsafe { raw(n, a[0], a[1], a[2], a[3], a[4], a[5]) })
}

/// Zapise text do logu jadra. Delsi nez `DEBUG_LOG_MAX` jde po kusech
/// (jadro by celou zpravu odmitlo a ztratila by se i s duvodem chyby).
pub fn debug_log(text: &str) {
    for part in abi::syscall::debug_log_chunks(text, abi::syscall::DEBUG_LOG_MAX) {
        let _ = call(
            nr::DEBUG_LOG,
            [part.as_ptr() as u64, part.len() as u64, 0, 0, 0, 0],
        );
    }
}

pub fn clock_monotonic() -> u64 {
    call(nr::CLOCK_MONOTONIC, [0; 6]).unwrap_or(0)
}

/// Realny cas; `NOT_SUPPORTED`, kdyz ho jadro nezna.
pub fn clock_realtime() -> Result<abi::time::Reading> {
    let mut buf = [0u8; abi::time::Reading::SIZE];
    call(nr::CLOCK_REALTIME, [buf.as_mut_ptr() as u64, 0, 0, 0, 0, 0])?;
    Ok(abi::time::Reading::decode(&buf))
}

/// Nastavi realny cas (Resource s pravem `SET_CLOCK`); `Ok(true)` = zapsano
/// i do hardwarovych hodin.
pub fn clock_set(resource: u32, req: &abi::time::SetRequest, flags: u64) -> Result<bool> {
    let buf = req.encode();
    call(
        nr::CLOCK_SET,
        [resource as u64, buf.as_ptr() as u64, flags, 0, 0, 0],
    )
    .map(|r| r == 1)
}

/// Plynula korekce realneho casu (Resource s pravem `SET_CLOCK`), viz
/// `abi::time::adjust`. Vraci stav po zmene.
pub fn clock_adjust(resource: u32, offset_ns: i64, freq_ppb: i64, flags: u64) -> Result<abi::time::AdjustStatus> {
    let mut buf = [0u8; abi::time::AdjustStatus::SIZE];
    call(
        nr::CLOCK_ADJUST,
        [resource as u64, offset_ns as u64, freq_ppb as u64, flags, buf.as_mut_ptr() as u64, 0],
    )?;
    Ok(abi::time::AdjustStatus::decode(&buf))
}

pub fn handle_close(h: u32) -> Result<()> {
    call(nr::HANDLE_CLOSE, [h as u64, 0, 0, 0, 0, 0]).map(|_| ())
}

/// `rights = None` zachova prava.
pub fn handle_duplicate(h: u32, rights: Option<u32>) -> Result<u32> {
    let r = rights.map_or(u32::MAX as u64, |r| r as u64);
    call(nr::HANDLE_DUPLICATE, [h as u64, r, 0, 0, 0, 0]).map(|v| v as u32)
}

pub fn object_wait_one(h: u32, mask: u32, deadline: u64) -> (Result<()>, u32) {
    let mut observed = 0u32;
    let r = call(
        nr::OBJECT_WAIT_ONE,
        [
            h as u64,
            mask as u64,
            deadline,
            &mut observed as *mut u32 as u64,
            0,
            0,
        ],
    );
    (r.map(|_| ()), observed)
}

pub fn object_wait_many(items: &mut [abi::object::WaitItem], deadline: u64) -> Result<usize> {
    call(
        nr::OBJECT_WAIT_MANY,
        [
            items.as_mut_ptr() as u64,
            items.len() as u64,
            deadline,
            0,
            0,
            0,
        ],
    )
    .map(|v| v as usize)
}

pub fn object_signal(h: u32, clear: u32, set: u32) -> Result<()> {
    call(
        nr::OBJECT_SIGNAL,
        [h as u64, clear as u64, set as u64, 0, 0, 0],
    )
    .map(|_| ())
}

pub fn event_create() -> Result<u32> {
    call(nr::EVENT_CREATE, [0; 6]).map(|v| v as u32)
}

pub fn channel_create() -> Result<(u32, u32)> {
    let mut out = [0u32; 2];
    call(nr::CHANNEL_CREATE, [out.as_mut_ptr() as u64, 0, 0, 0, 0, 0])?;
    Ok((out[0], out[1]))
}

pub fn channel_write(h: u32, data: &[u8], handles: &[u32]) -> Result<()> {
    call(
        nr::CHANNEL_WRITE,
        [
            h as u64,
            data.as_ptr() as u64,
            data.len() as u64,
            handles.as_ptr() as u64,
            handles.len() as u64,
            0,
        ],
    )
    .map(|_| ())
}

/// Vraci (delka dat, pocet handlu). Pri `BufferTooSmall` jsou to potrebne
/// velikosti.
pub fn channel_read(h: u32, data: &mut [u8], handles: &mut [u32]) -> (Result<()>, (usize, usize)) {
    let mut actual = [0u32; 2];
    let r = call(
        nr::CHANNEL_READ,
        [
            h as u64,
            data.as_mut_ptr() as u64,
            data.len() as u64,
            handles.as_mut_ptr() as u64,
            handles.len() as u64,
            actual.as_mut_ptr() as u64,
        ],
    );
    (r.map(|_| ()), (actual[0] as usize, actual[1] as usize))
}

/// Objekt pameti na ucet jobu procesu `process` (0 = vlastni).
pub fn vmo_create(size: u64, process: u32) -> Result<u32> {
    vmo_create_with(size, process, 0)
}

/// `VMO_CREATE` s volbami (`abi::vm::VMO_PINNED`).
pub fn vmo_create_with(size: u64, process: u32, flags: u64) -> Result<u32> {
    call(nr::VMO_CREATE, [size, process as u64, flags, 0, 0, 0]).map(|v| v as u32)
}

/// Zpristupni cast pripnuteho objektu pameti zarizeni (`DMA_MAP`); vrati
/// handle mapovani.
pub fn dma_map(resource: u32, vmo: u32, offset: u64, len: u64, flags: u64, iova_hint: u64) -> Result<u32> {
    call(
        nr::DMA_MAP,
        [resource as u64, vmo as u64, offset, len, flags, iova_hint],
    )
    .map(|v| v as u32)
}

/// Useky mapovani (`DMA_SEGMENTS`): zapise jich nejvys `out.len()`, vrati
/// pocet vsech.
pub fn dma_segments(map: u32, out: &mut [abi::dma::Segment]) -> Result<usize> {
    // `Segment` je #[repr(C)] (16 B, dve u64) - stejne jako zapisuje jadro.
    call(
        nr::DMA_SEGMENTS,
        [map as u64, out.as_mut_ptr() as u64, out.len() as u64, 0, 0, 0],
    )
    .map(|v| v as usize)
}

/// Udrzba cache mapovani (`DMA_SYNC`, `abi::dma::sync`).
pub fn dma_sync(map: u32, offset: u64, len: u64, direction: u64) -> Result<()> {
    call(nr::DMA_SYNC, [map as u64, offset, len, direction, 0, 0]).map(|_| ())
}

/// `VMO_CREATE` s `VMO_RESIZABLE`: pocatecni velikost a kapacita.
pub fn vmo_create_resizable(size: u64, capacity: u64, process: u32) -> Result<u32> {
    call(nr::VMO_CREATE, [size, process as u64, abi::vm::VMO_RESIZABLE, capacity, 0, 0]).map(|v| v as u32)
}

/// `VMO_SET_SIZE` (pravo `RESIZE`).
pub fn vmo_set_size(vmo: u32, size: u64) -> Result<()> {
    call(nr::VMO_SET_SIZE, [vmo as u64, size, 0, 0, 0, 0]).map(|_| ())
}

/// `VMO_DIRTY`: indexy spinavych stranek do `buf` (nejvys jeho delka),
/// oznaci je jako ciste; vraci pocet. Prvni volani zapne sledovani.
pub fn vmo_dirty(vmo: u32, buf: &mut [u64]) -> Result<usize> {
    call(nr::VMO_DIRTY, [vmo as u64, buf.as_mut_ptr() as u64, buf.len() as u64, 0, 0, 0]).map(|n| n as usize)
}

/// `VMO_SIZE`: aktualni velikost objektu pameti.
pub fn vmo_size(vmo: u32) -> Result<u64> {
    call(nr::VMO_SIZE, [vmo as u64, 0, 0, 0, 0, 0])
}

pub fn vmo_seal(vmo: u32) -> Result<()> {
    call(nr::VMO_SEAL, [vmo as u64, 0, 0, 0, 0, 0]).map(|_| ())
}

pub fn vmo_clone(vmo: u32, offset: u64, len: u64, process: u32) -> Result<u32> {
    call(nr::VMO_CLONE, [vmo as u64, offset, len, process as u64, 0, 0]).map(|v| v as u32)
}

pub fn vmo_read(h: u32, offset: u64, buf: &mut [u8]) -> Result<()> {
    call(
        nr::VMO_READ,
        [
            h as u64,
            offset,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
            0,
            0,
        ],
    )
    .map(|_| ())
}

pub fn vmo_write(h: u32, offset: u64, data: &[u8]) -> Result<()> {
    call(
        nr::VMO_WRITE,
        [
            h as u64,
            offset,
            data.as_ptr() as u64,
            data.len() as u64,
            0,
            0,
        ],
    )
    .map(|_| ())
}

pub fn vm_map(process: u32, vmo: u32, offset: u64, len: u64, flags: u32, addr: u64) -> Result<u64> {
    call(
        nr::VM_MAP,
        [process as u64, vmo as u64, offset, len, flags as u64, addr],
    )
}

pub fn vm_unmap(process: u32, addr: u64, len: u64) -> Result<()> {
    call(nr::VM_UNMAP, [process as u64, addr, len, 0, 0, 0]).map(|_| ())
}

/// `VM_UNMAP` s priznaky (`abi::vm::UNMAP_DECOMMIT`).
pub fn vm_unmap_flags(process: u32, addr: u64, len: u64, flags: u32) -> Result<()> {
    call(nr::VM_UNMAP, [process as u64, addr, len, flags as u64, 0, 0]).map(|_| ())
}

pub fn vm_reserve(process: u32, addr: u64, len: u64, flags: u32) -> Result<u64> {
    call(nr::VM_RESERVE, [process as u64, addr, len, flags as u64, 0, 0])
}

pub fn vm_protect(process: u32, addr: u64, len: u64, prot: u32) -> Result<()> {
    call(nr::VM_PROTECT, [process as u64, addr, len, prot as u64, 0, 0]).map(|_| ())
}

pub fn vm_query(process: u32, addr: u64) -> Result<abi::vm::RegionInfo> {
    let mut out = [0u8; abi::vm::RegionInfo::SIZE];
    call(nr::VM_QUERY, [process as u64, addr, out.as_mut_ptr() as u64, 0, 0, 0])?;
    Ok(abi::vm::RegionInfo::decode(&out))
}

pub fn process_create(job: u32, name: &str) -> Result<u32> {
    call(
        nr::PROCESS_CREATE,
        [job as u64, name.as_ptr() as u64, name.len() as u64, 0, 0, 0],
    )
    .map(|v| v as u32)
}

pub fn process_start(
    process: u32,
    thread: u32,
    entry: u64,
    stack: u64,
    bootstrap: u32,
) -> Result<()> {
    call(
        nr::PROCESS_START,
        [
            process as u64,
            thread as u64,
            entry,
            stack,
            bootstrap as u64,
            0,
        ],
    )
    .map(|_| ())
}

pub fn process_exit(code: i64) -> ! {
    let _ = call(nr::PROCESS_EXIT, [code as u64, 0, 0, 0, 0, 0]);
    // Jadro vlakno ukonci pri navratu ze syscallu; sem se nedostaneme.
    loop {
        core::hint::spin_loop();
    }
}

pub fn process_kill(process: u32) -> Result<()> {
    call(nr::PROCESS_KILL, [process as u64, 0, 0, 0, 0, 0]).map(|_| ())
}

pub fn process_exit_code(process: u32) -> Result<i64> {
    let mut code = 0u64;
    call(
        nr::PROCESS_EXIT_CODE,
        [process as u64, &mut code as *mut u64 as u64, 0, 0, 0, 0],
    )?;
    Ok(code as i64)
}

pub fn thread_create(process: u32, name: &str) -> Result<u32> {
    call(
        nr::THREAD_CREATE,
        [
            process as u64,
            name.as_ptr() as u64,
            name.len() as u64,
            0,
            0,
            0,
        ],
    )
    .map(|v| v as u32)
}

pub fn exception_channel(task: u32, flags: u32) -> Result<u32> {
    call(nr::EXCEPTION_CHANNEL, [task as u64, flags as u64, 0, 0, 0, 0]).map(|v| v as u32)
}

pub fn exception_handler(op: u32, a0: u64, a1: u64) -> Result<u64> {
    call(nr::EXCEPTION_HANDLER, [op as u64, a0, a1, 0, 0, 0])
}

pub fn exception_resolve(token: u32, action: u32) -> Result<()> {
    call(nr::EXCEPTION_RESOLVE, [token as u64, action as u64, 0, 0, 0, 0]).map(|_| ())
}

/// Handly vlaken procesu do `out`; vraci celkovy pocet vlaken.
pub fn process_threads(process: u32, out: &mut [u32]) -> Result<usize> {
    call(
        nr::PROCESS_THREADS,
        [process as u64, out.as_mut_ptr() as u64, out.len() as u64, 0, 0, 0],
    )
    .map(|v| v as usize)
}

/// Preruseni vlakna (`THREAD_INTERRUPT`): kod 0..63 se doruci obsluze ve
/// vlakne pri navratu do userspace; blokujici cekani vlakna vrati
/// `Err(Interrupted)`. Pravo `MANAGE`.
pub fn thread_interrupt(thread: u32, code: u32) -> Result<()> {
    call(nr::THREAD_INTERRUPT, [thread as u64, u64::from(code), 0, 0, 0, 0]).map(|_| ())
}

/// `PROCESS_MEMORY` READ: z `addr` v procesu `process` do `buf`; vraci
/// prenesene bajty (mene = castecna kopie).
pub fn process_memory_read(process: u32, addr: u64, buf: &mut [u8]) -> Result<usize> {
    call(
        nr::PROCESS_MEMORY,
        [process as u64, abi::process::memory::READ, addr, buf.as_mut_ptr() as u64, buf.len() as u64, 0],
    )
    .map(|v| v as usize)
}

/// `PROCESS_MEMORY` WRITE: `data` do `addr` v procesu `process`; vraci
/// prenesene bajty.
pub fn process_memory_write(process: u32, addr: u64, data: &[u8]) -> Result<usize> {
    call(
        nr::PROCESS_MEMORY,
        [process as u64, abi::process::memory::WRITE, addr, data.as_ptr() as u64, data.len() as u64, 0],
    )
    .map(|v| v as usize)
}

/// `THREAD_SELF`: handle volajiciho vlakna s pravy `rights` (0 = vychozi).
pub fn thread_self(rights: u32) -> Result<u32> {
    call(nr::THREAD_SELF, [u64::from(rights), 0, 0, 0, 0, 0]).map(|v| v as u32)
}

pub fn thread_suspend(thread: u32) -> Result<u32> {
    call(nr::THREAD_SUSPEND, [thread as u64, 0, 0, 0, 0, 0]).map(|v| v as u32)
}

pub fn thread_state_read(thread: u32, kind: u32, buf: &mut [u8]) -> Result<()> {
    call(
        nr::THREAD_STATE,
        [thread as u64, abi::thread::op::READ as u64, kind as u64, buf.as_mut_ptr() as u64, buf.len() as u64, 0],
    )
    .map(|_| ())
}

pub fn thread_state_write(thread: u32, kind: u32, buf: &[u8]) -> Result<()> {
    call(
        nr::THREAD_STATE,
        [thread as u64, abi::thread::op::WRITE as u64, kind as u64, buf.as_ptr() as u64, buf.len() as u64, 0],
    )
    .map(|_| ())
}

pub fn thread_start(thread: u32, entry: u64, stack: u64, arg0: u64, arg1: u64) -> Result<()> {
    call(
        nr::THREAD_START,
        [thread as u64, entry, stack, arg0, arg1, 0],
    )
    .map(|_| ())
}

pub fn thread_exit() -> ! {
    let _ = call(nr::THREAD_EXIT, [0; 6]);
    loop {
        core::hint::spin_loop();
    }
}

pub fn thread_yield() {
    let _ = call(nr::THREAD_YIELD, [0; 6]);
}

/// Usne, dokud je `*addr == expected` a nikdo nezavola [`futex_wake`] na
/// stejne adrese, nejdele do `deadline` (ns od startu, `u64::MAX` = navzdy).
/// Vyprseni = `Err(TimedOut)`. Planne probuzeni je mozne.
pub fn futex_wait(addr: &core::sync::atomic::AtomicU32, expected: u32, deadline: u64) -> Result<()> {
    call(nr::FUTEX, [nr::futex_op::WAIT, addr.as_ptr() as u64, u64::from(expected), deadline, 0, 0]).map(|_| ())
}

/// Probudi az `count` vlaken cekajicich na `addr`; vraci pocet probuzenych.
pub fn futex_wake(addr: &core::sync::atomic::AtomicU32, count: u32) -> u32 {
    call(nr::FUTEX, [nr::futex_op::WAKE, addr.as_ptr() as u64, u64::from(count), 0, 0, 0]).map_or(0, |n| n as u32)
}

/// Nastavi registr ukazatele vlakna (TLS) aktualniho vlakna.
pub fn thread_set_pointer(value: u64) -> Result<()> {
    call(nr::THREAD_SET_POINTER, [value, nr::thread_pointer::TLS, 0, 0, 0, 0]).map(|_| ())
}

/// Nastavi druhy ukazatel vlakna (x86_64 baze GS, napr. TEB Windows
/// vrstvy); na aarch64 `Err(NotSupported)`.
pub fn thread_set_second_pointer(value: u64) -> Result<()> {
    call(nr::THREAD_SET_POINTER, [value, nr::thread_pointer::SECOND, 0, 0, 0, 0]).map(|_| ())
}

/// Volani neexistujiciho syscallu (pro testy rozhrani).
pub fn invalid_syscall() -> Result<u64> {
    call(nr::COUNT + 1000, [0; 6])
}

pub fn vmo_create_contiguous(resource: u32, size: u64) -> Result<u32> {
    call(
        nr::VMO_CREATE_CONTIGUOUS,
        [resource as u64, size, 0, 0, 0, 0],
    )
    .map(|v| v as u32)
}

pub fn vmo_create_physical(resource: u32, phys: u64, size: u64, cache: u64) -> Result<u32> {
    call(
        nr::VMO_CREATE_PHYSICAL,
        [resource as u64, phys, size, cache, 0, 0],
    )
    .map(|v| v as u32)
}

pub fn vmo_revoke(resource: u32, vmo: u32) -> Result<()> {
    call(nr::VMO_REVOKE, [resource as u64, vmo as u64, 0, 0, 0, 0]).map(|_| ())
}

pub fn vmo_phys_addr(resource: u32, vmo: u32, offset: u64) -> Result<u64> {
    call(
        nr::VMO_PHYS_ADDR,
        [resource as u64, vmo as u64, offset, 0, 0, 0],
    )
}

pub fn interrupt_create(resource: u32) -> Result<(u32, abi::object::MsiMessage)> {
    let mut msi = abi::object::MsiMessage::default();
    let out = &mut msi as *mut abi::object::MsiMessage as u64;
    let h = call(nr::INTERRUPT_CREATE, [resource as u64, out, 0, 0, 0, 0])?;
    Ok((h as u32, msi))
}

/// Objekt preruseni na pevne lince (spousteni podle platformy: x86 MADT,
/// aarch64 uroven). Stejne jako drive; priznaky viz
/// [`interrupt_create_line_flags`].
pub fn interrupt_create_line(resource: u32, line: u32) -> Result<u32> {
    interrupt_create_line_flags(resource, line, 0)
}

/// Jako [`interrupt_create_line`] s priznaky `abi::syscall::line_flags`.
pub fn interrupt_create_line_flags(resource: u32, line: u32, flags: u64) -> Result<u32> {
    call(
        nr::INTERRUPT_CREATE_LINE,
        [resource as u64, line as u64, flags, 0, 0, 0],
    )
    .map(|h| h as u32)
}

/// `SCHED_CONTROL` (operace z `abi::sched::op`).
pub fn sched_control(op: u64, a: u64, b: u64, c: u64, d: u64) -> Result<u64> {
    call(nr::SCHED_CONTROL, [op, a, b, c, d, 0])
}

/// Vypnuti / restart pres firmware (`SYSTEM_POWER`); pri uspechu se nevraci.
pub fn system_power(resource: u32, op: u64) -> Result<()> {
    call(nr::SYSTEM_POWER, [resource as u64, op, 0, 0, 0, 0]).map(|_| ())
}

/// Cteni logu jadra od pozice `cursor` (`LOG_READ`): (skutecny zacatek,
/// pocet bajtu v `buf`, celkem ztracenych radku).
pub fn log_read(resource: u32, cursor: u64, buf: &mut [u8]) -> Result<(u64, usize, u64)> {
    let mut out = [0u64; 2];
    let n = call(
        nr::LOG_READ,
        [resource as u64, cursor, buf.as_mut_ptr() as u64, buf.len() as u64, out.as_mut_ptr() as u64, 0],
    )?;
    Ok((out[0], n as usize, out[1]))
}

/// Rizeni logu jadra (`LOG_CONTROL`), `op` z `abi::syscall::log_op`.
pub fn log_control(resource: u32, op: u64, buf: &mut [u8]) -> Result<usize> {
    call(nr::LOG_CONTROL, [resource as u64, op, buf.as_mut_ptr() as u64, buf.len() as u64, 0, 0]).map(|n| n as usize)
}

/// Zaznam padu jadra (`CRASH_RECORD`), `op` z `abi::syscall::crash_op`.
pub fn crash_record(resource: u32, op: u64, buf: &mut [u8]) -> Result<usize> {
    call(nr::CRASH_RECORD, [resource as u64, op, buf.as_mut_ptr() as u64, buf.len() as u64, 0, 0]).map(|n| n as usize)
}

pub fn ioport_read(resource: u32, port: u16, width: u8) -> Result<u32> {
    call(
        nr::IOPORT_READ,
        [resource as u64, port as u64, width as u64, 0, 0, 0],
    )
    .map(|v| v as u32)
}

pub fn ioport_write(resource: u32, port: u16, width: u8, value: u32) -> Result<()> {
    call(
        nr::IOPORT_WRITE,
        [resource as u64, port as u64, width as u64, value as u64, 0, 0],
    )
    .map(|_| ())
}

pub fn interrupt_ack(h: u32) -> Result<()> {
    call(nr::INTERRUPT_ACK, [h as u64, 0, 0, 0, 0, 0]).map(|_| ())
}

/// Velikost stranky adresniho prostoru procesu (`VM_PAGE_SIZE`).
pub fn vm_page_size(process: u32) -> Result<u64> {
    call(nr::VM_PAGE_SIZE, [process as u64, 0, 0, 0, 0, 0])
}

pub fn resource_create(
    resource: u32,
    ranges: &[(u64, u64)],
    irq_max: u32,
    dma_max: u64,
    device: u64,
) -> Result<u32> {
    let mut raw = alloc::vec::Vec::with_capacity(ranges.len() * 16);
    for &(s, l) in ranges {
        raw.extend_from_slice(&s.to_le_bytes());
        raw.extend_from_slice(&l.to_le_bytes());
    }
    call(
        nr::RESOURCE_CREATE,
        [
            resource as u64,
            raw.as_ptr() as u64,
            ranges.len() as u64,
            irq_max as u64,
            dma_max,
            device,
        ],
    )
    .map(|h| h as u32)
}

#[allow(dead_code)]
fn _assert_error_is_copy(e: Error) -> Error {
    e
}

/// Nahodne bajty (nejvys `RANDOM_MAX`); `Ok(true)` = generator nasazeny.
pub fn random_get(buf: &mut [u8], flags: u64) -> Result<bool> {
    call(
        nr::RANDOM_GET,
        [buf.as_mut_ptr() as u64, buf.len() as u64, flags, 0, 0, 0],
    )
    .map(|v| v == 1)
}

pub fn random_add(resource: u32, data: &[u8], credit_bits: u32) -> Result<()> {
    call(
        nr::RANDOM_ADD,
        [
            resource as u64,
            data.as_ptr() as u64,
            data.len() as u64,
            credit_bits as u64,
            0,
            0,
        ],
    )
    .map(|_| ())
}

pub fn port_create() -> Result<u32> {
    call(nr::PORT_CREATE, [0; 6]).map(|v| v as u32)
}

pub fn object_wait_async(h: u32, port: u32, key: u64, mask: u32) -> Result<()> {
    call(
        nr::OBJECT_WAIT_ASYNC,
        [h as u64, port as u64, key, mask as u64, 0, 0],
    )
    .map(|_| ())
}

pub fn port_wait(port: u32, deadline: u64) -> Result<abi::object::PortPacket> {
    let mut buf = [0u8; abi::object::PortPacket::SIZE];
    call(nr::PORT_WAIT, [port as u64, deadline, buf.as_mut_ptr() as u64, 0, 0, 0])?;
    Ok(abi::object::PortPacket::decode(&buf))
}

pub fn job_create(parent: u32) -> Result<u32> {
    call(nr::JOB_CREATE, [parent as u64, 0, 0, 0, 0, 0]).map(|v| v as u32)
}

pub fn job_set_limit(job: u32, pool: u32, bytes: u64) -> Result<()> {
    call(nr::JOB_SET_LIMIT, [job as u64, pool as u64, bytes, 0, 0, 0]).map(|_| ())
}

/// `FB_CONTROL`: geometrie framebufferu.
pub fn fb_info(vmo: u32) -> Result<abi::syscall::FbInfo> {
    let mut buf = [0u8; abi::syscall::FbInfo::SIZE];
    call(nr::FB_CONTROL, [vmo as u64, abi::syscall::fb_op::INFO, buf.as_mut_ptr() as u64, 0, 0, 0])?;
    Ok(abi::syscall::FbInfo::decode(&buf))
}

/// `FB_CONTROL`: prevzit (`true`) nebo vratit (`false`) framebuffer.
pub fn fb_claim(vmo: u32, claim: bool) -> Result<()> {
    let op = if claim { abi::syscall::fb_op::CLAIM } else { abi::syscall::fb_op::RELEASE };
    call(nr::FB_CONTROL, [vmo as u64, op, 0, 0, 0, 0]).map(|_| ())
}

/// Procesy jobu a jeho potomku (`JOB_PROCESSES`); `buf` = misto pro zaznamy
/// (nasobek 64 B). Vrati celkovy pocet (i nad kapacitu `buf`).
pub fn job_processes(job: u32, buf: &mut [u8]) -> Result<usize> {
    let max = buf.len() / abi::job::ProcEntry::SIZE;
    call(nr::JOB_PROCESSES, [job as u64, buf.as_mut_ptr() as u64, max as u64, 0, 0, 0]).map(|n| n as usize)
}

/// Ukonci procesy jobu a jeho potomku a job uzavre (`JOB_KILL`).
pub fn job_kill(job: u32, code: i64) -> Result<()> {
    call(nr::JOB_KILL, [job as u64, code as u64, 0, 0, 0, 0]).map(|_| ())
}

pub fn job_info(job: u32) -> Result<abi::job::JobInfo> {
    let mut buf = [0u8; abi::job::JobInfo::SIZE];
    call(nr::JOB_INFO, [job as u64, buf.as_mut_ptr() as u64, 0, 0, 0, 0])?;
    Ok(abi::job::JobInfo::decode(&buf))
}

pub fn fence_create(initial: u64) -> Result<u32> {
    call(nr::FENCE_CREATE, [initial, 0, 0, 0, 0, 0]).map(|v| v as u32)
}

pub fn fence_signal(h: u32, value: u64, flags: u64) -> Result<()> {
    call(nr::FENCE_SIGNAL, [h as u64, value, flags, 0, 0, 0]).map(|_| ())
}

/// `FENCE_WAIT`; pri chybe plotu vrati i index rozhodujici polozky.
pub fn fence_wait(
    items: &[abi::fence::WaitItem],
    flags: u64,
    deadline: u64,
) -> core::result::Result<usize, (abi::Error, Option<usize>)> {
    let mut index = u32::MAX;
    call(
        nr::FENCE_WAIT,
        [
            items.as_ptr() as u64,
            items.len() as u64,
            flags,
            deadline,
            &mut index as *mut u32 as u64,
            0,
        ],
    )
    .map(|v| v as usize)
    .map_err(|e| (e, (index != u32::MAX).then_some(index as usize)))
}

pub fn fence_wait_async(h: u32, value: u64, port: u32, key: u64) -> Result<()> {
    call(nr::FENCE_WAIT_ASYNC, [h as u64, value, port as u64, key, 0, 0]).map(|_| ())
}

pub fn fence_query(h: u32) -> Result<abi::fence::Info> {
    let mut buf = [0u8; abi::fence::Info::SIZE];
    call(nr::FENCE_QUERY, [h as u64, buf.as_mut_ptr() as u64, 0, 0, 0, 0])?;
    Ok(abi::fence::Info::decode(&buf))
}
