//! Spusteni prvniho procesu (`init`) z initfs.
//!
//! Jadro umi nahrat jediny program - `init`. Vsechny dalsi procesy spousti
//! userspace (libos) pres syscally, jadro o formatu programu nic nevi.
//!
//! `init` dostane bootstrap kanal se zpravou (viz `abi::bootstrap`):
//! handle na vlastni proces, korenovy job (bez limitu, se vsemi pravy)
//! a objekt pameti s obrazem initfs.

use alloc::sync::Arc;
use alloc::vec;

use abi::bootstrap::{self, Entry, Header};
use abi::vm::{MAP_EXECUTE, MAP_FIXED, MAP_READ, MAP_WRITE};
use boot_info::{BootInfo, MemoryKind};

use crate::acpi::Platform;
use crate::error::{Error, Result};
use crate::log;
use crate::mm::{PhysAddr, phys};
use crate::object::channel::{Channel, Message};
use crate::object::job::Job;
use crate::object::process::Process;
use crate::object::resource::Resource;
use crate::object::vmo::MemoryObject;
use crate::object::{KernelObject, Rights};
use crate::sched;

/// Velikost zasobniku hlavniho vlakna init.
const STACK_SIZE: u64 = 256 * 1024;

/// Nahraje a spusti `init` z initfs predaneho loaderem. Obraz zkopiruje do
/// objektu pameti (ten dostane init) a pamet loaderu vrati alokatoru.
/// `Ok(None)`, pokud loader zadny initfs nepredal.
pub fn start_init(boot_info: &BootInfo, platform: &Platform) -> Result<Option<Arc<Process>>> {
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
    let root_job = Job::new_root();
    let initfs_vmo = MemoryObject::new(boot_info.initfs_len, Some(&root_job))?;
    initfs_vmo.write(0, image)?;
    let result = load_init(image, &initfs_vmo, &root_job, platform);
    // Obraz uz je v objektu pameti a init nahrany: pamet loaderu je volna.
    // SAFETY: BootInfo overeny pri startu.
    let regions = unsafe { boot_info.memory_map() };
    phys::add_regions(regions, MemoryKind::Initfs);
    log::info!("initfs: {len} B prevzato");
    result.map(Some)
}

fn load_init(
    image: &[u8],
    initfs_vmo: &Arc<MemoryObject>,
    root_job: &Arc<Job>,
    platform: &Platform,
) -> Result<Arc<Process>> {
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

    let process = Process::new("init", root_job.clone())?;
    load_segments(&process, &elf)?;
    let stack_top = map_stack(&process)?;

    let (kernel_end, init_end) = Channel::create();
    let bootstrap = process
        .handles()
        .insert(init_end.clone(), init_end.default_rights())?;
    drop(init_end);
    send_bootstrap(&kernel_end, &process, initfs_vmo, root_job, platform)?;
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
    // Segmenty se zarovnavaji na velikost stranky procesu (ne jadra).
    let page = process.address_space().page_size();
    for seg in elf.load_segments() {
        let seg = seg.map_err(|e| {
            log::error!("init: poskozeny segment: {e}");
            Error::Corrupted
        })?;
        let start = seg.vaddr & !(page - 1);
        let end = (seg.vaddr + seg.mem_size).next_multiple_of(page);
        let vmo = MemoryObject::new(end - start, Some(process.job()))?;
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
    let vmo = MemoryObject::new(STACK_SIZE, Some(process.job()))?;
    let base = process
        .address_space()
        .map(vmo, 0, STACK_SIZE, MAP_READ | MAP_WRITE, 0)?;
    Ok(base + STACK_SIZE)
}

/// Objekt pameti framebufferu predany initu (`FB_CONTROL` overi, ze volajici
/// drzi prave tento objekt).
pub static FB_VMO: crate::sync::SpinLock<Option<Arc<MemoryObject>>> = crate::sync::SpinLock::new(None);

fn send_bootstrap(
    channel: &Channel,
    process: &Arc<Process>,
    initfs: &Arc<MemoryObject>,
    root_job: &Arc<Job>,
    platform: &Platform,
) -> Result<()> {
    let mut entries: vec::Vec<(u32, u32, Arc<dyn KernelObject>, Rights)> = vec![
        (
            bootstrap::kind::SELF_PROCESS,
            if cfg!(feature = "qemu-test") {
                bootstrap::flags::TEST_RUN
            } else {
                0
            },
            process.clone(),
            process.default_rights(),
        ),
        (
            bootstrap::kind::INITFS,
            0,
            initfs.clone(),
            Rights::BASIC | Rights::READ | Rights::MAP | Rights::EXECUTE,
        ),
        (
            bootstrap::kind::JOB,
            0,
            root_job.clone(),
            root_job.default_rights(),
        ),
    ];
    // Pristup k hardwaru: init ho preda spravci zarizeni.
    let root = Resource::new_root();
    let root_rights = root.default_rights();
    entries.push((bootstrap::kind::ROOT_RESOURCE, 0, root, root_rights));
    // Framebuffer z firmware: init ho preda jedinemu procesu (textova
    // konzole). Bez framebufferu (nebo s nezarovnanou adresou) polozka chybi.
    if let Some(fb) = crate::console::framebuffer()
        && fb.phys_addr % crate::arch::paging::PAGE_SIZE == 0
        && fb.len > 0
    {
        let vmo = MemoryObject::new_physical(fb.phys_addr, fb.len, None)?;
        *FB_VMO.lock() = Some(vmo.clone());
        entries.push((
            bootstrap::kind::FRAMEBUFFER,
            0,
            vmo,
            Rights::BASIC | Rights::READ | Rights::WRITE | Rights::MAP,
        ));
    }
    for ecam in &platform.pci_ecam {
        let buses = ecam.bus_end as u64 - ecam.bus_start as u64 + 1;
        // Okno pocina na sbernici bus_start; 1 MiB konfigurace na sbernici.
        let phys = ecam.phys + ecam.bus_start as u64 * (1 << 20);
        let vmo = MemoryObject::new_physical(phys, buses << 20, None)?;
        let arg = ecam.bus_start as u32 | (ecam.bus_end as u32) << 8 | (ecam.segment as u32) << 16;
        entries.push((
            bootstrap::kind::PCI_ECAM,
            arg,
            vmo,
            Rights::BASIC | Rights::READ | Rights::WRITE | Rights::MAP,
        ));
    }

    let mut data = vec::Vec::new();
    let header = Header {
        magic: bootstrap::MAGIC,
        version: bootstrap::VERSION,
        count: entries.len() as u32,
        _reserved: 0,
    };
    data.extend_from_slice(&header.to_bytes());
    let mut handles = vec::Vec::new();
    for (kind, arg, object, rights) in entries {
        data.extend_from_slice(&Entry { kind, arg }.to_bytes());
        handles.push((object, rights));
    }
    channel.write(Message::new(data, handles))
}
