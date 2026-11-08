//! Rozpoznani generace podle PCI ID.
//!
//! Seznamy ID: `include/drm/intel/pciids.h` z Linuxu (licence MIT,
//! overeno v hlavicce; prevzata jen fakta - cisla ID). Podporujeme
//! displej verze 9 (Skylake) az 20 (Lunar Lake, Battlemage) - viz
//! `docs/cs/displej.md`, proc tento rozsah. Starsi generace (Gen8 a nize)
//! maji jine roviny (ne "universal plane") a jine porty; Panther Lake
//! (displej 30) zatim jen rozpoznavame a nepouzivame.

/// Rodina displeje (urcuje rozlozeni registru).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Family {
    /// Skylake, Kaby Lake, Coffee/Whiskey/Comet Lake (displej 9).
    Gen9,
    /// Ice Lake, Elkhart/Jasper Lake (displej 11).
    Gen11,
    /// Tiger Lake, Rocket Lake, Alder Lake S, DG1 (displej 12, Xe).
    Gen12,
    /// Alder Lake P/N, Raptor Lake, DG2 (displej 13, Xe-LPD).
    Gen13,
    /// Meteor Lake, Arrow Lake (displej 14, Xe-LPD+).
    Gen14,
    /// Lunar Lake, Battlemage (displej 20, Xe2-LPD).
    Gen20,
}

/// Typ jizni casti (PCH): urcuje piny GMBUS a bity hotplugu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pch {
    /// Sunrise Point (SKL/KBL).
    Spt,
    /// Cannon Point / Comet Point (CFL, CML, WHL).
    Cnp,
    /// Ice Point a novejsi (ICL+).
    Icp,
    /// Diskretni karta (bez PCH, displej na karte).
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Platform {
    pub name: &'static str,
    pub family: Family,
    pub pch: Pch,
    pub discrete: bool,
}

impl Platform {
    /// Nova generace (TGL+): transcoder EDP neexistuje, vyber portu
    /// v `TRANS_DDI_FUNC_CTL` ma 4 bity (cislo portu + 1).
    pub fn tgl_ddi_select(&self) -> bool {
        self.family >= Family::Gen12
    }

    /// Ma transcoder EDP (SKL az ICL).
    pub fn has_edp_transcoder(&self) -> bool {
        self.family <= Family::Gen11
    }

    /// Pocet pipe/transcoderu A az D.
    pub fn pipes(&self) -> usize {
        if self.family >= Family::Gen12 { 4 } else { 3 }
    }

    /// AUX kanal DP od MTL jinde (XELPDP) - zatim nepodporujeme
    /// (jen prevzeti rezimu firmwaru bez EDID).
    pub fn has_legacy_aux(&self) -> bool {
        self.family < Family::Gen14
    }
}

const fn p(name: &'static str, family: Family, pch: Pch, discrete: bool) -> Platform {
    Platform {
        name,
        family,
        pch,
        discrete,
    }
}

const SKL: Platform = p("Skylake", Family::Gen9, Pch::Spt, false);
const KBL: Platform = p("Kaby Lake", Family::Gen9, Pch::Spt, false);
const CFL: Platform = p("Coffee Lake", Family::Gen9, Pch::Cnp, false);
const WHL: Platform = p("Whiskey Lake", Family::Gen9, Pch::Cnp, false);
const CML: Platform = p("Comet Lake", Family::Gen9, Pch::Cnp, false);
const ICL: Platform = p("Ice Lake", Family::Gen11, Pch::Icp, false);
const EHL: Platform = p("Elkhart Lake", Family::Gen11, Pch::Icp, false);
const JSL: Platform = p("Jasper Lake", Family::Gen11, Pch::Icp, false);
const TGL: Platform = p("Tiger Lake", Family::Gen12, Pch::Icp, false);
const RKL: Platform = p("Rocket Lake", Family::Gen12, Pch::Icp, false);
const ADLS: Platform = p("Alder Lake-S", Family::Gen12, Pch::Icp, false);
const ADLP: Platform = p("Alder Lake-P", Family::Gen13, Pch::Icp, false);
const ADLN: Platform = p("Alder Lake-N", Family::Gen13, Pch::Icp, false);
const RPLS: Platform = p("Raptor Lake-S", Family::Gen12, Pch::Icp, false);
const RPLP: Platform = p("Raptor Lake-P", Family::Gen13, Pch::Icp, false);
const DG2: Platform = p("DG2 (Arc Alchemist)", Family::Gen13, Pch::None, true);
const MTL: Platform = p("Meteor Lake", Family::Gen14, Pch::Icp, false);
const ARL: Platform = p("Arrow Lake", Family::Gen14, Pch::Icp, false);
const LNL: Platform = p("Lunar Lake", Family::Gen20, Pch::Icp, false);
const BMG: Platform = p("Battlemage", Family::Gen20, Pch::None, true);

