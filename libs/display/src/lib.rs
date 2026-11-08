//! Spolecna cast nativnich ovladacu displeje (nastaveni obrazu bez
//! akcelerace), nezavisla na vyrobci a na jadre OS. Testuje se na
//! hostiteli (`cargo test -p display`), vcetne realnych dumpu EDID.
//!
//! - [`timing`]: casovani obrazu (pixel clock, porchy, synchronizace).
//! - [`cvt`]: vypocet casovani podle VESA CVT 1.2 (standardni, RB v1, RB v2).
//! - [`dmt`]: tabulka VESA DMT (presne hodnoty beznych rezimu).
//! - [`edid`]: zakladni blok EDID 1.3/1.4 (deskriptory, standardni casovani).
//! - [`cta`]: rozsireni CTA-861 (VIC, HDMI VSDB, HF-VSDB, audio).
//! - [`displayid`]: DisplayID 1.3 a 2.0 (casovani typu I a VII).
//! - [`modes`]: seznam rezimu monitoru ze vsech zdroju.
//! - [`dp`]: DisplayPort (AUX, DPCD, navazani linky, MST - priprava).
//! - [`hdmi`]: HDMI (infoframy, TMDS, SCDC).
//!
//! Puvod: datove formaty podle verejnych specifikaci VESA a CTA; postupy
//! (navazani linky DP, kodovani AUX a sideband zprav MST) podle
//! linuxovych `drm_dp_helper.c`, `drm_dp_mst_topology.c`, `drm_edid.c`,
//! `drm_displayid.c`, `drm_modes.c` a `intel_dp_link_training.c`, ktere
//! jsou pod licenci MIT (overeno v hlavickach, viz THIRD-PARTY.md).
//! Kod je napsany znovu v Rustu, ne mechanicky preklad.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod cta;
pub mod cvt;
pub mod displayid;
pub mod dmt;
pub mod dp;
pub mod edid;
pub mod hdmi;
pub mod modes;
pub mod timing;

#[cfg(test)]
mod tests;

/// Chyba pri cteni dat displeje nebo pri komunikaci s nim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Data jsou kratsi, nez rika jejich hlavicka.
    Truncated,
    /// Spatna hlavicka nebo znacka bloku.
    BadHeader,
    /// Nesouhlasi kontrolni soucet.
    BadChecksum,
    /// Hodnota mimo povoleny rozsah.
    Invalid,
    /// Prijemce (monitor) transakci odmitl (NACK).
    Nack,
    /// Prijemce opakovane zadal o odklad (DEFER) nebo neodpovedel.
    Timeout,
    /// Chyba prenosu na strane zdroje (ovladac, hardware).
    Io,
    /// Navazani linky selhalo (obnova hodin).
    ClockRecovery,
    /// Navazani linky selhalo (vyrovnani kanalu).
    ChannelEq,
    /// Linka nema dost sirky pasma pro rezim.
    Bandwidth,
}

pub type Result<T> = core::result::Result<T, Error>;
