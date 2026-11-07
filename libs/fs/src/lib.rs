//! Souborovy system (format v2).
//!
//! - **Copy-on-write:** nic zive se neprepisuje na miste. Commit zapise nove
//!   uzly od listu ke koreni, metadata volneho mista a nakonec superblok do
//!   dalsiho slotu kruhu. Vypadek proudu kdykoli = po pripojeni stav po
//!   poslednim dokoncenem commitu (plus zmeny potvrzene `fsync`).
//! - **Merkleuv strom:** kazdy ukazatel nese 128bitovy kontrolni soucet
//!   (XXH3-128 nebo BLAKE3) a generaci; odhali poskozeni i ztracene zapisy.
//!   Kdyz je nejnovejsi stav poskozeny, pripojeni se vrati az o 3 generace.
//! - **Jeden B+strom pro metadata** s klicem (objekt, druh, offset): zaznamy
//!   objektu se souhrny podstromu (okamzita velikost adresare), polozky
//!   adresaru, mala data primo ve strome, extenty, snapshoty, deadlisty.
//! - **Extenty a odlozena alokace:** data dostanou bloky az pri zapisu, souvisle.
//! - **Alokacni skupiny se space logy**, odlozene znovupouziti bloku, TRIM.
//! - **Intent log** pro rychly `fsync`.
//! - **Snapshoty** jen pro cteni a jejich mazani pres deadlisty.
//! - **Defragmentace** na pozadi a **kontrola konzistence** (fsck, scrub).
//! - **Jmena** se porovnavaji po NFC a prevodu na mala pismena
//!   (case-insensitive), ulozi se tak, jak je uzivatel zadal.
//!
//! Podrobny popis: `docs/cs/souborovy-system.md`.
//!
//! Knihovna je `no_std` a pracuje nad libovolnym [`BlockDevice`]: ve sluzbe
//! `fsd` nad ovladacem disku, v xtask nad souborem obrazu, v testech nad
//! pameti.

#![cfg_attr(not(any(test, feature = "std")), no_std)]

extern crate alloc;

mod btree;
mod device;
mod fs;
mod hash;
mod layout;
mod names;
mod node;
pub mod perm;
mod space;
mod store;

#[cfg(test)]
mod tests;

pub use abi::{Error, Result};
pub use device::{BlockDevice, Durability, MemDevice};
pub use fs::{Change, ChangeKind, CheckReport, DatasetInfo, Handle, Revision, access, share, DefragReport, DirEntry, Fs, FsInfo, Kind, MountOptions, Permissions, SetTimes, SnapshotInfo, Stat, ds_flags};
pub use perm::{Acl, EVERYONE, Grant, Principal, SYSTEM, acl_flags, grant, rights};
pub use hash::Algo;
pub use layout::{BLOCK_SIZE, INLINE_MAX, MAX_EXTENT, MAX_FILE_SIZE, REUSE_DELAY, SYMLINK_MAX, TimeFormat, Timestamp};
pub use names::{NAME_MAX, NAME_MAX_CHARS, fold_name, is_bidi_control, validate_name};
pub use store::FormatOptions;

#[cfg(feature = "std")]
pub use device::FileDevice;
