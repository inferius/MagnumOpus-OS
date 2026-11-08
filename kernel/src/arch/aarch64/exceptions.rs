//! Tabulka vektoru vyjimek (VBAR_EL1), ulozeni registru a rozdeleni
//! vyjimek na syscally, chyby programu, preruseni a fatalni chyby jadra.
//!
//! Kazdy vstup tabulky (0x80 bajtu) jen zalozi ramec na zasobniku, ulozi
//! x0/x1, do x1 da druh vyjimky a skoci do spolecne casti. Ta ulozi zbytek
//! registru do [`TrapFrame`] a zavola Rust. Navrat obnovi registry a `eret`.
//!
//! Zasobnik: jadro bezi v EL1 s SP_EL1 (SPSel = 1). Pri vyjimce z EL0 pouzije
//! CPU aktualni hodnotu SP_EL1 - ta je po kazdem navratu do EL0 rovna vrcholu
//! zasobniku jadra vlakna (navrat obnovi `sp` na hodnotu pred ramcem
//! a prvni vstup do EL0 nastavi `sp` na vrchol, viz `user::enter_user`).
//!
//! Omezeni: preteceni zasobniku jadra (ochranna stranka) vyvola dalsi
//! vyjimku na stejnem zasobniku a CPU se zacykli; x86 to resi samostatnym
//! zasobnikem (IST), tady zatim nic. Projevi se zaseknutim, ne poskozenim
//! pameti.

use core::arch::global_asm;
use core::mem::size_of;

use super::{gic, timer};
use crate::fatal::{fatal_stop, report, report_stack_trace};
use crate::{log, trap};

/// Ulozeny stav CPU v okamziku vyjimky. Poradi odpovida `__exc_common`.
#[repr(C)]
#[derive(Debug)]
pub struct TrapFrame {
    /// x0..x30 (x29 = ukazatel ramce, x30 = navratova adresa).
    pub x: [u64; 31],
    /// Zasobnik programu (SP_EL0).
    pub sp_el0: u64,
    /// Adresa instrukce, kam se vyjimka vrati.
    pub elr: u64,
    /// Ulozeny PSTATE.
    pub spsr: u64,
    /// Priciny vyjimky (ESR_EL1) a adresa chyby (FAR_EL1).
    pub esr: u64,
    pub far: u64,
}

const FRAME_SIZE: usize = size_of::<TrapFrame>();
const _: () = assert!(FRAME_SIZE == 288 && FRAME_SIZE % 16 == 0);

// Druh vyjimky (cislo predane vstupem tabulky). 3 = SError v EL1,
// 8 = vyjimka v EL1 se SP_EL0 nebo z AArch32 (jadro ani programy to
// nepouzivaji) - obe konci fatalni chybou.
const KIND_EL1_SYNC: u64 = 0;
const KIND_EL1_IRQ: u64 = 1;
const KIND_EL1_FIQ: u64 = 2;
const KIND_EL0_SYNC: u64 = 4;
const KIND_EL0_IRQ: u64 = 5;
const KIND_EL0_FIQ: u64 = 6;
const KIND_EL0_SERROR: u64 = 7;

