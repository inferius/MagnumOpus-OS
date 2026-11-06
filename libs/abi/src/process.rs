//! Navratove kody procesu.
//!
//! Proces konci kodem, ktery predal `PROCESS_EXIT` (0 = uspech, cokoli jineho
//! = chyba podle konvence programu). Kdyz proces ukonci jadro, pouzije kod
//! z rozsahu nize, aby se dal odlisit od kodu programu.

/// Proces byl ukoncen pres `PROCESS_KILL`.
pub const EXIT_KILLED: i64 = -1_000_000;
/// Proces zpusobil vyjimku CPU (napr. page fault); kod je
/// `EXIT_FAULT_BASE - vektor` (page fault = -1_000_114).
pub const EXIT_FAULT_BASE: i64 = -1_000_100;
/// Proces zpanikoval (libos panic handler).
pub const EXIT_PANIC: i64 = -1_000_001;
/// Posledni vlakno skoncilo bez `PROCESS_EXIT`.
pub const EXIT_NO_THREADS: i64 = -1_000_002;

/// Popis navratoveho kodu pro logy.
pub fn describe_exit(code: i64) -> &'static str {
    match code {
        0 => "uspech",
        EXIT_KILLED => "ukoncen jinym procesem (PROCESS_KILL)",
        EXIT_PANIC => "panika v programu",
        EXIT_NO_THREADS => "posledni vlakno skoncilo bez PROCESS_EXIT",
        c if (EXIT_FAULT_BASE - 255..=EXIT_FAULT_BASE).contains(&c) => "vyjimka CPU (pad programu)",
        _ => "kod programu",
    }
}
