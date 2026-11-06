//! `osfs` - hostitelsky nastroj pro souborovy system MagnumOpus.
//!
//! - `api`: serverova cast rozsireneho API (`osfs-proto`) nad `fs::Fs`,
//! - `image`: vytvoreni a otevreni souboru s obrazem, pomocne funkce CLI,
//! - `lock`: zamek obrazu (nejvys jeden zapisovatel),
//! - `versions`: datasety a verze (dataset k ceste, seznam, radky rozdilu),
//! - `names`: preklad jmen mezi nasim FS a Windows,
//! - `mount` (jen Windows): pripojeni obrazu jako jednotky pres WinFsp.
//!
//! Pouziva jen verejne API knihovny `fs` (`fs::Fs`); knihovnu nemeni.
//! Popis: `docs/cs/osfs.md`.

pub mod api;
pub mod image;
pub mod lock;
pub mod names;
pub mod versions;

#[cfg(windows)]
pub mod mount;