/// (platforma, ID).
const TABLE: &[(Platform, &[u16])] = &[
    (SKL, &[0x1906, 0x1913, 0x190E, 0x1915, 0x1902, 0x190A, 0x190B, 0x1917, 0x1916, 0x1921, 0x191E, 0x1912, 0x191A, 0x191B, 0x191D, 0x1923, 0x1926, 0x1927, 0x192A, 0x192B, 0x192D, 0x1932, 0x193A, 0x193B, 0x193D]),
    (KBL, &[0x5906, 0x5913, 0x590E, 0x5915, 0x5902, 0x5908, 0x590A, 0x590B, 0x5916, 0x5921, 0x591E, 0x5912, 0x5917, 0x591A, 0x591B, 0x591D, 0x5926, 0x5923, 0x5927, 0x593B, 0x591C, 0x87C0]),
    (CFL, &[0x3E90, 0x3E93, 0x3E99, 0x3E91, 0x3E92, 0x3E96, 0x3E98, 0x3E9A, 0x3E9C, 0x3E94, 0x3E9B, 0x3EA9, 0x3EA5, 0x3EA6, 0x3EA7, 0x3EA8, 0x87CA]),
    (WHL, &[0x3EA1, 0x3EA4, 0x3EA0, 0x3EA3, 0x3EA2]),
    (CML, &[0x9BA2, 0x9BA4, 0x9BA5, 0x9BA8, 0x9BC2, 0x9BC4, 0x9BC5, 0x9BC6, 0x9BC8, 0x9BE6, 0x9BF6, 0x9B21, 0x9BAA, 0x9BAC, 0x9B41, 0x9BCA, 0x9BCC]),
    (ICL, &[0x8A50, 0x8A52, 0x8A53, 0x8A54, 0x8A56, 0x8A57, 0x8A58, 0x8A59, 0x8A5A, 0x8A5B, 0x8A5C, 0x8A70, 0x8A71, 0x8A51, 0x8A5D]),
    (EHL, &[0x4541, 0x4551, 0x4555, 0x4557, 0x4570, 0x4571]),
    (JSL, &[0x4E51, 0x4E55, 0x4E57, 0x4E61, 0x4E71]),
    (TGL, &[0x9A60, 0x9A68, 0x9A70, 0x9A40, 0x9A49, 0x9A59, 0x9A78, 0x9AC0, 0x9AC9, 0x9AD9, 0x9AF8]),
    (RKL, &[0x4C80, 0x4C8A, 0x4C8B, 0x4C8C, 0x4C90, 0x4C9A]),
    (ADLS, &[0x4680, 0x4682, 0x4688, 0x468A, 0x468B, 0x4690, 0x4692, 0x4693]),
    (ADLP, &[0x46A0, 0x46A1, 0x46A2, 0x46A3, 0x46A6, 0x46A8, 0x46AA, 0x462A, 0x4626, 0x4628, 0x46B0, 0x46B1, 0x46B2, 0x46B3, 0x46C0, 0x46C1, 0x46C2, 0x46C3]),
    (ADLN, &[0x46D0, 0x46D1, 0x46D2, 0x46D3, 0x46D4]),
    (RPLS, &[0xA780, 0xA781, 0xA782, 0xA783, 0xA788, 0xA789, 0xA78A, 0xA78B]),
    // RPL-U a RPL-P maji displej jako ADL-P.
    (RPLP, &[0xA721, 0xA7A1, 0xA7A9, 0xA7AC, 0xA7AD, 0xA720, 0xA7A0, 0xA7A8, 0xA7AA, 0xA7AB]),
    (DG2, &[0x56A0, 0x56A1, 0x56A2, 0x56BE, 0x56BF, 0x5690, 0x5691, 0x5692, 0x56A5, 0x56A6, 0x56B0, 0x56B1, 0x56BA, 0x56BB, 0x56BC, 0x56BD, 0x5693, 0x5694, 0x5695, 0x56A3, 0x56A4, 0x56B2, 0x56B3, 0x5696, 0x5697]),
    (MTL, &[0x7D40, 0x7D45, 0x7D55, 0x7D60, 0x7DD5]),
    (ARL, &[0x7D51, 0x7DD1, 0x7D41, 0x7D67, 0xB640]),
    (LNL, &[0x6420, 0x64A0, 0x64B0]),
    (BMG, &[0xE202, 0xE209, 0xE20B, 0xE20C, 0xE20D, 0xE210, 0xE211, 0xE212, 0xE216, 0xE220, 0xE221, 0xE222, 0xE223]),
];

/// Panther Lake (displej 30): rozpoznat, ale nepouzit.
const PTL_IDS: &[u16] = &[0xB080, 0xB081, 0xB082, 0xB083, 0xB084, 0xB085, 0xB086, 0xB087, 0xB08F, 0xB090, 0xB0A0, 0xB0A1, 0xB0B0];

/// Vysledek rozpoznani.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detect {
    Supported(Platform),
    /// Znama, ale zatim nepodporovana generace (jmeno).
    Unsupported(&'static str),
    Unknown,
}

pub fn detect(vendor: u16, device: u16) -> Detect {
    if vendor != 0x8086 {
        return Detect::Unknown;
    }
    for (p, ids) in TABLE {
        if ids.contains(&device) {
            return Detect::Supported(*p);
        }
    }
    if PTL_IDS.contains(&device) {
        return Detect::Unsupported("Panther Lake (displej 30)");
    }
    Detect::Unknown
}

/// Pocet podporovanych ID (pro testy).
pub fn supported_ids() -> impl Iterator<Item = (u16, Platform)> {
    TABLE.iter().flat_map(|(p, ids)| ids.iter().map(move |&i| (i, *p)))
}
