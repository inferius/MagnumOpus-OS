//! Informace o stroji do logu pri startu (pro hlaseni z fyzickeho PC):
//! procesor (CPUID / MIDR), mapa pameti od loaderu a framebuffer z GOP.
//! Tabulky ACPI loguje `acpi`, zarizeni PCI spravce zarizeni.

use boot_info::{BootInfo, MemoryKind};

use crate::log;

pub fn log_machine(boot_info: &BootInfo) {
    log_cpu();
    // SAFETY: BootInfo je overeny a prima mapa namapovana (po mm::init).
    let regions = unsafe { boot_info.memory_map() };
    let mut ram = 0u64;
    for r in regions {
        if matches!(r.kind, MemoryKind::Usable | MemoryKind::Reclaimable | MemoryKind::BootData | MemoryKind::Kernel | MemoryKind::Initfs) {
            ram += r.len;
        }
        log::info!("stroj: pamet {:#014x}-{:#014x} {:?}", r.start, r.end(), r.kind);
    }
    log::info!("stroj: {} oblasti pameti, RAM {} MiB", regions.len(), ram >> 20);
    let fb = &boot_info.framebuffer;
    if fb.is_present() {
        log::info!(
            "stroj: framebuffer GOP {}x{} (radek {} px, {:?}) na {:#x}",
            fb.width,
            fb.height,
            fb.stride,
            fb.format,
            fb.phys_addr
        );
    } else {
        log::info!("stroj: framebuffer GOP neni");
    }
}

#[cfg(target_arch = "x86_64")]
fn log_cpu() {
    use core::arch::x86_64::__cpuid;
    // CPUID je na x86_64 vzdy; listy 0x8000_0002..4 jen kdyz je hlasi list
    // 0x8000_0000.
    let max_ext = __cpuid(0x8000_0000).eax;
    let leaf1 = __cpuid(1);
    let mut brand = [0u8; 48];
    if max_ext >= 0x8000_0004 {
        for i in 0..3u32 {
            let r = __cpuid(0x8000_0002 + i);
            for (j, v) in [r.eax, r.ebx, r.ecx, r.edx].iter().enumerate() {
                let o = i as usize * 16 + j * 4;
                brand[o..o + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
    }
    let end = brand.iter().position(|&b| b == 0).unwrap_or(brand.len());
    let name = core::str::from_utf8(&brand[..end]).unwrap_or("?").trim();
    let sig = leaf1.eax;
    let family = ((sig >> 8) & 0xF) + if (sig >> 8) & 0xF == 0xF { (sig >> 20) & 0xFF } else { 0 };
    let model = ((sig >> 4) & 0xF) | (((sig >> 16) & 0xF) << 4);
    log::info!("stroj: CPU '{name}', rodina {family:#x}, model {model:#x}, stepping {}", sig & 0xF);
}

#[cfg(target_arch = "aarch64")]
fn log_cpu() {
    let midr: u64;
    // SAFETY: cteni identifikacniho registru, bez vedlejsich ucinku.
    unsafe { core::arch::asm!("mrs {}, midr_el1", out(reg) midr) };
    log::info!(
        "stroj: CPU MIDR {midr:#x} (vyrobce {:#x}, cast {:#x}, varianta {}, revize {})",
        (midr >> 24) & 0xFF,
        (midr >> 4) & 0xFFF,
        (midr >> 20) & 0xF,
        midr & 0xF
    );
}
