//! mowin - rozhrani C pro vic oken kompozitoru v jednom procesu
//! (`include/mowin.h`; ovladac grafiky Wine `winemo.drv`,
//! docs/cs/windows-vrstva.md, kapitola W3).
//!
//! Soucast `libc.a` (`libs/cstd-libc`) jako `libs/mosdl`: dve staticke
//! knihovny v Rustu v jednom programu by mely kazda svou kopii std.
//! Program bez oken nic nezaplati (`--gc-sections` zahodi nepouzite).
//!
//! - `event.rs` - preklad udalosti okna a kopie snimku (ciste, testy na
//!   hostiteli); spolecny modul s `libs/mosdl`,
//! - `os/` - symboly `mowin_*` nad spojenim s kompozitorem (jen cil
//!   magnumopus); `os/audio.rs` zvuk `moaudio_*` (`include/moaudio.h`),
//!   spolecne jadro se `mosdl_audio_*`.

#![cfg_attr(target_os = "magnumopus", feature(rustc_private))]

pub mod event;
#[cfg(target_os = "magnumopus")]
mod os;

/// Zvuk aplikace (`moaudio_*`): spolecne jadro i pro `mosdl_audio_*`.
#[cfg(target_os = "magnumopus")]
pub use os::audio;

#[cfg(test)]
mod tests;
