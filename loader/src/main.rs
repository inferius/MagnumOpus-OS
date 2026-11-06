//! UEFI loader.
//!
//! Postup:
//! 1. nacte `\os\kernel.elf` ze stejneho svazku, ze ktereho byl spusten,
//! 2. pripravi tabulky stranek (identita, prima mapa fyzicke pameti, kernel),
//! 3. zjisti framebuffer (GOP) a ACPI RSDP,
//! 4. ukonci boot services, prelozi mapu pameti do formatu `boot_info`,
//! 5. prepne CR3 a skoci na vstupni bod kernelu s ukazatelem na `BootInfo`.

#![no_std]
#![no_main]

extern crate alloc;

mod arch;

use alloc::vec::Vec;
use core::mem::size_of;

use boot_info::{
    FirmwareTime,
    BOOT_INFO_MAGIC, BOOT_INFO_VERSION, BootInfo, Framebuffer, KERNEL_BASE, KERNEL_STACK_SIZE,
    MemoryKind, MemoryRegion, PHYS_MAP_BASE, PixelFormat,
};
use uefi::boot::{self, AllocateType, MemoryType, OpenProtocolAttributes, OpenProtocolParams};
use uefi::mem::memory_map::MemoryMap;
use uefi::prelude::*;
use uefi::proto::console::gop::{self, GraphicsOutput};
use uefi::table::cfg::ConfigTableEntry;
use uefi::{CStr16, cstr16};

use arch::paging::{Flags, PageTables, SIZE_4K};
use elf::{ElfFile, PF_W, PF_X, Segment};

/// Cesta ke kernelu na systemovem oddilu (ESP).
const KERNEL_PATH: &CStr16 = cstr16!("\\os\\kernel.elf");

/// Vlastni UEFI typy pameti (rozsah 0x8000_0000.. je vyhrazen pro OS loader),
/// diky nim kernel v mape pameti pozna, co mu loader pripravil.
const MEM_KERNEL: MemoryType = MemoryType::custom(0x8000_0001);
const MEM_BOOT_DATA: MemoryType = MemoryType::custom(0x8000_0002);
/// Cesta k obrazu initfs (nepovinny; bez nej jadro nespusti zadny proces).
const INITFS_PATH: &CStr16 = cstr16!("\\os\\initfs.img");

const MEM_INITFS: MemoryType = MemoryType::custom(0x8000_0003);

/// Rezerva polozek mapy pameti navic: mezi zjistenim velikosti a ukoncenim
/// boot services se mapa jeste muze rozdelit (alokace firmwaru i nase).
const MEMORY_MAP_SLACK: usize = 128;

const RW: Flags = Flags {
    writable: true,
    executable: false,
};
const RWX: Flags = Flags {
    writable: true,
    executable: true,
};

