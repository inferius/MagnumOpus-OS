//! Spusteni prvniho procesu (`init`) z initfs.
//!
//! Jadro umi nahrat jediny program - `init`. Vsechny dalsi procesy spousti
//! userspace (libos) pres syscally, jadro o formatu programu nic nevi.
//!
//! `init` dostane bootstrap kanal se zpravou (viz `abi::bootstrap`):
//! handle na vlastni proces a objekt pameti s obrazem initfs.

use alloc::sync::Arc;
use alloc::vec;

use abi::bootstrap::{self, Entry, Header};
use abi::vm::{MAP_EXECUTE, MAP_FIXED, MAP_READ, MAP_WRITE, PAGE_SIZE};
use boot_info::{BootInfo, MemoryKind};

use crate::error::{Error, Result};
use crate::log;
use crate::mm::{PhysAddr, phys};
use crate::object::channel::{Channel, Message};
use crate::object::process::Process;
use crate::object::vmo::MemoryObject;
use crate::object::{KernelObject, Rights};
use crate::sched;

/// Velikost zasobniku hlavniho vlakna init.
const STACK_SIZE: u64 = 256 * 1024;

/// Nahraje a spusti `init` z initfs predaneho loaderem. Obraz zkopiruje do
/// objektu pameti (ten dostane init) a pamet loaderu vrati alokatoru.
/// `Ok(None)`, pokud loader zadny initfs nepredal.
pub fn start_init(boot_info: &BootInfo) -> Result<Option<Arc<Process>>> {
    if boot_info.initfs_len == 0 {
        log::warn!("loader nepredal initfs: zadny proces se nespusti");
        return Ok(None);
    }
    let len = boot_info.initfs_len as usize;
    // SAFETY: loader nahral `len` bajtu na `initfs_phys` do pameti typu
    // Initfs, kterou az do `add_regions` nize nikdo jiny nepouziva.
    let image = unsafe {
        core::slice::from_raw_parts(
            PhysAddr::new(boot_info.initfs_phys)
                .to_virt()
                .as_mut_ptr::<u8>(),
            len,
        )
    };
    let initfs_vmo = MemoryObject::new(boot_info.initfs_len)?;
    initfs_vmo.write(0, image)?;
    let result = load_init(image, &initfs_vmo);
    // Obraz uz je v objektu pameti a init nahrany: pamet loaderu je volna.
    // SAFETY: BootInfo overeny pri startu.
    let regions = unsafe { boot_info.memory_map() };
    phys::add_regions(regions, MemoryKind::Initfs);
    log::info!("initfs: {len} B prevzato");
    result.map(Some)
}

fn load_init(image: &[u8], initfs_vmo: &Arc<MemoryObject>) -> Result<Arc<Process>> {
    let fs = initfs::Initfs::parse(image).map_err(|e| {
        log::error!("initfs je poskozeny: {e}");
        Error::Corrupted
    })?;
    let file = fs.find("init").ok_or_else(|| {
        log::error!("initfs neobsahuje program init");
        Error::NotFound
    })?;
    let elf = elf::ElfFile::parse(file.data).map_err(|e| {
        log::error!("init neni platny program: {e}");
        Error::Corrupted
    })?;

    let process = Process::new("init")?;
    load_segments(&process, &elf)?;
    let stack_top = map_stack(&process)?;

    let (kernel_end, init_end) = Channel::create();
    let bootstrap = process
        .handles()
        .insert(init_end.clone(), init_end.default_rights())?;
    drop(init_end);
    send_bootstrap(&kernel_end, &process, initfs_vmo)?;
    drop(kernel_end);

    let thread = sched::create_user_thread(&process, "init")?;
    sched::start_user_thread(&thread, true, elf.entry, stack_top, bootstrap.0 as u64, 0)?;
    log::info!(
        "init spusten (proces {}), vstup {:#x}",
        process.id,
        elf.entry
    );
    Ok(process)
}

fn load_segments(process: &Process, elf: &elf::ElfFile) -> Result<()> {
    for seg in elf.load_segments() {
        let seg = seg.map_err(|e| {
            log::error!("init: poskozeny segment: {e}");
            Error::Corrupted
        })?;
        let start = seg.vaddr & !(PAGE_SIZE - 1);
        let end = (seg.vaddr + seg.mem_size).next_multiple_of(PAGE_SIZE);
        let vmo = MemoryObject::new(end - start)?;
        vmo.write(seg.vaddr - start, elf.segment_data(&seg))?;
        let mut flags = MAP_READ | MAP_FIXED;
        if seg.flags & elf::PF_W != 0 {
            flags |= MAP_WRITE;
        }
        if seg.flags & elf::PF_X != 0 {
            flags |= MAP_EXECUTE;
        }
        process
            .address_space()
            .map(vmo, 0, end - start, flags, start)?;
    }
    Ok(())
}

fn map_stack(process: &Process) -> Result<u64> {
    let vmo = MemoryObject::new(STACK_SIZE)?;
    let base = process
        .address_space()
        .map(vmo, 0, STACK_SIZE, MAP_READ | MAP_WRITE, 0)?;
    Ok(base + STACK_SIZE)
}

fn send_bootstrap(
    channel: &Channel,
    process: &Arc<Process>,
    initfs: &Arc<MemoryObject>,
) -> Result<()> {
    let kinds = [bootstrap::kind::SELF_PROCESS, bootstrap::kind::INITFS];
    let mut data = vec::Vec::new();
    data.extend_from_slice(
        &Header {
            magic: bootstrap::MAGIC,
            version: bootstrap::VERSION,
            count: 2,
            _reserved: 0,
        }
        .to_bytes(),
    );
    for kind in kinds {
        data.extend_from_slice(&Entry { kind, _reserved: 0 }.to_bytes());
    }
    let initfs_rights = Rights::BASIC | Rights::READ | Rights::MAP | Rights::EXECUTE;
    let handles: vec::Vec<(Arc<dyn KernelObject>, Rights)> = vec![
        (process.clone(), process.default_rights()),
        (initfs.clone(), initfs_rights),
    ];
    channel.write(Message { data, handles })
}
