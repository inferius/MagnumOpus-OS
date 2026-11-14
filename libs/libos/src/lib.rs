//! libos - knihovna pro programy v userspace.
//!
//! Stabilni rozhrani pro aplikace: syscally jsou interni detail, programy
//! pouzivaji typy a funkce z teto knihovny. Obsahuje:
//!
//! - typy objektu s automatickym zavrenim handlu ([`Handle`], [`Event`],
//!   [`Channel`], [`Vmo`], [`Process`], [`Thread`]),
//! - cekani ([`wait_one`], [`wait_many`], [`Port`]), cas ([`time`]),
//!   nahodna cisla ([`random`]),
//! - log do jadra ([`println!`]),
//! - start procesu: vstupni bod, heap, panic handler, startovni zprava
//!   ([`Startup`], makro [`entry!`]),
//! - spousteni programu z initfs ([`spawn`]),
//! - bariery pameti pro ovladace ([`barrier`]),
//! - velikost stranky procesu ([`vm::page_size`]).
//!
//! Pouziti v programu:
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//! libos::entry!(main);
//! fn main(startup: libos::Startup) -> i64 { 0 }
//! ```

#![no_std]

extern crate alloc;

pub mod barrier;
pub mod dma;
mod channel;
mod event;
pub mod exception;
pub mod fence;
mod handle;
pub mod hw;
pub mod initfs;
pub mod job;
#[cfg(feature = "libd")]
pub mod libd;
pub mod loader;
pub mod log;
mod mapping;
mod port;
mod process;
pub mod random;
pub mod params;
pub mod rt;
#[cfg(feature = "rt")]
mod rt_own;
pub mod sched;
pub mod spawn;
pub mod sync;
pub mod sys;
pub mod thread;
pub mod time;
pub mod vm;
mod vmo;
mod wait;

/// Rozhrani jadra (kody chyb, konstanty mapovani, kody procesu, ...).
pub use abi;
pub use abi::{Error, Result, Rights, Signals};
pub use channel::{Channel, Message};
pub use event::Event;
pub use dma::DmaMap;
pub use fence::Fence;
pub use handle::Handle;
pub use mapping::{Mapping, Pod};
pub use port::{Port, PortPacket};
pub use process::{Process, SuspendToken, Thread};
pub use rt::Startup;
pub use job::Job;
pub use spawn::{spawn, spawn_in};
pub use vmo::Vmo;
pub use wait::{wait_many, wait_one};

/// Definuje vstupni funkci programu: `fn main(startup: Startup) -> i64`.
/// Navratova hodnota je navratovy kod procesu (0 = uspech). Jen pro
/// programy `no_std` (na cili magnumopus je vstupni bod v `std`).
#[cfg(feature = "rt")]
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        #[unsafe(no_mangle)]
        fn __libos_main(startup: $crate::Startup) -> i64 {
            $main(startup)
        }
    };
}

/// Ukonci proces s navratovym kodem.
pub fn exit(code: i64) -> ! {
    sys::process_exit(code)
}