#[entry]
fn main() -> Status {
    uefi::helpers::init().expect("inicializace UEFI helperu selhala");
    log::info!("os loader {}", env!("CARGO_PKG_VERSION"));

    let kernel_file = read_kernel();
    let elf = ElfFile::parse(&kernel_file).expect("kernel neni platny ELF");

    let nx = arch::enable_nx();
    let mut tables = PageTables::new(MEM_BOOT_DATA, nx).expect("alokace PML4 selhala");
    map_physical_memory(&mut tables);

    let framebuffer = find_framebuffer();
    if framebuffer.is_present() {
        tables
            .map_range_2m(PHYS_MAP_BASE, framebuffer.phys_addr, framebuffer.len, RW)
            .expect("mapovani framebufferu selhalo");
    }
    let acpi_rsdp = find_acpi_rsdp();
    let (initfs_phys, initfs_len) = load_initfs();

    let (kernel_phys_start, kernel_phys_len) = load_kernel(&elf, &mut tables);
    let entry = elf.entry;
    drop(kernel_file);

    // Zasobnik kernelu, pristupny pres primou mapu fyzicke pameti.
    let stack_phys = alloc_pages(KERNEL_STACK_SIZE / SIZE_4K, MEM_BOOT_DATA);
    let stack_top = PHYS_MAP_BASE + stack_phys + KERNEL_STACK_SIZE;

    // BootInfo a buffer pro mapu pameti se musi alokovat jeste pred
    // ukoncenim boot services.
    let boot_info_phys = alloc_pages(1, MEM_BOOT_DATA);
    let map_capacity = boot::memory_map(MemoryType::LOADER_DATA)
        .expect("ziskani mapy pameti selhalo")
        .len()
        + MEMORY_MAP_SLACK;
    let map_bytes = (map_capacity * size_of::<MemoryRegion>()) as u64;
    let map_phys = alloc_pages(map_bytes.div_ceil(SIZE_4K), MEM_BOOT_DATA);

    log::info!(
        "kernel {kernel_phys_len:#x} B na {kernel_phys_start:#x}, vstup {entry:#x}, NX: {nx}"
    );
    // Cas az tesne pred koncem boot services (co nejmensi posun do jadra).
    let firmware_time = read_firmware_time();
    log::info!("ukoncuji boot services a startuji kernel");

    // SAFETY: zadne protokoly ani pool alokace uz nedrzime (kernel_file je
    // uvolneny, GOP protokol zavreny v `find_framebuffer`). Od teto chvile
    // se nesmi volat zadne boot services ani logovat.
    let uefi_map = unsafe { boot::exit_boot_services(None) };

    // SAFETY: `map_phys` je nase alokace s mistem pro `map_capacity` polozek,
    // pristupna pres identicke mapovani; nic jineho na ni neukazuje.
    let regions =
        unsafe { core::slice::from_raw_parts_mut(map_phys as *mut MemoryRegion, map_capacity) };
    let map_len = convert_memory_map(&uefi_map, regions);

    let boot_info = BootInfo {
        magic: BOOT_INFO_MAGIC,
        version: BOOT_INFO_VERSION,
        _reserved: 0,
        phys_offset: PHYS_MAP_BASE,
        memory_map: PHYS_MAP_BASE + map_phys,
        memory_map_len: map_len as u64,
        framebuffer,
        acpi_rsdp,
        kernel_phys_start,
        kernel_phys_len,
        initfs_phys,
        initfs_len,
        firmware_time,
    };
    // SAFETY: `boot_info_phys` je cela stranka alokovana pro BootInfo,
    // zarovnana na 4 KiB (vic nez zarovnani BootInfo).
    unsafe { (boot_info_phys as *mut BootInfo).write(boot_info) };

    // SAFETY: tabulky mapuji identicky veskerou pamet z mapy UEFI (vcetne
    // kodu loaderu), zasobnik a BootInfo v prime mape a kernel na jeho
    // adresach. Boot services jsou ukoncene. stack_top je zarovnany na 4 KiB.
    unsafe {
        arch::jump_to_kernel(
            tables.root(),
            stack_top,
            entry,
            PHYS_MAP_BASE + boot_info_phys,
        )
    }
}

fn boot_fs() -> uefi::fs::FileSystem {
    let fs = boot::get_image_file_system(boot::image_handle())
        .expect("nelze otevrit souborovy system, ze ktereho byl loader spusten");
    uefi::fs::FileSystem::new(fs)
}

/// Nahraje initfs do stranek typu MEM_INITFS (jadro je po prevzeti uvolni).
/// Vraci (fyzicka adresa, delka), (0, 0) pokud soubor chybi.
fn load_initfs() -> (u64, u64) {
    let data = match boot_fs().read(INITFS_PATH) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("initfs nenalezen ({e:?}), jadro nespusti zadny proces");
            return (0, 0);
        }
    };
    let len = data.len() as u64;
    let phys = alloc_pages(len.div_ceil(SIZE_4K).max(1), MEM_INITFS);
    // SAFETY: cil je prave alokovany blok o velikosti alespon `len`,
    // zdroj je samostatny Vec.
    unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), phys as *mut u8, data.len()) };
    log::info!("initfs {len} B na {phys:#x}");
    (phys, len)
}

fn read_kernel() -> Vec<u8> {
    boot_fs()
        .read(KERNEL_PATH)
        .expect("nelze nacist \\os\\kernel.elf")
}

fn alloc_pages(count: u64, ty: MemoryType) -> u64 {
    let ptr = boot::allocate_pages(AllocateType::AnyPages, ty, count as usize)
        .expect("nedostatek pameti");
    // SAFETY: prave alokovany blok `count` stranek, nikdo jiny ho nepouziva.
    unsafe { ptr.as_ptr().write_bytes(0, (count * SIZE_4K) as usize) };
    ptr.as_ptr() as u64
}

/// Namapuje vsechny oblasti z aktualni mapy pameti identicky (kvuli prechodu
/// z loaderu do kernelu) a na `PHYS_MAP_BASE` (prima mapa pro kernel).
///
/// Vse, co loader alokuje pozdeji, lezi v oblastech, ktere v teto mape uz
/// jsou (jen se rozdeli), takze bude namapovane taky.
fn map_physical_memory(tables: &mut PageTables) {
    let map = boot::memory_map(MemoryType::LOADER_DATA).expect("ziskani mapy pameti selhalo");
    for desc in map.entries() {
        let len = desc.page_count * SIZE_4K;
        tables
            .map_range_2m(0, desc.phys_start, len, RWX)
            .and_then(|()| tables.map_range_2m(PHYS_MAP_BASE, desc.phys_start, len, RW))
            .expect("nedostatek pameti pro tabulky stranek");
    }
}

