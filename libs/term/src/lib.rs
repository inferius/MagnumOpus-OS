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
//! - [`broker`]: prostrednik termd - smerovani zprav mezi oknem
//!   a procesy relaci (kazda relace ve vlastnim procesu).
//! - [`ctl`]: ridici zpravy termd <-> relace a registr relaci (relace
//!   preziji konec termd).
//! - [`theme`]: barevny motiv (semanticke tridy -> barvy, svetly a tmavy,
//!   soubor `theme.toml`), [`render`]: vykresleni s motivem do ANSI.
//! - [`perm`]: prevod opravneni programu (manifest) na pripojene cesty
//!   relace fsd (`CONNECT_AS`/`GRANT`).

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod activity;
pub mod binary;
pub mod broker;
pub mod ctl;
pub mod perm;
pub mod proto;
pub mod render;
pub mod screen;
pub mod session;
pub mod taskboard;
pub mod tasks;
pub mod theme;
pub mod ui;
pub mod vt;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_broker;
#[cfg(test)]
mod tests_contrast;
#[cfg(test)]
mod tests_render;
#[cfg(test)]
mod tests_taskd;
#[cfg(test)]
mod tests_tasks;
#[cfg(test)]
mod tests_ui;
