//! Rozsirene API souboroveho systemu MagnumOpus pro aplikace ve Windows.
//!
//! Na pripojene jednotce (`osfs mount`, pozdeji nativni ovladac) jde bezne
//! pracovat pres Win32. Tohle API dodava navic to, co Win32 neumi: okamzite
//! velikosti adresaru, identitu objektu (oid + generace), jmena skryta
//! ve Windows, cile symbolickych odkazu, snapshoty a stav svazku.
//!
//! Kanal: `DeviceIoControl` na handlu otevreneho souboru nebo adresare
//! jednotky (obvykle korene `Q:\`), kody [`IOCTL_READ`] a [`IOCTL_WRITE`].
//! Stejne kody bude obsluhovat i budouci nativni ovladac, aplikace tedy
//! nepozna, ktery ovladac bezi.
//!
//! Zprava (obe smery): `MAGIC` u32, `VERSION` u16, `op` u16, u odpovedi
//! jeste `status` i32 (0 = v poradku, jinak kod z [`error`]), pak telo.
//! Cisla little-endian, retezec = delka u16 + UTF-8. Kazda zprava ma nejvys
//! [`MAX_MESSAGE`] bajtu (WinFsp propusti `DeviceIoControl` jen do 4 KiB
//! v kazdem smeru); dlouhe vypisy jdou po strankach (`cursor`), cteni po
//! [`MAX_READ`] bajtech. Server sve skutecne maximum hlasi v
//! `HelloInfo::max_message` (nativni ovladac muze mit vic).
//!
//! Crate je `no_std` + `alloc` (server, ovladac jadra); feature `client`
//! pridava klienta pro Windows ([`client::Client`]).
//! Popis: `docs/cs/osfs-api.md`.

#![cfg_attr(not(any(test, feature = "client")), no_std)]

extern crate alloc;

pub mod codec;
mod msg;

#[cfg(all(feature = "client", windows))]
pub mod client;

pub use msg::*;

/// "MOFS" (little-endian).
pub const MAGIC: u32 = 0x5346_4F4D;
/// Verze protokolu. Nova verze muze pridat operace a polozky na konec tela;
/// stary klient pozna novejsi server z `Hello` a pouziva jen to, co zna.
/// Verze 2: masky dostupnosti v `EntryInfo` (zmena tvaru polozky).
pub const VERSION: u16 = 2;

/// Jmeno souboroveho systemu, ktere jednotka hlasi Windows
/// (`GetVolumeInformationW`) - podle nej aplikace pozna nas svazek.
pub const FS_NAME: &str = "MagnumOpusFS";

/// Nejvetsi zprava (pozadavek i odpoved) v bajtech: limit WinFsp pro
/// `DeviceIoControl` (`FSP_FSCTL_DEVICECONTROL_SIZEMAX`).
pub const MAX_MESSAGE: usize = 4096;
/// Nejvic bajtu dat v jedne odpovedi na `Read` (zprava minus hlavicka
/// a delka dat).
pub const MAX_READ: u32 = (MAX_MESSAGE - RESPONSE_HEADER - 4) as u32;
/// Nejdelsi cesta v pozadavku (bajty UTF-8); delsi cesta se do zpravy
/// nevejde.
pub const MAX_PATH: usize = MAX_MESSAGE - 64;
/// Nejdelsi kurzor vypisu (bajty). Kurzor je slozene jmeno posledni
/// polozky; u delsich jmen se pouzije poradi ve snimku vypisu.
pub const MAX_CURSOR: usize = 1024;

const fn ctl_code(device: u32, function: u32, method: u32, access: u32) -> u32 {
    (device << 16) | (access << 14) | (function << 2) | method
}

/// Typ zarizeni v kodu: vlastni kody musi mit bit 0x8000 (WinFsp i Windows).
pub const DEVICE_TYPE: u32 = 0xA0F5;
const METHOD_BUFFERED: u32 = 0;
const FILE_READ_ACCESS: u32 = 1;
const FILE_WRITE_ACCESS: u32 = 2;

/// Kod pro operace jen pro cteni. Handle staci otevrit pro cteni.
pub const IOCTL_READ: u32 = ctl_code(DEVICE_TYPE, 0x801, METHOD_BUFFERED, FILE_READ_ACCESS);
/// Kod pro operace, ktere meni svazek ([`Request::is_write`]). Windows ho
/// pusti jen na handle otevreny i pro zapis; server navic odmitne zapisovou
/// operaci poslanou kodem pro cteni.
pub const IOCTL_WRITE: u32 = ctl_code(DEVICE_TYPE, 0x802, METHOD_BUFFERED, FILE_WRITE_ACCESS);