/// Nahraje PT_LOAD segmenty do jednoho souvisleho fyzickeho bloku
/// a namapuje je 4 KiB strankami podle jejich prav.
/// Vraci fyzicky zacatek a delku bloku.
fn load_kernel(elf: &ElfFile, tables: &mut PageTables) -> (u64, u64) {
    let segments: Vec<Segment> = elf
        .load_segments()
        .collect::<Result<_, _>>()
        .expect("poskozene program headery kernelu");
    assert!(!segments.is_empty(), "kernel nema zadny PT_LOAD segment");

    let start = segments.iter().map(|s| s.vaddr).min().unwrap() & !(SIZE_4K - 1);
    let end = segments
        .iter()
        .map(|s| s.vaddr + s.mem_size)
        .max()
        .unwrap()
        .next_multiple_of(SIZE_4K);
    assert!(
        start >= KERNEL_BASE,
        "kernel musi byt linkovany od KERNEL_BASE"
    );
    assert!(
        (start..end).contains(&elf.entry),
        "vstupni bod kernelu lezi mimo jeho segmenty"
    );

    let len = end - start;
    let phys = alloc_pages(len / SIZE_4K, MEM_KERNEL);

    for seg in &segments {
        let data = elf.segment_data(seg);
        let dst = phys + (seg.vaddr - start);
        // SAFETY: `dst..dst + file_size` lezi uvnitr bloku `phys..phys + len`
        // (vaddr + mem_size <= end a file_size <= mem_size), zdroj je
        // samostatny Vec; zbytek segmentu (.bss) je uz vynulovany.
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), dst as *mut u8, data.len()) };

        let flags = Flags {
            writable: seg.flags & PF_W != 0,
            executable: seg.flags & PF_X != 0,
        };
        let first = seg.vaddr & !(SIZE_4K - 1);
        let last = (seg.vaddr + seg.mem_size).next_multiple_of(SIZE_4K);
        for virt in (first..last).step_by(SIZE_4K as usize) {
            tables
                .map_4k(virt, phys + (virt - start), flags)
                .expect("nedostatek pameti pro tabulky stranek");
        }
    }
    (phys, len)
}

fn find_framebuffer() -> Framebuffer {
    let Ok(handle) = boot::get_handle_for_protocol::<GraphicsOutput>() else {
        log::warn!("GOP neni k dispozici, framebuffer nebude");
        return Framebuffer::none();
    };
    let params = OpenProtocolParams {
        handle,
        agent: boot::image_handle(),
        controller: None,
    };
    // SAFETY: GetProtocol neodpoji ostatni uzivatele GOP (textova konzole
    // firmwaru dal funguje). GOP jen cteme a protokol zavreme pred
    // ukoncenim boot services (drop na konci funkce).
    let gop = unsafe {
        boot::open_protocol::<GraphicsOutput>(params, OpenProtocolAttributes::GetProtocol)
    };
    let Ok(mut gop) = gop else {
        log::warn!("GOP nelze otevrit, framebuffer nebude");
        return Framebuffer::none();
    };

    let info = gop.current_mode_info();
    let format = match info.pixel_format() {
        gop::PixelFormat::Rgb => PixelFormat::Rgb,
        gop::PixelFormat::Bgr => PixelFormat::Bgr,
        other => {
            log::warn!("nepodporovany format pixelu {other:?}, framebuffer nebude");
            return Framebuffer::none();
        }
    };
    let (width, height) = info.resolution();
    let mut fb = gop.frame_buffer();
    let phys_addr = fb.as_mut_ptr() as u64;
    log::info!("framebuffer {width}x{height} {format:?} na {phys_addr:#x}");
    Framebuffer {
        phys_addr,
        virt_addr: PHYS_MAP_BASE + phys_addr,
        len: fb.size() as u64,
        width: width as u32,
        height: height as u32,
        stride: info.stride() as u32,
        format,
    }
}

