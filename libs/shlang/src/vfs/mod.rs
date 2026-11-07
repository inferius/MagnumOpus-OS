//! Souborove vrstvy shellu.
//!
//! - [`path`]: normalizace cest (`~`, `.`, `..`, relativni cesty), porovnani
//!   bez ohledu na velikost pismen (FS je case-insensitive).
//! - [`mem`]: FS v pameti (testy na hostiteli), s datasety, klony a pohledy
//!   ([`mem_hist`]).
//! - [`draft`]: koncept radku - zmeny nanecisto v klonech FS ("ukaz, co se
//!   stane", zamek nebezpecnych operaci, `preview`).
//! - [`diff`]: polozky diffu a jejich tabulka.
//! - [`undo`]: vraceni prikazu podle pohledu FS.

pub mod diff;
pub mod draft;
pub mod mem;
mod mem_hist;
pub mod path;
pub mod undo;
