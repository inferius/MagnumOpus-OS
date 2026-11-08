//! ID zarizeni PCI, ktera ovladace obsluhuji (zdroj: tabulky ovladacu
//! Linuxu e1000e/hw.h, igc/igc_hw.h, r8169_main.c - jen ID jako fakta,
//! zadny kod). Podle nich spravce zarizeni vybere program.

/// Vyrobce Intel.
pub const INTEL: u16 = 0x8086;
/// Vyrobce Realtek.
pub const REALTEK: u16 = 0x10EC;

/// Program ovladace pro jednu rodinu karet.
pub struct Family {
    /// Program v initfs.
    pub program: &'static str,
    pub vendor: u16,
    pub devices: &'static [u16],
}

/// Generace karty e1000e (lisi se reset a probuzeni PHY).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum E1000eKind {
    /// Samostatny radic 82574/82583 (NVM v EEPROM, MSI-X).
    Discrete,
    /// Radic v cipove sade PCH od Lynx Point (I217, I218, I219): PHY na
    /// SMBus/PCIe, NVM ve flash BIOSu, jen MSI.
    Pch,
}

/// 82574L, 82574LA (i virtualni karta "e1000e" VMware a QEMU), 82583V.
pub const E1000E_DISCRETE: &[u16] = &[0x10D3, 0x10F6, 0x150C];

/// I217, I218, I219 (PCH LPT az NVL).
pub const E1000E_PCH: &[u16] = &[
    // LPT: I217-LM, I217-V; LPT-LP: I218-LM, I218-V; I218 LM2/V2/LM3/V3
    0x153A, 0x153B, 0x155A, 0x1559, 0x15A0, 0x15A1, 0x15A2, 0x15A3,
    // SPT/LBG: I219 LM, V, LM2, V2, LM3, LM4, V4, LM5, V5
    0x156F, 0x1570, 0x15B7, 0x15B8, 0x15B9, 0x15D7, 0x15D8, 0x15E3, 0x15D6,
    // CNP: LM6, V6, LM7, V7
    0x15BD, 0x15BE, 0x15BB, 0x15BC,
    // ICP: LM8, V8, LM9, V9
    0x15DF, 0x15E0, 0x15E1, 0x15E2,
    // CMP: LM10, V10, LM11, V11, LM12, V12
    0x0D4E, 0x0D4F, 0x0D4C, 0x0D4D, 0x0D53, 0x0D55,
    // TGP: LM13, V13, LM14, V14, LM15, V15
    0x15FB, 0x15FC, 0x15F9, 0x15FA, 0x15F4, 0x15F5,
    // RPL: LM23, V23, LM22, V22
    0x0DC5, 0x0DC6, 0x0DC7, 0x0DC8,
    // ADP: LM16, V16, LM17, V17, LM19, V19
    0x1A1E, 0x1A1F, 0x1A1C, 0x1A1D, 0x550C, 0x550D,
    // MTP: LM18, V18; LNP: LM20, V20, LM21, V21
    0x550A, 0x550B, 0x550E, 0x550F, 0x5510, 0x5511,
    // ARL: LM24, V24; PTP: LM25, V25, LM27, V27; NVL: LM29, V29
    0x57A0, 0x57A1, 0x57B3, 0x57B4, 0x57B7, 0x57B8, 0x57B9, 0x57BA,
];

/// I225 (LM, V, I, K, K2, LMVP, IT, bez NVM), I226 (LM, V, IT, K, LMVP,
/// bez NVM), I220-V, I221-V.
pub const IGC: &[u16] = &[
    0x15F2, 0x15F3, 0x15F8, 0x15F7, 0x3100, 0x3101, 0x3102, 0x5502, 0x5503, 0x0D9F, 0x125B,
    0x125C, 0x125D, 0x125E, 0x125F, 0x15FD,
];

/// RTL8111/8168 (vsechny revize cipu maji stejne ID 0x8168; 0x8161
/// a 0x8162 jsou varianty pro OEM). RTL8169 (PCI), RTL8101/8136 (jen
/// 100 Mb/s) a RTL8125/8126 (2,5/5 Gb/s, jine registry) sem nepatri.
pub const R8169: &[u16] = &[0x8168, 0x8161, 0x8162];

pub const FAMILIES: &[Family] = &[
    Family {
        program: "e1000e",
        vendor: INTEL,
        devices: E1000E_DISCRETE,
    },
    Family {
        program: "e1000e",
        vendor: INTEL,
        devices: E1000E_PCH,
    },
    Family {
        program: "igc",
        vendor: INTEL,
        devices: IGC,
    },
    Family {
        program: "r8169",
        vendor: REALTEK,
        devices: R8169,
    },
];

/// Program ovladace pro zarizeni, nebo `None`.
pub fn program_for(vendor: u16, device: u16) -> Option<&'static str> {
    FAMILIES
        .iter()
        .find(|f| f.vendor == vendor && f.devices.contains(&device))
        .map(|f| f.program)
}

/// Generace karty e1000e podle ID.
pub fn e1000e_kind(device: u16) -> Option<E1000eKind> {
    if E1000E_DISCRETE.contains(&device) {
        Some(E1000eKind::Discrete)
    } else if E1000E_PCH.contains(&device) {
        Some(E1000eKind::Pch)
    } else {
        None
    }
}
