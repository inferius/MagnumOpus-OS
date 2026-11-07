//! Skladani kompozitoru na GPU pres Venus.
//!
//! Snimek a zdrojove povrchy jsou buffery pixelu (`u32` 0xAARRGGBB,
//! premultiplied) v pameti viditelne z hosta: proces je ma namapovane
//! (blob z `VkDeviceMemory`), takze zapis zdroje i cteni hotoveho snimku
//! jsou primy pristup do pameti. Kazda operace [`plan::Draw`] je jeden
//! dispatch vypocetniho shaderu `spirv::blend` (stejne vzorce jako
//! `gfx::Canvas::draw_image`), mezi operacemi bariera na snimku.
//!
//! - [`plan`] - prevod operace na parametry shaderu (testy na hostiteli),
//! - `gpu` (vlastnost `os`) - zarizeni, pipeline, povrchy, skladani.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod plan;

#[cfg(feature = "os")]
pub mod gpu;

#[cfg(test)]
mod tests;
