//! Semihosting: volani hostitele (QEMU) instrukci `hlt #0xf000`. Jadro ho
//! pouziva jen k ukonceni QEMU s vysledkem testu (`cargo xtask test`);
//! QEMU musi byt spustene s `-semihosting-config enable=on`.
//!
//! Pozor: bez semihostingu je `hlt` neplatna instrukce a vyvola vyjimku,
//! proto se volani provadi jen v testovacim buildu (feature `qemu-test`).

use core::arch::asm;

/// SYS_EXIT (0x18) s parametrem ADP_Stopped_ApplicationExit a kodem.
const SYS_EXIT: u64 = 0x18;
const ADP_STOPPED_APPLICATION_EXIT: u64 = 0x20026;

/// Ukonci QEMU s navratovym kodem `code` (vrati se jen bez semihostingu).
pub fn exit(code: u64) {
    let block = [ADP_STOPPED_APPLICATION_EXIT, code];
    // SAFETY: volani semihostingu cte jen dvouslovny blok parametru.
    unsafe {
        asm!(
            "hlt #0xf000",
            in("x0") SYS_EXIT,
            in("x1") block.as_ptr(),
            options(nostack, readonly)
        )
    };
}
