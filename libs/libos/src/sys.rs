//! Syrova volani jadra. Programy maji pouzivat typy z libos; tyto funkce
//! jsou verejne kvuli testum rozhrani a nizkourovnovym nastrojum.

use core::arch::asm;

use abi::syscall as nr;
use abi::{Error, Result};

/// Syscall se sesti argumenty. Jadro zachova vsechny registry krome
/// `rax` (vysledek) a `rcx`, `r11` (prepise je instrukce `syscall`).
///
/// # Safety
///
/// Ukazatele v argumentech musi odpovidat popisu daneho syscallu.
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

#[inline(always)]
fn call(n: u64, a: [u64; 6]) -> Result<u64> {
    // SAFETY: vsechny obalky nize predavaji ukazatele na platne buffery
    // odpovidajici velikosti.
    abi::error::decode(unsafe { raw(n, a[0], a[1], a[2], a[3], a[4], a[5]) })
}

pub fn debug_log(text: &str) {
    let _ = call(
        nr::DEBUG_LOG,
        [text.as_ptr() as u64, text.len() as u64, 0, 0, 0, 0],
    );
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
    call(nr::CLOCK_SET, [resource as u64, buf.as_ptr() as u64, flags, 0, 0, 0]).map(|r| r == 1)
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

pub fn vmo_create(size: u64) -> Result<u32> {
    call(nr::VMO_CREATE, [size, 0, 0, 0, 0, 0]).map(|v| v as u32)
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

pub fn process_create(name: &str) -> Result<u32> {
    call(
        nr::PROCESS_CREATE,
        [name.as_ptr() as u64, name.len() as u64, 0, 0, 0, 0],
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

pub fn vmo_create_physical(resource: u32, phys: u64, size: u64) -> Result<u32> {
    call(
        nr::VMO_CREATE_PHYSICAL,
        [resource as u64, phys, size, 0, 0, 0],
    )
    .map(|v| v as u32)
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
