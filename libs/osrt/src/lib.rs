//! osrt - behove prostredi `std` pro MagnumOpus.
//!
//! `std` pro cil `*-unknown-magnumopus` (fork `library/std` v
//! `third_party/rust-std`) obsahuje jen tenke lepidlo; vsechna logika je
//! tady, nad libos a protokoly sluzeb (vzor: `moto-rt` pro Motor OS,
//! `hermit-abi` pro Hermit). Rozhrani pouziva jen typy `core`/`alloc`
//! a `abi::Error`; prevod na `io::Error` a dalsi typy std dela lepidlo.
//!
//! Moduly:
//! - [`start`] - vstupni bod procesu, startovni zprava, standardni handly,
//! - [`heap`] - alokator (dlmalloc nad objekty pameti),
//! - [`tls`] - bloky TLS vlaken (nativni ELF TLS, local-exec),
//! - [`thread`] - vlakna, [`futex`] - cekani pro `std::sync`,
//! - [`stdio`], [`env`], [`time`], [`fs`].
//!
//! Navrh: `docs/cs/rust-std.md`.

#![no_std]

extern crate alloc;

pub mod env;
pub mod fs;
pub mod futex;
pub mod heap;
pub mod net;
pub mod pipe;
pub mod process;
pub mod start;
pub mod stdio;
pub mod thread;
pub mod time;
pub mod tls;

pub use abi::Error;

/// Vysledek operaci runtime.
pub type Result<T> = core::result::Result<T, Error>;

/// Ukonci proces s navratovym kodem.
pub fn exit(code: i32) -> ! {
    libos::sys::process_exit(i64::from(code))
}

/// Okamzite ukonceni po fatalni chybe (`std::process::abort`).
pub fn abort() -> ! {
    libos::sys::process_exit(abi::process::EXIT_PANIC)
}

/// Text do logu jadra (posledni zachrana, kdyz neni stderr).
pub fn debug_log(text: &str) {
    libos::sys::debug_log(text);
}

/// Nahodne bajty z generatoru jadra (po kusech `RANDOM_MAX`).
pub fn fill_random(buf: &mut [u8]) {
    for chunk in buf.chunks_mut(abi::syscall::RANDOM_MAX) {
        // Slaby vystup pred nasazenim generatoru je pro HashMap v poradku;
        // kryptografie si vynucuje REQUIRE_SEEDED sama (libos::random).
        let _ = libos::sys::random_get(chunk, 0);
    }
}