global_asm!(
    r#"
    .macro VENTRY kind
    .balign 0x80
    sub sp, sp, #{frame}
    stp x0, x1, [sp, #0]
    mov x1, #\kind
    b __exc_common
    .endm

    .pushsection .text.vectors, "ax"
    .balign 0x800
    .global __exc_vectors
__exc_vectors:
    VENTRY 8
    VENTRY 8
    VENTRY 8
    VENTRY 8
    VENTRY 0
    VENTRY 1
    VENTRY 2
    VENTRY 3
    VENTRY 4
    VENTRY 5
    VENTRY 6
    VENTRY 7
    VENTRY 8
    VENTRY 8
    VENTRY 8
    VENTRY 8

__exc_common:
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    mrs x2, sp_el0
    stp x30, x2, [sp, #240]
    mrs x2, elr_el1
    mrs x3, spsr_el1
    stp x2, x3, [sp, #256]
    mrs x2, esr_el1
    mrs x3, far_el1
    stp x2, x3, [sp, #272]
    mov x0, sp
    bl {dispatch}
    ldp x2, x3, [sp, #256]
    msr elr_el1, x2
    msr spsr_el1, x3
    ldp x30, x2, [sp, #240]
    msr sp_el0, x2
    ldp x0, x1, [sp, #0]
    ldp x2, x3, [sp, #16]
    ldp x4, x5, [sp, #32]
    ldp x6, x7, [sp, #48]
    ldp x8, x9, [sp, #64]
    ldp x10, x11, [sp, #80]
    ldp x12, x13, [sp, #96]
    ldp x14, x15, [sp, #112]
    ldp x16, x17, [sp, #128]
    ldp x18, x19, [sp, #144]
    ldp x20, x21, [sp, #160]
    ldp x22, x23, [sp, #176]
    ldp x24, x25, [sp, #192]
    ldp x26, x27, [sp, #208]
    ldp x28, x29, [sp, #224]
    add sp, sp, #{frame}
    eret
    .popsection
    "#,
    frame = const FRAME_SIZE,
    dispatch = sym trap_dispatch,
);

unsafe extern "C" {
    static __exc_vectors: u8;
}

/// Nahraje tabulku vektoru na aktualnim CPU a zajisti SP_EL1 pro EL1.
///
/// # Safety
///
/// Volat pri startu kazdeho CPU se zakazanymi prerusenimi.
pub unsafe fn init_cpu() {
    // SAFETY: tabulka je staticka a zarovnana na 2 KiB; SPSel = 1 jen
    // potvrdi zasobnik, na kterem jadro bezi.
    unsafe {
        super::regs::msr!("vbar_el1", &raw const __exc_vectors as u64);
        core::arch::asm!("msr spsel, #1", options(nostack));
    }
    super::regs::isb();
}

// Trida vyjimky (ESR_EL1.EC).
const EC_UNKNOWN: u64 = 0x00;
const EC_SVC64: u64 = 0x15;
const EC_IABT_LOWER: u64 = 0x20;
const EC_IABT_CUR: u64 = 0x21;
const EC_PC_ALIGN: u64 = 0x22;
const EC_DABT_LOWER: u64 = 0x24;
const EC_DABT_CUR: u64 = 0x25;
const EC_SP_ALIGN: u64 = 0x26;
const EC_BRK: u64 = 0x3C;

// Cisla chyb pro ukonceny proces (ABI, stejna jako vektory x86).
const FAULT_INVALID_OPCODE: u8 = 6;
const FAULT_BREAKPOINT: u8 = 3;
const FAULT_GENERAL: u8 = 13;
const FAULT_PAGE: u8 = 14;
const FAULT_ALIGNMENT: u8 = 17;

extern "C" fn trap_dispatch(frame: &mut TrapFrame, kind: u64) {
    match kind {
        KIND_EL0_SYNC => el0_sync(frame),
        KIND_EL1_SYNC => el1_sync(frame),
        KIND_EL0_IRQ | KIND_EL1_IRQ => {
            irq();
            trap::after_interrupt();
        }
        // Vsechna preruseni jsou ve skupine 1 (IRQ); FIQ by byla chyba
        // nastaveni, SError chyba hardwaru.
        KIND_EL0_FIQ | KIND_EL1_FIQ => log::warn!("necekane FIQ"),
        // SError, vyjimka v EL1 se SP_EL0 nebo z AArch32: chyba, ktera
        // nema nastat.
        _ => fatal(frame, kind),
    }
    if (KIND_EL0_SYNC..=KIND_EL0_SERROR).contains(&kind) {
        trap::return_to_user();
    }
}

fn el0_sync(frame: &mut TrapFrame) {
    let ec = frame.esr >> 26;
    let iss = frame.esr & 0x1FF_FFFF;
    let code = match ec {
        EC_SVC64 => {
            // Zasobnik je jadra, registry ulozene: preruseni uz smi prijit.
            super::interrupts::enable();
            let args = [
                frame.x[0], frame.x[1], frame.x[2], frame.x[3], frame.x[4], frame.x[5],
            ];
            frame.x[0] = trap::syscall(frame.x[8], args);
            super::interrupts::disable();
            return;
        }
        EC_DABT_LOWER | EC_IABT_LOWER => FAULT_PAGE,
        EC_PC_ALIGN | EC_SP_ALIGN => FAULT_ALIGNMENT,
        EC_BRK => FAULT_BREAKPOINT,
        EC_UNKNOWN => FAULT_INVALID_OPCODE,
        _ => FAULT_GENERAL,
    };
    let addr = if code == FAULT_PAGE { frame.far } else { 0 };
    trap::user_fault(code, iss, frame.elr, addr)
}

fn el1_sync(frame: &mut TrapFrame) {
    if frame.esr >> 26 == EC_BRK {
        log::info!("breakpoint na {:#x}", frame.elr);
        frame.elr += 4;
        return;
    }
    fatal(frame, KIND_EL1_SYNC)
}

fn irq() {
    let Some(intid) = gic::acknowledge() else {
        return; // spurious
    };
    match intid {
        timer::INTID => {
            timer::on_interrupt();
            gic::end_of_interrupt(intid);
            trap::timer_interrupt();
        }
        gic::SGI_RESCHEDULE => {
            gic::end_of_interrupt(intid);
            trap::reschedule_ipi();
        }
        gic::SGI_TLB_FLUSH => {
            gic::end_of_interrupt(intid);
            trap::tlb_flush_ipi();
        }
        gic::SGI_HALT => super::halt_forever(),
        id if (gic::LPI_FIRST..=super::DEVICE_IRQ_LAST).contains(&id) => {
            gic::end_of_interrupt(intid);
            trap::device_irq(id);
        }
        id if (gic::SPI_FIRST..=gic::SPI_LAST).contains(&id) => {
            // Linku spoustenou urovni zamaskuje `dispatch` (do potvrzeni
            // ovladacem) - az pak EOI, aby se preruseni hned neopakovalo.
            match super::spi_irq(id) {
                Some(irq) => trap::device_irq(irq),
                None => log::warn!("SPI {id} bez objektu preruseni"),
            }
            gic::end_of_interrupt(intid);
        }
        id => {
            gic::end_of_interrupt(intid);
            log::warn!("necekane preruseni, INTID {id}");
        }
    }
}

fn ec_name(ec: u64) -> &'static str {
    match ec {
        EC_UNKNOWN => "nedefinovana instrukce",
        EC_SVC64 => "SVC",
        EC_IABT_LOWER | EC_IABT_CUR => "Instruction Abort",
        EC_PC_ALIGN => "nezarovnane PC",
        EC_DABT_LOWER | EC_DABT_CUR => "Data Abort",
        EC_SP_ALIGN => "nezarovnany SP",
        EC_BRK => "BRK",
        0x07 => "pristup k FPU/SIMD",
        0x0E => "neplatny stav (IL)",
        0x2F => "SError",
        _ => "jina",
    }
}

fn fatal(f: &TrapFrame, kind: u64) -> ! {
    let ec = f.esr >> 26;
    report(format_args!(
        "KERNEL: neosetrena vyjimka {} na cpu{}, druh {kind}, ESR {:#x} (EC {ec:#x})",
        ec_name(ec),
        trap::current_cpu_index(),
        f.esr
    ));
    if matches!(
        ec,
        EC_DABT_CUR | EC_DABT_LOWER | EC_IABT_CUR | EC_IABT_LOWER
    ) {
        let wnr = f.esr & (1 << 6) != 0;
        report(format_args!(
            "  adresa {:#x}: {}, {}",
            f.far,
            if wnr { "zapis" } else { "cteni" },
            if f.spsr & 0xF == 0 { "user" } else { "kernel" },
        ));
    }
    report(format_args!(
        "  pc {:#018x}  spsr {:#x}  sp_el0 {:#x}  sp {:#x}",
        f.elr,
        f.spsr,
        f.sp_el0,
        f as *const TrapFrame as u64 + FRAME_SIZE as u64
    ));
    // Bez alokace (heap muze byt zamceny): po ctyrech registrech.
    for i in (0..28).step_by(4) {
        report(format_args!(
            "  x{:<2} {:#018x}  x{:<2} {:#018x}  x{:<2} {:#018x}  x{:<2} {:#018x}",
            i,
            f.x[i],
            i + 1,
            f.x[i + 1],
            i + 2,
            f.x[i + 2],
            i + 3,
            f.x[i + 3]
        ));
    }
    report(format_args!(
        "  x28 {:#018x}  x29 {:#018x}  x30 {:#018x}",
        f.x[28], f.x[29], f.x[30]
    ));
    report_stack_trace(Some(f.x[29]));
    fatal_stop()
}
