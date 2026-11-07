//! Chytry terminal (milnik 6b).
//!
//! - [`proto`]: nativni protokol mezi terminalem (oknem) a sluzbou relaci:
//!   typovane hodnoty, tabulky se schematem, odkazy, prubeh, vyzvy, bloky
//!   vystupu. Zadne escape sekvence.
//! - [`session`]: sluzba relaci. Relace a jeji bezici prikazy patri sluzbe,
//!   ne oknu: po zavreni okna (odpojeni) bezi dal a jde se k nim vratit.
//! - [`vt`]: adapter na text s escape sekvencemi VT100 (seriova linka,
//!   prevzate programy).
//! - [`screen`]: textova obrazovka a kresleni vlastnim pismem do
//!   framebufferu (zachranna konzole a zaklad pro okno v 6c).
//! - [`perm`]: prevod opravneni programu (manifest) na pripojene cesty
//!   relace fsd (`CONNECT_AS`/`GRANT`).

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod perm;
pub mod proto;
pub mod screen;
pub mod session;
pub mod vt;

#[cfg(test)]
mod tests;
