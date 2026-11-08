//! Zpracovani zvuku pro `audiod` a programy: bez zavislosti na jadru,
//! testuje se na hostiteli.
//!
//! - [`convert`]: vzorky libovolneho formatu -> vnitrni stereo Q23,
//! - [`src`]: prevod vzorkovaci frekvence (kubicka interpolace Hermite),
//! - [`mix`]: scitani streamu se zesilenim a orezanim do 16 bitu,
//! - [`tone`]: sinusovy ton v celych cislech (testy, `audio tone`),
//! - [`wav`]: cteni a zapis souboru WAV (RIFF).
//!
//! Vse je v celych cislech: na aarch64 bezi programy bez FPU (softfloat)
//! a mix kazde periody musi stihnout desetiny milisekundy.

#![no_std]

extern crate alloc;

#[cfg(test)]
mod tests;

pub mod convert;
pub mod mix;
pub mod src;
pub mod tone;
pub mod wav;

/// Vnitrni vzorek: 24 bitu se znamenkem v i32 (rozsah +/- 2^23).
pub type Q23 = i32;
pub const Q23_MAX: i32 = (1 << 23) - 1;
pub const Q23_MIN: i32 = -(1 << 23);

/// Stereo snimek (L, R).
pub type Frame = [Q23; 2];
