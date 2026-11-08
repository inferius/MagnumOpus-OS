//! UEFI loader.
//!
//! Postup:
//! 1. nacte `\os\kernel.elf` ze stejneho svazku, ze ktereho byl spusten,
//! 2. pripravi tabulky stranek (identita, prima mapa fyzicke pameti, kernel),
//! 3. zjisti framebuffer (GOP) a ACPI RSDP,
//! 4. ukonci boot services, prelozi mapu pameti do formatu `boot_info`,
//! 5. prepne tabulky stranek a skoci na vstupni bod kernelu s ukazatelem na
//!    `BootInfo` (architekturne zavisle casti v `arch/`).

#![no_std]
#![no_main]

extern crate alloc;

mod arch;
mod pci_id;

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
/// Zaznam padu jadra (pstore), viz [`alloc_pstore`].
const MEM_PSTORE: MemoryType = MemoryType::custom(0x8000_0004);
/// Velikost oblasti pro zaznam padu.
const PSTORE_LEN: u64 = 64 * 1024;
/// Pevne adresy oblasti zaznamu padu (v poradi): stejna adresa pri kazdem
/// startu = zaznam z minuleho behu je po teplem restartu tam, kam ho jadro
/// zapsalo. x86 PC maji RAM od 0 (64 MiB, 128 MiB ...), aarch64 virt
/// a vetsina desek od 1 GiB nebo 2 GiB.
const PSTORE_CANDIDATES: &[u64] = &[
    0x0400_0000,
    0x0800_0000,
    0x1000_0000,
    0x4400_0000,
    0x4800_0000,
    0x8400_0000,
    0x8800_0000,
    0x1_0400_0000,
];

/// Rezerva polozek mapy pameti navic: mezi zjistenim velikosti a ukoncenim
/// boot services se mapa jeste muze rozdelit (alokace firmwaru i nase).
const MEMORY_MAP_SLACK: usize = 128;

