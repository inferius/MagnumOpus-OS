//! Start dalsich CPU pres PSCI (`CPU_ON`).
//!
//! Firmware (na QEMU virt primo QEMU) zapne CPU v EL1 s vypnutou MMU na
//! fyzicke adrese vstupniho stubu `__ap_entry` a s `x0` = kontext (fyzicka
//! adresa bloku parametru). Stub nastavi MAIR, TCR, TTBR0 i TTBR1 na tabulky
//! jadra, zapne MMU a skoci do jadra v horni polovine. Stranka se stubem
//! musi byt v tabulkach jadra docasne identicky namapovana (po zapnuti MMU
//! se dalsi instrukce cte z teze adresy), podobne jako trampolina na x86.
//!
//! Blok parametru se cte s vypnutou MMU (bez cache), proto se po zapisu
//! zapise cache do pameti; stub ho cely nacte jeste pred zapnutim MMU.

use core::arch::{asm, global_asm};

use super::regs::{dcache_clean_invalidate, hw_id_to_mpidr, mrs};
use super::timer;
use crate::arch::api::CpuStarter;
use crate::log;
use crate::mm::{self, PhysAddr, VirtAddr};

/// PSCI CPU_ON (SMC64 konvence).
const PSCI_CPU_ON: u64 = 0xC400_0003;
const PSCI_SUCCESS: i64 = 0;
const PSCI_ALREADY_ON: i64 = -4;

global_asm!(
    r#"
    .pushsection .text.ap_entry, "ax"
    .balign 4096
    .global __ap_entry
__ap_entry:
    // Vse z bloku parametru nacist pred zapnutim MMU: po zapnuti je
    // identicky namapovana jen stranka se stubem, ne blok parametru.
    ldr x3, [x0, #24]
    ldr x2, [x0, #32]
    ldr x4, [x0, #40]
    ldr x1, [x0, #0]
    msr mair_el1, x1
    ldr x1, [x0, #8]
    msr tcr_el1, x1
    ldr x1, [x0, #16]
    msr ttbr0_el1, x1
    msr ttbr1_el1, x1
    isb
    tlbi vmalle1
    dsb nsh
    isb
    mrs x1, sctlr_el1
    orr x1, x1, #1
    orr x1, x1, #(1 << 2)
    orr x1, x1, #(1 << 12)
    msr sctlr_el1, x1
    isb
    mov sp, x3
    mov x0, x4
    mov x29, xzr
    mov x30, xzr
    br x2
    .balign 4096
    .popsection
    "#
);

unsafe extern "C" {
    static __ap_entry: u8;
}

/// Blok parametru pro stub (poradi pouziva `__ap_entry`).
#[repr(C)]
struct Params {
    mair: u64,
    tcr: u64,
    ttbr: u64,
    stack_top: u64,
    entry: u64,
    arg: u64,
}

/// Pripraveny start CPU: identicky namapovany stub a stranka parametru.
pub struct Psci {
    /// Fyzicka adresa stranky se stubem.
    stub: u64,
    /// Fyzicka adresa bloku parametru.
    params: PhysAddr,
    hvc: bool,
}

/// Pripravi start CPU (`api::Smp::prepare_cpu_start`).
pub fn prepare(hvc: bool) -> Option<Psci> {
    let stub_virt = &raw const __ap_entry as u64;
    let Some(stub) = mm::translate(VirtAddr::new(stub_virt)) else {
        log::warn!("stub startu CPU neni namapovany, bezim jen na jednom CPU");
        return None;
    };
    let params = mm::phys::alloc_frame_zeroed()?;
    mm::map_identity_page(stub);
    Some(Psci {
        stub: stub.as_u64(),
        params,
        hvc,
    })
}

/// Volani PSCI: x0 = funkce, x1-x3 argumenty, vysledek v x0.
fn psci_call(hvc: bool, func: u64, a1: u64, a2: u64, a3: u64) -> i64 {
    let ret: u64;
    // SAFETY: volani firmwaru (PSCI); podle SMCCC muze zmenit x0-x17.
    unsafe {
        if hvc {
            asm!("hvc #0", inout("x0") func => ret, inout("x1") a1 => _, inout("x2") a2 => _,
                 inout("x3") a3 => _, out("x4") _, out("x5") _, out("x6") _, out("x7") _,
                 out("x8") _, out("x9") _, out("x10") _, out("x11") _, out("x12") _,
                 out("x13") _, out("x14") _, out("x15") _, out("x16") _, out("x17") _,
                 options(nostack));
        } else {
            asm!("smc #0", inout("x0") func => ret, inout("x1") a1 => _, inout("x2") a2 => _,
                 inout("x3") a3 => _, out("x4") _, out("x5") _, out("x6") _, out("x7") _,
                 out("x8") _, out("x9") _, out("x10") _, out("x11") _, out("x12") _,
                 out("x13") _, out("x14") _, out("x15") _, out("x16") _, out("x17") _,
                 options(nostack));
        }
    }
    ret as i64
}

impl CpuStarter for Psci {
    fn start(
        &self,
        hw_id: u32,
        stack_top: u64,
        entry: extern "C" fn(u64) -> !,
        arg: u64,
        started: impl Fn() -> bool,
    ) -> bool {
        // Stejne tabulky a TCR jako bootovaci CPU, ale se zapnutou spodni
        // polovinou (identita stubu); planovac ji pri prvnim prepnuti vypne.
        const TCR_EPD0: u64 = 1 << 7;
        let params = Params {
            mair: mrs!("mair_el1"),
            tcr: mrs!("tcr_el1") & !TCR_EPD0,
            ttbr: mm::kernel_root().as_u64(),
            stack_top,
            entry: entry as usize as u64,
            arg,
        };
        let virt = self.params.to_virt();
        // SAFETY: stranka parametru patri tomuto startu.
        unsafe { (virt.as_mut_ptr::<Params>()).write_volatile(params) };
        dcache_clean_invalidate(virt.as_u64(), core::mem::size_of::<Params>() as u64);

        let mpidr = hw_id_to_mpidr(hw_id);
        let ret = psci_call(
            self.hvc,
            PSCI_CPU_ON,
            mpidr,
            self.stub,
            self.params.as_u64(),
        );
        if ret != PSCI_SUCCESS && ret != PSCI_ALREADY_ON {
            log::error!("PSCI CPU_ON pro MPIDR {mpidr:#x} selhalo: {ret}");
            return false;
        }
        for _ in 0..2000 {
            if started() {
                return true;
            }
            timer::busy_wait_ns(500_000);
        }
        started()
    }

    fn finish(self) {
        mm::unmap_identity_page(PhysAddr::new(self.stub));
        mm::phys::free_pages(self.params, 0);
    }
}
