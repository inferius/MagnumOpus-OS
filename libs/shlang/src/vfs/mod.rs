//! Souborove vrstvy shellu.
//!
//! - [`path`]: normalizace cest (`~`, `.`, `..`, relativni cesty), porovnani
//!   bez ohledu na velikost pismen (FS je case-insensitive).
//! - [`mem`]: FS v pameti (testy na hostiteli).
//! - [`overlay`]: zmeny prikazu nanecisto ("ukaz, co se stane", zamek
//!   nebezpecnych operaci) a jejich diff.
//! - [`undo`]: liny snapshot pri prvni zmene a vraceni prikazu.

pub mod mem;
pub mod overlay;
pub mod path;
pub mod undo;
