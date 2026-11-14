//! mosdl - rozhrani C pro port SDL na MagnumOpus (docs/cs/sdl.md).
//!
//! Soucast `libc.a` (`libs/cstd-libc`): dve staticke knihovny v Rustu
//! v jednom programu by mely kazda svou kopii std. Program bez SDL nic
//! nezaplati (`--gc-sections` zahodi nepouzite).
//!
//! - `event.rs` - preklad udalosti okna a kopie snimku (ciste, testy na
//!   hostiteli),
//! - `os.rs` - symboly `mosdl_*` nad oknem z `libs/ui` (jen cil magnumopus).
//!
//! Hlavicka pro C: `include/mosdl.h` (v sysrootu C `include/mosdl.h`).

#![cfg_attr(target_os = "magnumopus", feature(rustc_private))]

pub mod event;
#[cfg(target_os = "magnumopus")]
mod os;

#[cfg(test)]
mod tests;
