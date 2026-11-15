//! `libc.so` pro programy PIE v C (docs/cs/cstd.md, kapitola libc.so):
//! sdilena knihovna s knihovnou C `cstd`, `std` a behovym prostredim
//! `osrt` prelozenymi jako PIC. Program ji ma v `DT_NEEDED`, zavadec
//! `ld.so` ji nacte a `_start` programu (crt1.o ze sysrootu C) zavola
//! `__libc_start` z osrt. Kod je cely v `cstd` a `osrt`.

#![no_std]

extern crate cstd;
extern crate mosys;
extern crate mowin;

unsafe extern "C" {
    /// Z `libs/cstd/c/math_ld.c` (funkce `long double`): odkaz vynuti
    /// prilinkovani celeho objektu, jinak by ho linker z archivu nevzal
    /// a verzovaci skript (`export-c.txt`) by nemel co exportovat.
    fn strtold();
}

#[used]
static KEEP_MATH_LD: unsafe extern "C" fn() = strtold;
