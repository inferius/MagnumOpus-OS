//! cstd - knihovna C a vrstva POSIX pro aplikace se std (cil magnumopus).
//!
//! Stoji nad `std` (a rozsirenimi `std::os::magnumopus`), takze proces ma
//! vzdy jeden runtime: alokator, TLS, vlakna i stdio sdili s kodem v Rustu
//! (navrh: docs/cs/cstd.md). Spolecne casti bez systemovych sluzeb
//! (retezce, ctype, `malloc`, `snprintf`, ochrana zasobniku) jsou v
//! `cstd-core`, ktery pouzivaji i sluzby no_std.
//!
//! Cast 1: `errno.h`, `stdio.h` (FILE, soubory, stdin/stdout/stderr),
//! `stdlib.h` (`exit`, `atexit`, `getenv`, `abort`), `assert`.
//! Cast 2a: cisla fd (`sys`, tabulka v osrt), `unistd.h`, `fcntl.h`,
//! `sys/stat.h`, `dirent.h`; `FILE` stoji nad cisly fd.
//!
//! Program v C: crate `#![no_main]` s `extern crate cstd;`, ktery v
//! `build.rs` prelozi zdroje C (`cbuild`); adresare hlavicek dostane od
//! cargo v promenne `DEP_CSTD_INCLUDE` (oddelene `;`, viz build.rs).
//! osrt pri startu zavola C `main(argc, argv)`.
//!
//! Symboly C se exportuji jen na cili magnumopus: na hostiteli
//! by se tloukly se systemovou libc (testy logiky bezi na hostiteli).

#![allow(non_camel_case_types, non_upper_case_globals)]
#![cfg_attr(target_os = "magnumopus", feature(magnumopus_runtime, rustc_private))]

// Symboly cstd-core (strlen, malloc, snprintf, ...) se musi prilinkovat.
extern crate cstd_core;

pub mod dirent;
pub mod errno;
pub mod flock;
pub mod pthread;
pub mod stat;
pub mod stdio;
pub mod stdlib;
pub mod sys;
pub mod unistd;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod fd_tests;
#[cfg(test)]
mod pthread_tests;

pub use cstd_core::ctypes::{c_char, c_int, c_long, c_uint, c_ulong, size_t};

/// Retezec C (`*const c_char` zakonceny nulou) jako bajty.
///
/// # Safety
///
/// `s` je platny retezec zakonceny nulou.
pub(crate) unsafe fn c_bytes<'a>(s: *const c_char) -> &'a [u8] {
    // SAFETY: predano volajicim.
    unsafe { core::slice::from_raw_parts(s, cstd_core::string::strlen(s)) }
}