const RW: Flags = Flags {
    writable: true,
    executable: false,
    device: false,
};
const RWX: Flags = Flags {
    writable: true,
    executable: true,
    device: false,
};
const DEVICE: Flags = Flags {
    writable: true,
    executable: false,
    device: true,
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
    let device_tree = find_device_tree();
    // Ladici linka (aarch64: z ACPI SPCR, loader ji identicky namapuje jako
    // zarizeni, aby jadro mohlo psat hned od startu).
    let debug_serial = arch::debug_serial(acpi_rsdp, &mut tables);
    let (initfs_phys, initfs_len) = load_initfs();
    let (pstore_phys, pstore_len) = alloc_pstore();
    let boot_flags = ask_boot_mode();

    let (kernel_phys_start, kernel_phys_len) = load_kernel(&elf, &mut tables);
    // aarch64: kod jadra zapsany pres datovou cache musi byt videt
    // instrukcni cache (a CPU startujicim s vypnutou MMU).
    arch::sync_kernel_image(kernel_phys_start, kernel_phys_len);
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
        device_tree,
        debug_serial,
        pstore_phys,
        pstore_len,
        boot_flags,
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

/// Oblast pro zaznam padu: prvni kandidat z [`PSTORE_CANDIDATES`], ktery
/// je cely volna RAM (a uz nese platny zaznam), jinak prvni volny. Cteni
/// kandidata pred alokaci je bezpecne: lezi v konvencni pameti z mapy.
/// `(0, 0)` = zadny kandidat neni volny (jadro pak zaznam pady neulozi).
fn alloc_pstore() -> (u64, u64) {
    let free = |addr: u64| -> bool {
        let Ok(map) = boot::memory_map(MemoryType::LOADER_DATA) else {
            return false;
        };
        map.entries().any(|d| {
            d.ty == MemoryType::CONVENTIONAL
                && d.phys_start <= addr
                && addr + PSTORE_LEN <= d.phys_start + d.page_count * SIZE_4K
        })
    };
    let has_record = |addr: u64| {
        // SAFETY: `addr..addr+PSTORE_LEN` je volna konvencni RAM
        // identicky namapovana firmwarem; jen cteni.
        let area = unsafe { core::slice::from_raw_parts(addr as *const u8, PSTORE_LEN as usize) };
        crashrec::parse(area).is_some()
    };
    let take = |addr: u64| {
        boot::allocate_pages(AllocateType::Address(addr), MEM_PSTORE, (PSTORE_LEN / SIZE_4K) as usize).is_ok()
    };
    let with_record = PSTORE_CANDIDATES.iter().copied().find(|&a| free(a) && has_record(a));
    for addr in with_record.into_iter().chain(PSTORE_CANDIDATES.iter().copied()) {
        if free(addr) && take(addr) {
            log::info!(
                "zaznam padu na {addr:#x}{}",
                if with_record == Some(addr) { " (je v nem zaznam z minuleho behu)" } else { "" }
            );
            return (addr, PSTORE_LEN);
        }
    }
    log::warn!("pro zaznam padu neni volna zadna pevna adresa");
    (0, 0)
}

/// Nabidka pri startu: 2 s na klavesu L = rezim "vytahni logy" (zive USB:
/// zkopiruje logy nainstalovaneho systemu na flashku). Jinak bezny start.
fn ask_boot_mode() -> u64 {
    use uefi::proto::console::text::Key;
    log::info!("Stiskni L = vytahnout logy a zaznamy padu nainstalovaneho systemu na flashku (2 s)");
    for _ in 0..40 {
        let key = uefi::system::with_stdin(|stdin| stdin.read_key());
        if let Ok(Some(Key::Printable(c))) = key
            && matches!(char::from(c), 'l' | 'L')
        {
            log::info!("rezim: vytahnout logy");
            return boot_info::boot_flags::RESCUE_LOGS;
        }
        boot::stall(core::time::Duration::from_millis(50));
    }
    0
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
        // Registry zarizeni se mapuji jako zarizeni (na aarch64 by mapovani
        // jako bezne pameti dovolilo spekulativni cteni registru); do prime
        // mapy nepatri, jadro je mapuje samo (`mm::map_mmio`).
        if matches!(desc.ty, MemoryType::MMIO | MemoryType::MMIO_PORT_SPACE) {
            tables
                .map_range_2m(0, desc.phys_start, len, DEVICE)
                .expect("nedostatek pameti pro tabulky stranek");
            continue;
        }
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
            device: false,
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

/// Vybere vystup GOP: prednost ma kazdy jiny nez virtio-gpu (jeho obraz se
/// do zarizeni prenasi prikazy firmwaru, po ukonceni boot services by
/// zustal prazdny). Virtio-gpu jen kdyz jiny vystup neni.
fn pick_gop() -> Option<uefi::Handle> {
    let handles = boot::find_handles::<GraphicsOutput>().ok()?;
    for &h in &handles {
        if let Some((v, d)) = pci_id::pci_ids_of(h) {
            log::info!("GOP na zarizeni PCI {v:04x}:{d:04x}");
        } else {
            log::info!("GOP mimo PCI (napr. ramfb)");
        }
    }
    let ok = handles.iter().copied().find(|&h| !pci_id::is_virtio_gpu(pci_id::pci_ids_of(h)));
    match ok {
        Some(h) => Some(h),
        None => {
            let h = handles.first().copied()?;
            log::warn!("jediny GOP je virtio-gpu: po startu jadra muze byt obrazovka prazdna");
            Some(h)
        }
    }
}

fn find_framebuffer() -> Framebuffer {
    let Some(handle) = pick_gop() else {
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
    let counter = arch::cycle_counter();
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
        counter,
    }
}

/// GUID tabulky firmwaru s device tree (EFI_DTB_TABLE_GUID).
const DTB_GUID: uefi::Guid = uefi::guid!("b1b621d5-f19c-41a5-830b-d9152c69aae0");

/// Fyzicka adresa device tree od firmwaru, 0 pokud ho nedal.
fn find_device_tree() -> u64 {
    uefi::system::with_config_table(|entries| {
        entries
            .iter()
            .find(|e| e.guid == DTB_GUID)
            .map_or(0, |e| e.address as u64)
    })
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
        MEM_PSTORE => MemoryKind::Pstore,
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
