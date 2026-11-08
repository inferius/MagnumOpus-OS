//! Registry radice HDA (BAR0, 16 KiB). Offsety podle specifikace,
//! kapitola 3.3.

/// Globalni schopnosti (u16): OSS 15:12, ISS 11:8, BSS 7:3, 64OK bit 0.
pub const GCAP: u64 = 0x00;
pub const VMIN: u64 = 0x02;
pub const VMAJ: u64 = 0x03;
/// Globalni rizeni (u32): bit 0 CRST (0 = reset), bit 8 UNSOL.
pub const GCTL: u64 = 0x08;
pub const GCTL_CRST: u32 = 1 << 0;
pub const GCTL_UNSOL: u32 = 1 << 8;
pub const WAKEEN: u64 = 0x0C;
/// Ktere kodeky se ozvaly po resetu (u16, bity 0..14; zapis 1 maze).
pub const STATESTS: u64 = 0x0E;
/// Rizeni preruseni (u32): bit 31 GIE, bit 30 CIE, bity 29:0 streamy.
pub const INTCTL: u64 = 0x20;
pub const INTCTL_GIE: u32 = 1 << 31;
pub const INTCTL_CIE: u32 = 1 << 30;
pub const INTSTS: u64 = 0x24;

pub const CORBLBASE: u64 = 0x40;
pub const CORBUBASE: u64 = 0x44;
pub const CORBWP: u64 = 0x48;
/// Cteci ukazatel CORB (u16); bit 15 = reset ukazatele.
pub const CORBRP: u64 = 0x4A;
pub const CORBRP_RST: u16 = 1 << 15;
/// Rizeni CORB (u8): bit 1 RUN, bit 0 MEIE.
pub const CORBCTL: u64 = 0x4C;
pub const CORBCTL_RUN: u8 = 1 << 1;
pub const CORBSTS: u64 = 0x4D;
/// Velikost CORB (u8): bity 1:0 vybrana (0 = 2, 1 = 16, 2 = 256 polozek),
/// bity 7:4 schopnosti (bit 4 = 2, 5 = 16, 6 = 256).
pub const CORBSIZE: u64 = 0x4E;

pub const RIRBLBASE: u64 = 0x50;
pub const RIRBUBASE: u64 = 0x54;
/// Zapisovy ukazatel RIRB (u16); bit 15 = reset.
pub const RIRBWP: u64 = 0x58;
pub const RIRBWP_RST: u16 = 1 << 15;
/// Po kolika odpovedich preruseni (u16).
pub const RINTCNT: u64 = 0x5A;
/// Rizeni RIRB (u8): bit 0 RINTCTL, bit 1 DMAEN, bit 2 ORUN preruseni.
pub const RIRBCTL: u64 = 0x5C;
pub const RIRBCTL_DMAEN: u8 = 1 << 1;
pub const RIRBSTS: u64 = 0x5D;
pub const RIRBSIZE: u64 = 0x5E;

/// Okamzite rozhrani prikazu (zalozni cesta bez CORB/RIRB).
pub const ICOI: u64 = 0x60;
pub const ICII: u64 = 0x64;
/// Stav (u16): bit 0 ICB (zaneprazdneno), bit 1 IRV (odpoved platna).
pub const ICIS: u64 = 0x68;
pub const ICIS_ICB: u16 = 1 << 0;
pub const ICIS_IRV: u16 = 1 << 1;

/// Buffer pozic DMA: dolni adresa, bit 0 = zapnuto.
pub const DPLBASE: u64 = 0x70;
pub const DPUBASE: u64 = 0x74;

/// Prvni deskriptor streamu; kazdy ma 0x20 bajtu. Poradi: vstupni
/// (ISS), vystupni (OSS), obousmerne (BSS).
pub const SD_BASE: u64 = 0x80;
pub const SD_SIZE: u64 = 0x20;

/// Offsety uvnitr deskriptoru streamu.
pub mod sd {
    /// Rizeni (3 bajty): bit 0 SRST, 1 RUN, 2 IOCE, 3 FEIE, 4 DEIE;
    /// bajt 2: bity 7:4 cislo streamu (tag), bit 3 DIR, bit 2 TP.
    pub const CTL: u64 = 0x00;
    pub const CTL_SRST: u8 = 1 << 0;
    pub const CTL_RUN: u8 = 1 << 1;
    pub const CTL_IOCE: u8 = 1 << 2;
    pub const CTL_FEIE: u8 = 1 << 3;
    pub const CTL_DEIE: u8 = 1 << 4;
    pub const CTL2: u64 = 0x02;
    /// Stav (u8): bit 2 BCIS (dokoncen buffer), 3 FIFOE, 4 DESE; zapis 1 maze.
    pub const STS: u64 = 0x03;
    pub const STS_BCIS: u8 = 1 << 2;
    pub const STS_FIFOE: u8 = 1 << 3;
    pub const STS_DESE: u8 = 1 << 4;
    /// Pozice v bufferu (bajty, u32).
    pub const LPIB: u64 = 0x04;
    /// Delka cykliceho bufferu (bajty, u32).
    pub const CBL: u64 = 0x08;
    /// Index posledni platne polozky BDL (u16).
    pub const LVI: u64 = 0x0C;
    pub const FIFOS: u64 = 0x10;
    /// Format (u16), viz [`crate::format`].
    pub const FMT: u64 = 0x12;
    pub const BDPL: u64 = 0x18;
    pub const BDPU: u64 = 0x1C;
}

/// Offset deskriptoru streamu `index` (pocitano od prvniho vstupniho).
pub const fn sd_offset(index: u32) -> u64 {
    SD_BASE + index as u64 * SD_SIZE
}

/// Rozlozeni GCAP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gcap {
    pub output_streams: u32,
    pub input_streams: u32,
    pub bidi_streams: u32,
    pub addr64: bool,
}

impl Gcap {
    pub fn decode(v: u16) -> Self {
        Self {
            output_streams: (v >> 12 & 0xF) as u32,
            input_streams: (v >> 8 & 0xF) as u32,
            bidi_streams: (v >> 3 & 0x1F) as u32,
            addr64: v & 1 != 0,
        }
    }

    /// Index prvniho vystupniho deskriptoru (za vstupnimi).
    pub fn first_output(&self) -> u32 {
        self.input_streams
    }
}

/// Polozka seznamu bufferu (BDL): 16 bajtu.
pub const BDL_ENTRY: usize = 16;
/// Priznak polozky BDL: preruseni po dokonceni (IOC).
pub const BDL_IOC: u32 = 1;
/// Zarovnani adresy bufferu i BDL (bajty).
pub const BDL_ALIGN: u64 = 128;

/// Zakoduje polozku BDL.
pub fn bdl_entry(addr: u64, len: u32, ioc: bool) -> [u8; BDL_ENTRY] {
    let mut e = [0u8; BDL_ENTRY];
    e[0..8].copy_from_slice(&addr.to_le_bytes());
    e[8..12].copy_from_slice(&len.to_le_bytes());
    e[12..16].copy_from_slice(&(if ioc { BDL_IOC } else { 0 }).to_le_bytes());
    e
}

/// Velikost CORB/RIRB: (kod do registru, pocet polozek) - nejvetsi, kterou
/// radic umi (bity schopnosti 7:4).
pub fn ring_size(size_reg: u8) -> (u8, u32) {
    if size_reg & 0x40 != 0 {
        (2, 256)
    } else if size_reg & 0x20 != 0 {
        (1, 16)
    } else {
        (0, 2)
    }
}
