//! Cast loaderu specificka pro aarch64.
//!
//! Prechod do jadra: loader bezi pod UEFI v EL1 se zapnutou MMU (identita
//! podle tabulek firmwaru). Pred skokem do jadra kratce vypne MMU, nastavi
//! vlastni MAIR, TCR a tabulky (TTBR0 = TTBR1 = koren z `paging`), znovu MMU
//! zapne a skoci na vstupni bod jadra v horni polovine. Behem vypnute MMU
//! bezi jen par instrukci nad registry (kod je identicky mapovany, takze PC
//! po zapnuti dal plati).

pub mod paging;

use core::arch::asm;

use boot_info::{DebugSerial, SerialKind, aarch64};
use paging::{Flags, PageTables, SIZE_2M};

/// Citac (CNTVCT_EL0) pro navazani casu firmwaru na hodiny jadra.
pub fn cycle_counter() -> u64 {
    let v: u64;
    // SAFETY: cteni citace generic timeru je v EL1 vzdy povolene; isb
    // zajisti, ze se necte spekulativne driv.
    unsafe { asm!("isb", "mrs {}, cntvct_el0", out(reg) v, options(nomem, nostack)) };
    v
}

/// Zakaz spousteni stranek (XN) ma aarch64 vzdy.
pub fn enable_nx() -> bool {
    true
}

/// Najde ladici seriovou linku v ACPI tabulce SPCR a identicky ji namapuje
/// jako zarizeni, aby do ni jadro mohlo psat hned od startu.
pub fn debug_serial(acpi_rsdp: u64, tables: &mut PageTables) -> DebugSerial {
    let Some(base) = find_spcr_uart(acpi_rsdp) else {
        log::warn!("ACPI SPCR s PL011 nenalezena, jadro nebude mit ladici vystup");
        return DebugSerial::none();
    };
    let device = Flags {
        writable: true,
        executable: false,
        device: true,
    };
    tables
        .map_range_2m(0, base, SIZE_2M, device)
        .expect("nedostatek pameti pro tabulky stranek");
    log::info!("ladici linka PL011 na {base:#x}");
    DebugSerial {
        kind: SerialKind::Pl011,
        _reserved: 0,
        base,
    }
}

/// Adresa PL011 z ACPI SPCR (typ rozhrani 0x03 = PL011, 0x0E = SBSA UART).
fn find_spcr_uart(rsdp: u64) -> Option<u64> {
    if rsdp == 0 {
        return None;
    }
    // SAFETY: RSDP a tabulky ACPI jsou v pameti firmwaru dostupne pres
    // identicke mapovani; delky se kontroluji podle hlavicek.
    unsafe {
        let rsdp = rsdp as *const u8;
        if *rsdp.add(15) < 2 {
            return None;
        }
        let xsdt = (rsdp.add(24) as *const u64).read_unaligned() as *const u8;
        let len = (xsdt.add(4) as *const u32).read_unaligned() as usize;
        let count = len.saturating_sub(36) / 8;
        for i in 0..count {
            let table = (xsdt.add(36 + i * 8) as *const u64).read_unaligned() as *const u8;
            if core::slice::from_raw_parts(table, 4) != b"SPCR" {
                continue;
            }
            let kind = *table.add(36);
            let space = *table.add(40);
            let base = (table.add(44) as *const u64).read_unaligned();
            // Adresni prostor 0 = pamet (MMIO).
            if matches!(kind, 0x03 | 0x0E) && space == 0 && base != 0 {
                return Some(base);
            }
        }
    }
    None
}

/// Kod jadra zapsany pres datovou cache musi byt videt instrukcni cache
/// a CPU, ktera startuji s vypnutou MMU (bez cache): zapsat cache do
/// pameti (PoC) a zneplatnit instrukcni cache. Overi i uroven vyjimek.
pub fn sync_kernel_image(phys: u64, len: u64) {
    let el: u64;
    // SAFETY: cteni CurrentEL nema vedlejsi ucinky.
    unsafe { asm!("mrs {}, CurrentEL", out(reg) el, options(nomem, nostack)) };
    assert_eq!(
        (el >> 2) & 3,
        1,
        "loader musi bezet v EL1 (QEMU bez virtualization=on)"
    );
    let ctr: u64;
    // SAFETY: cteni CTR_EL0 nema vedlejsi ucinky.
    unsafe { asm!("mrs {}, ctr_el0", out(reg) ctr, options(nomem, nostack)) };
    let line = 4u64 << ((ctr >> 16) & 0xF);
    let mut addr = phys & !(line - 1);
    while addr < phys + len {
        // SAFETY: rozsah je obraz jadra nahrany loaderem, identicky mapovany.
        unsafe { asm!("dc cvac, {}", in(reg) addr, options(nostack)) };
        addr += line;
    }
    // SAFETY: bariery a zneplatneni instrukcni cache nemeni data.
    unsafe { asm!("dsb ish", "ic ialluis", "dsb ish", "isb", options(nostack)) };
}

/// Prepne na tabulky jadra, nastavi zasobnik a skoci do jadra.
///
/// # Safety
///
/// - `root` musi byt fyzicka adresa korenove tabulky, ktera identicky mapuje
///   kod loaderu (bezi i po prepnuti), v horni polovine mapuje `stack_top`
///   a vstupni bod jadra.
/// - `stack_top` musi byt zarovnany na 16 bajtu.
/// - Boot services uz musi byt ukoncene.
pub unsafe fn jump_to_kernel(root: u64, stack_top: u64, entry: u64, boot_info: u64) -> ! {
    let mmfr0: u64;
    // SAFETY: cteni ID registru nema vedlejsi ucinky.
    unsafe { asm!("mrs {}, id_aa64mmfr0_el1", out(reg) mmfr0, options(nomem, nostack)) };
    // Tabulky jsou 48bitove: vetsi fyzicky prostor nez 48 bitu (5) nepouzijeme.
    let tcr = aarch64::tcr((mmfr0 & 0xF).min(5));
    // SAFETY: podminky viz dokumentace funkce. Mezi vypnutim a zapnutim MMU
    // se pracuje jen s registry. Tabulky (zapsane pres cache) cte MMU po
    // zapnuti s cache (TCR: Write-Back), takze vidi aktualni obsah.
    unsafe {
        asm!(
            "msr daifset, #0xf",
            "dsb sy",
            "mrs x9, sctlr_el1",
            "bic x10, x9, #1",
            "msr sctlr_el1, x10",
            "isb",
            "msr mair_el1, x1",
            "msr tcr_el1, x2",
            "msr ttbr0_el1, x3",
            "msr ttbr1_el1, x3",
            "isb",
            "tlbi vmalle1",
            "dsb nsh",
            "isb",
            // M (MMU), C (datova cache), I (instrukcni cache).
            "orr x9, x9, #1",
            "orr x9, x9, #(1 << 2)",
            "orr x9, x9, #(1 << 12)",
            "msr sctlr_el1, x9",
            "isb",
            "mov sp, x4",
            "mov x29, #0",
            "mov x30, #0",
            "br x5",
            // Pevne registry: x9 a x10 slouzi jako pracovni.
            in("x0") boot_info,
            in("x1") aarch64::MAIR,
            in("x2") tcr,
            in("x3") root,
            in("x4") stack_top,
            in("x5") entry,
            options(noreturn)
        )
    }
}
