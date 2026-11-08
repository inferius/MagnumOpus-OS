//! Intel High Definition Audio podle verejne specifikace (rev. 1.0a) -
//! cast bez hardwaru, testovatelna na hostiteli:
//!
//! - [`regs`]: registry radice (offsety a bity),
//! - [`verb`]: prikazy kodeku a cisla parametru,
//! - [`caps`]: dekodovani odpovedi (schopnosti widgetu, pinu, zesilovacu,
//!   vychozi konfigurace pinu, seznam spojeni),
//! - [`format`]: format streamu (registr SDnFMT a verb SET_CONVERTER_FORMAT),
//! - [`graph`]: graf widgetu jedne audio funkcni skupiny a hledani cest
//!   od pinu k prevodniku (DAC).
//!
//! Ovladac (`user/hda`) jen posila verby a cte registry; rozhodovani
//! (kterou cestu, ktery pin, jake zesileni) je zde.

#![no_std]

extern crate alloc;

#[cfg(test)]
mod tests;

pub mod caps;
pub mod format;
pub mod graph;
pub mod regs;
pub mod verb;
