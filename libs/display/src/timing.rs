//! Casovani obrazu: jeden radek = aktivni pixely, predni porch (front),
//! synchronizacni pulz (sync), zadni porch (back). Totez svisle v radcich.
//!
//! ```text
//!  |<- active ->|<- front ->|<- sync ->|<- back ->|
//!  |<------------------- total ------------------>|
//! ```
//!
//! U prokladaneho rezimu (`INTERLACED`) je `vactive` vyska celeho snimku,
//! svisle porchy a pulz plati pro jedno pole (pulsnimek).

/// Priznaky casovani.
pub mod flags {
    /// Kladna polarita vodorovne synchronizace.
    pub const HSYNC_POS: u32 = 1 << 0;
    /// Kladna polarita svisle synchronizace.
    pub const VSYNC_POS: u32 = 1 << 1;
    /// Prokladany rezim.
    pub const INTERLACED: u32 = 1 << 2;
    /// Preferovany (nativni) rezim monitoru.
    pub const PREFERRED: u32 = 1 << 3;
    /// Redukovane zatemneni (CVT-RB, typicke pro LCD).
    pub const REDUCED: u32 = 1 << 4;
}

/// Odkud casovani pochazi (pro razeni a ladeni).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    /// Detailni casovani (EDID DTD, DisplayID typ I/VII).
    Detailed,
    /// Kod VIC z CTA-861.
    Vic,
    /// Tabulka VESA DMT.
    Dmt,
    /// Vypocet CVT (standardni casovani bez zaznamu v DMT).
    Cvt,
    /// Nastaveni firmwaru (GOP), znamo jen castecne.
    Firmware,
}

/// Casovani jednoho rezimu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// Pixel clock v kHz.
    pub clock_khz: u32,
    pub hactive: u16,
    pub hfront: u16,
    pub hsync: u16,
    pub hback: u16,
    pub vactive: u16,
    pub vfront: u16,
    pub vsync: u16,
    pub vback: u16,
    /// [`flags`].
    pub flags: u32,
}

impl Timing {
    pub const fn htotal(&self) -> u32 {
        self.hactive as u32 + self.hfront as u32 + self.hsync as u32 + self.hback as u32
    }

    /// Celkovy pocet radku jednoho pole (u prokladaneho rezimu polovina
    /// aktivnich radku + zatemneni pole).
    pub const fn vtotal(&self) -> u32 {
        let active = if self.interlaced() { self.vactive as u32 / 2 } else { self.vactive as u32 };
        active + self.vfront as u32 + self.vsync as u32 + self.vback as u32
    }

    pub const fn interlaced(&self) -> bool {
        self.flags & flags::INTERLACED != 0
    }

    pub const fn preferred(&self) -> bool {
        self.flags & flags::PREFERRED != 0
    }

    /// Obnovovaci frekvence v mHz (tisicinach Hz), zaokrouhlena. U
    /// prokladaneho rezimu frekvence snimku (dve pole).
    pub fn refresh_mhz(&self) -> u32 {
        let total = self.htotal() as u64 * self.vtotal() as u64;
        if total == 0 {
            return 0;
        }
        // Pole za sekundu = clock / total; snimek = 2 pole u prokladaneho.
        let fields_mhz = (self.clock_khz as u64 * 1_000_000 + total / 2) / total;
        if self.interlaced() { (fields_mhz / 2) as u32 } else { fields_mhz as u32 }
    }

    /// Obnovovaci frekvence zaokrouhlena na cele Hz (59.94 -> 60).
    pub fn refresh_hz(&self) -> u32 {
        (self.refresh_mhz() + 500) / 1000
    }

    /// Vodorovna frekvence v Hz.
    pub fn hfreq_hz(&self) -> u32 {
        let t = self.htotal();
        if t == 0 { 0 } else { (self.clock_khz as u64 * 1000 / t as u64) as u32 }
    }

    /// Je casovani smysluplne (nenulove rozmery, porchy v rozumnych mezich)?
    pub fn is_valid(&self) -> bool {
        self.clock_khz > 0
            && self.hactive > 0
            && self.vactive > 0
            && self.hsync > 0
            && self.vsync > 0
            && self.htotal() <= u16::MAX as u32
            && self.vtotal() <= u16::MAX as u32
    }

    /// Stejny rezim (rozmery, prokladani a obnova s toleranci 0.5 Hz)?
    pub fn same_mode(&self, o: &Timing) -> bool {
        self.hactive == o.hactive
            && self.vactive == o.vactive
            && self.interlaced() == o.interlaced()
            && self.refresh_mhz().abs_diff(o.refresh_mhz()) <= 500
    }
}
