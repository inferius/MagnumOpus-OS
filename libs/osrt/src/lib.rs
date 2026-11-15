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
//! - [`stdio`], [`env`], [`time`], [`fs`], [`fd`] (cisla fd pro cstd),
//!   [`inherit`] (dedeni fd potomkem `posix_spawn`).
//!
//! Navrh: `docs/cs/rust-std.md`.

#![no_std]

extern crate alloc;

pub mod dl;
pub mod env;
pub mod exception;
pub mod fd;
pub mod fmap;
pub mod fs;
pub mod futex;
pub mod gui;
pub mod heap;
pub mod inherit;
pub mod memfd;
pub mod net;
pub mod pipe;
pub mod poll;
pub mod process;
pub mod sock;
pub mod start;
pub mod stdio;
pub mod thread;
pub mod tty;
pub mod time;
pub mod tls;
pub mod unix;
pub mod vm;

pub use abi::Error;

/// Vysledek operaci runtime.
pub type Result<T> = core::result::Result<T, Error>;

/// Ukonci proces s navratovym kodem.
pub fn exit(code: i32) -> ! {
    run_at_exit();
    // Pod zavadecem: finalizatory knihoven a programu (DT_FINI_ARRAY).
    #[cfg(target_env = "dyn")]
    libos::loader::run_fini();
    // Staticky program: .fini_array (symboly linker skriptu).
    #[cfg(not(target_env = "dyn"))]
    start::init_array::run_fini();
    libos::sys::process_exit(i64::from(code))
}

/// Funkce volane pri ukonceni procesu (`exit`, navrat z `main`): C
/// `atexit` a vyprazdneni bufferu stdio knihovny `cstd`. Volaji se v
/// obracenem poradi registrace, kazda jednou.
static AT_EXIT: libos::sync::Mutex<alloc::vec::Vec<extern "C" fn()>> = libos::sync::Mutex::new(alloc::vec::Vec::new());

/// Zaregistruje funkci pro ukonceni procesu; `false` = moc registraci.
pub fn at_exit(f: extern "C" fn()) -> bool {
    let mut v = AT_EXIT.lock();
    // C pozaduje aspon 32; vic nedovolime (ochrana pred smyckou).
    if v.len() >= 1024 {
        return false;
    }
    v.push(f);
    true
}

fn run_at_exit() {
    loop {
        // Zamek se pred volanim uvolni: funkce smi registrovat dalsi.
        let Some(f) = AT_EXIT.lock().pop() else { break };
        f();
    }
}

/// Ukonci proces bez funkci `at_exit` (C `_Exit`).
pub fn exit_now(code: i32) -> ! {
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

/// Nahodne bajty jen z nasazeneho generatoru (`getentropy`, klice);
/// `Err(ShouldWait)` = generator jeste nema dost entropie, nic se nezapsalo
/// do zbytku bufferu.
pub fn fill_random_seeded(buf: &mut [u8]) -> Result<()> {
    libos::random::fill_seeded(buf)
}