/// Precte realny cas z UEFI (`GetTime`) a prevede ho na sekundy od 1970.
/// Kdyz firmware pasmo nezna, cas se bere jako UTC (jadro o tom vi
/// z `zone_minutes`).
fn read_firmware_time() -> FirmwareTime {
    let tsc = arch::rdtsc();
    let t = match uefi::runtime::get_time() {
        Ok(t) if t.is_valid().is_ok() => t,
        Ok(_) => {
            log::warn!("UEFI vratil neplatny cas");
            return FirmwareTime::none();
        }
        Err(e) => {
            log::warn!("UEFI GetTime selhal: {:?}", e.status());
            return FirmwareTime::none();
        }
    };
    let civil = abi::time::Civil {
        year: t.year() as i64,
        month: t.month(),
        day: t.day(),
        hour: t.hour(),
        minute: t.minute(),
        second: t.second(),
        nanos: t.nanosecond(),
    };
    let Some(local) = abi::time::WallTime::from_civil(&civil) else {
        log::warn!("UEFI cas mimo kalendar: {t:?}");
        return FirmwareTime::none();
    };
    // UEFI: mistni cas = UTC + pasmo, tedy UTC = mistni - pasmo.
    let (zone, utc) = match t.time_zone() {
        Some(z) => (z, local.add_ns(-(z as i128) * 60_000_000_000)),
        None => (FirmwareTime::ZONE_UNSPECIFIED, local),
    };
    log::info!(
        "cas firmwaru: {:04}-{:02}-{:02} {:02}:{:02}:{:02} ({})",
        civil.year,
        civil.month,
        civil.day,
        civil.hour,
        civil.minute,
        civil.second,
        if zone == FirmwareTime::ZONE_UNSPECIFIED { "pasmo neuvedeno, beru jako UTC" } else { "s pasmem" }
    );
    FirmwareTime {
        valid: 1,
        zone_minutes: zone,
        _reserved: 0,
        secs: utc.secs,
        nanos: utc.nanos,
        _reserved2: 0,
        tsc,
    }
}

fn find_acpi_rsdp() -> u64 {
    let rsdp = uefi::system::with_config_table(|entries| {
        entries
            .iter()
            .find(|e| e.guid == ConfigTableEntry::ACPI2_GUID)
            .map(|e| e.address as u64)
    });
    if rsdp.is_none() {
        log::warn!("ACPI 2.0 RSDP nenalezen");
    }
    rsdp.unwrap_or(0)
}

fn memory_kind(ty: MemoryType) -> MemoryKind {
    match ty {
        MemoryType::CONVENTIONAL => MemoryKind::Usable,
        MemoryType::LOADER_CODE
        | MemoryType::LOADER_DATA
        | MemoryType::BOOT_SERVICES_CODE
        | MemoryType::BOOT_SERVICES_DATA => MemoryKind::Reclaimable,
        MEM_KERNEL => MemoryKind::Kernel,
        MEM_BOOT_DATA => MemoryKind::BootData,
        MEM_INITFS => MemoryKind::Initfs,
        MemoryType::ACPI_RECLAIM => MemoryKind::AcpiReclaimable,
        MemoryType::ACPI_NON_VOLATILE => MemoryKind::AcpiNvs,
        MemoryType::RUNTIME_SERVICES_CODE | MemoryType::RUNTIME_SERVICES_DATA => {
            MemoryKind::FirmwareRuntime
        }
        MemoryType::MMIO | MemoryType::MMIO_PORT_SPACE => MemoryKind::Mmio,
        MemoryType::UNUSABLE => MemoryKind::Unusable,
        _ => MemoryKind::Reserved,
    }
}

/// Prelozi mapu UEFI do `out`, seradi ji a spoji sousedni oblasti stejneho
/// druhu. Vraci pocet zapsanych polozek. Bezi po ukonceni boot services,
/// takze nesmi alokovat ani logovat.
fn convert_memory_map(uefi_map: &impl MemoryMap, out: &mut [MemoryRegion]) -> usize {
    let mut len = 0;
    for desc in uefi_map.entries() {
        if len == out.len() {
            // Nemelo by nastat (viz MEMORY_MAP_SLACK); zbytek mapy se zahodi,
            // coz je bezpecne - kernel jen nepouzije cast pameti.
            break;
        }
        out[len] = MemoryRegion {
            start: desc.phys_start,
            len: desc.page_count * SIZE_4K,
            kind: memory_kind(desc.ty),
            _reserved: 0,
        };
        len += 1;
    }

    let regions = &mut out[..len];
    regions.sort_unstable_by_key(|r| r.start);

    let mut merged = 0;
    for i in 0..len {
        let r = regions[i];
        if merged > 0 {
            let last = &mut regions[merged - 1];
            if last.kind == r.kind && last.end() == r.start {
                last.len += r.len;
                continue;
            }
        }
        regions[merged] = r;
        merged += 1;
    }
    merged
}
