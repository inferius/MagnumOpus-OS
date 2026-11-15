//! Obsluha vyjimek ve vlakne pro signaly knihovny C (`cstd`:
//! `SIGSEGV`, `SIGBUS`, `SIGFPE`, `SIGILL`, `SIGTRAP`): obalka
//! `libos::exception` (cstd nema libos primo).

pub use abi::exception::{Report, access, kind};
pub use libos::exception::{
    Frame, Handler, SignalClass, alloc_alt_stack, clear_handler, min_alt_stack, set_alt_stack, set_handler, signal_class,
    signal_class_of,
};