/// Co server umi (bity v `HelloInfo::features`).
pub mod feature {
    /// Snapshoty: seznam, vytvoreni, cteni (`at`).
    pub const SNAPSHOTS: u64 = 1 << 0;
    /// Prejmenovani podle jmen v nasem FS (i skrytych ve Windows).
    pub const RENAME: u64 = 1 << 1;
    /// Cteni souboru podle jmena v nasem FS (i skrytych ve Windows).
    pub const READ: u64 = 1 << 2;
    /// Datasety: `Kind::Dataset`, `EntryInfo::dataset`, snapshoty po
    /// datasetech.
    pub const DATASETS: u64 = 1 << 3;
}

/// Dostupnost hodnot v `EntryInfo` (masky `known` a `withheld`), bity jako
/// `fsproto::avail` v MagnumOpus OS. Bit v `known` = hodnota plati;
/// v `withheld` = existuje, ale volajici ji nesmi videt (pole je 0); ani
/// jeden = nedostupna (FS ji nevede nebo k druhu nepatri, napr. `TREE`
/// u souboru). Oba najednou = poskozena zprava.
pub mod avail {
    /// Velikost (u adresare `tree_bytes`).
    pub const SIZE: u16 = 1 << 0;
    pub const MTIME: u16 = 1 << 1;
    pub const CTIME: u16 = 1 << 2;
    pub const BTIME: u16 = 1 << 3;
    /// Vlastnik (`EntryInfo` ho zatim nenese, bit se nepouziva).
    pub const OWNER: u16 = 1 << 4;
    /// Pocty v podstromu (`tree_files`, `tree_dirs`).
    pub const TREE: u16 = 1 << 5;
    /// Prava (`EntryInfo` je zatim nenese, bit se nepouziva).
    pub const RIGHTS: u16 = 1 << 6;
    pub const ALL: u16 = SIZE | MTIME | CTIME | BTIME | OWNER | TREE | RIGHTS;
}

/// Kody chyb v odpovedi. Hodnoty jsou stejne jako `abi::Error` v
/// MagnumOpus OS (hlida test v `tools/osfs`).
pub mod error {
    pub const OK: i32 = 0;
    pub const ACCESS_DENIED: i32 = -2;
    pub const WRONG_TYPE: i32 = -3;
    pub const NO_MEMORY: i32 = -5;
    pub const INVALID_ARGS: i32 = -7;
    pub const NOT_FOUND: i32 = -8;
    pub const NOT_SUPPORTED: i32 = -9;
    pub const CORRUPTED: i32 = -10;
    pub const BUFFER_TOO_SMALL: i32 = -14;
    pub const OUT_OF_RANGE: i32 = -15;
    pub const ALREADY_EXISTS: i32 = -19;
    pub const NOT_A_DIRECTORY: i32 = -20;
    pub const IS_A_DIRECTORY: i32 = -21;
    pub const NOT_EMPTY: i32 = -22;
    pub const NO_SPACE: i32 = -23;
    pub const IO_ERROR: i32 = -24;
    pub const READ_ONLY: i32 = -25;
    /// Sdileci rezim nebo zamek jineho handlu.
    pub const LOCKED: i32 = -26;
    /// Podminena zmena: objekt se mezitim zmenil.
    pub const CONFLICT: i32 = -27;

    /// Strojove jmeno kodu (pro hlasky a logy).
    pub fn name(code: i32) -> &'static str {
        match code {
            OK => "OK",
            ACCESS_DENIED => "ACCESS_DENIED",
            WRONG_TYPE => "WRONG_TYPE",
            NO_MEMORY => "NO_MEMORY",
            INVALID_ARGS => "INVALID_ARGS",
            NOT_FOUND => "NOT_FOUND",
            NOT_SUPPORTED => "NOT_SUPPORTED",
            CORRUPTED => "CORRUPTED",
            BUFFER_TOO_SMALL => "BUFFER_TOO_SMALL",
            OUT_OF_RANGE => "OUT_OF_RANGE",
            ALREADY_EXISTS => "ALREADY_EXISTS",
            NOT_A_DIRECTORY => "NOT_A_DIRECTORY",
            IS_A_DIRECTORY => "IS_A_DIRECTORY",
            NOT_EMPTY => "NOT_EMPTY",
            NO_SPACE => "NO_SPACE",
            IO_ERROR => "IO_ERROR",
            READ_ONLY => "READ_ONLY",
            LOCKED => "LOCKED",
            CONFLICT => "CONFLICT",
            _ => "ERROR",
        }
    }
}
