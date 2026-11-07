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
mod channel;
mod event;
mod handle;
pub mod hw;
pub mod initfs;
pub mod log;
mod mapping;
mod port;
mod process;
pub mod random;
mod rt;
pub mod spawn;
pub mod sys;
pub mod time;
pub mod vm;
mod vmo;
mod wait;

/// Rozhrani jadra (kody chyb, konstanty mapovani, kody procesu, ...).
pub use abi;
pub use abi::{Error, Result, Rights, Signals};
pub use channel::{Channel, Message};
pub use event::Event;
pub use handle::Handle;
pub use mapping::{Mapping, Pod};
pub use port::{Port, PortPacket};
pub use process::{Process, Thread};
pub use rt::Startup;
pub use spawn::spawn;
pub use vmo::Vmo;
pub use wait::{wait_many, wait_one};

/// Definuje vstupni funkci programu: `fn main(startup: Startup) -> i64`.
/// Navratova hodnota je navratovy kod procesu (0 = uspech).
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
