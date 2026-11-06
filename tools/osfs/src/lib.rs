//! `osfs` - hostitelsky nastroj pro souborovy system MagnumOpus.
//!
//! - `image`: vytvoreni a otevreni souboru s obrazem, pomocne funkce CLI,
//! - `lock`: zamek obrazu (nejvys jeden zapisovatel),
//! - `names`: preklad jmen mezi nasim FS a Windows,
//! - `mount` (jen Windows): pripojeni obrazu jako jednotky pres WinFsp.
//!
//! Pouziva jen verejne API knihovny `fs` (`fs::Fs`); knihovnu nemeni.
//! Popis: `docs/cs/osfs.md`.

pub mod image;
pub mod lock;
pub mod names;

#[cfg(windows)]
pub mod mount;
