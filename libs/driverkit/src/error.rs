//! Chyby ovladace: co se stalo a co s tim.
//!
//! Kazda varianta nese dost informaci, aby hlaska v logu stacila k oprave
//! bez ladeni: ktery BAR, ktery buffer, jaky offset, jaka kvota. Text
//! (`Display`) je cesky jako ostatni hlasky systemu; rada za `->` rika, co
//! zmenit (manifest, kod ovladace, model v simulaci).

use alloc::string::String;
use core::fmt;

pub type Result<T, E = DriverError> = core::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverError {
    /// Volani jadra selhalo (`op` = co ovladac delal).
    Os { op: &'static str, err: abi::Error },
    /// Ovladac sahl na prostredek, ktery nedostal (BAR, I/O port, pravo).
    MissingRight { what: String, hint: &'static str },
    /// DMA pamet nad kvotu z manifestu (`resources.dma_max`).
    DmaQuota { buffer: &'static str, size: u64 },
    /// Preruseni nad kvotu z manifestu (`resources.irq_max`) nebo zarizeni
    /// nema dost vektoru.
    IrqQuota { index: u16, hint: &'static str },
    /// Pristup mimo DMA buffer (ovladac by dal zarizeni adresu mimo
    /// pridelenou pamet, nebo cetl za koncem).
    DmaBounds { buffer: &'static str, off: u64, len: u64, size: u64 },
    /// Zarizeni sahlo pres DMA mimo pamet pridelenou ovladaci (v simulaci;
    /// na skutecnem stroji to zablokuje IOMMU a jadro zaloguje
    /// `IOMMU: zablokovan ...`).
    DmaFault { iova: u64, len: u64, write: bool },
    /// Hodnota se nevejde do pole registru.
    FieldOverflow { reg: &'static str, field: &'static str, value: u64, bits: u32 },
    /// Zarizeni nedokoncilo operaci vcas.
    Timeout { what: &'static str, ms: u64 },
    /// Zarizeni se chova jinak, nez ma (vadne nebo podvrzene zarizeni).
    Device(String),
    /// Kruh je plny.
    RingFull { ring: &'static str },
    /// Spatna konfigurace od spravce zarizeni nebo spatny manifest.
    Config(String),
    /// Klient zavrel kanal (normalni konec ovladace).
    ClientClosed,
}

impl DriverError {
    /// Chyba volani jadra s popisem operace.
    pub fn os(op: &'static str) -> impl FnOnce(abi::Error) -> Self {
        move |err| Self::Os { op, err }
    }

    /// Navratovy kod procesu ovladace pro tuto chybu (viz `Exit::code`).
    pub fn exit_code(&self) -> i64 {
        match self {
            Self::ClientClosed => 0,
            Self::Config(_) => 2,
            Self::MissingRight { .. } | Self::DmaQuota { .. } | Self::IrqQuota { .. } => 3,
            _ => 4,
        }
    }
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Os { op, err } => {
                write!(f, "{op}: {err}")?;
                match err {
                    abi::Error::AccessDenied => write!(
                        f,
                        " -> ovladac na to nema pravo; zkontrolujte resources v manifestu ovladace"
                    ),
                    abi::Error::NoResources => write!(
                        f,
                        " -> vycerpana kvota (resources.irq_max / dma_max v manifestu)"
                    ),
                    _ => Ok(()),
                }
            }
            Self::MissingRight { what, hint } => {
                write!(f, "ovladac nesmi: {what} -> {hint}")
            }
            Self::DmaQuota { buffer, size } => write!(
                f,
                "DMA buffer '{buffer}' ({size} B) se nevejde do kvoty -> zvyste resources.dma_max v manifestu, nebo pouzijte mensi buffer"
            ),
            Self::IrqQuota { index, hint } => {
                write!(f, "preruseni c. {index} nelze zapnout -> {hint}")
            }
            Self::DmaBounds {
                buffer,
                off,
                len,
                size,
            } => write!(
                f,
                "pristup mimo DMA buffer '{buffer}': offset {off:#x} + {len} B > velikost {size} B -> chyba v ovladaci (spatny index nebo delka)"
            ),
            Self::DmaFault { iova, len, write } => write!(
                f,
                "DMA mimo pridelenou pamet: zarizeni {} {len} B na {iova:#x} -> ovladac zapsal do zarizeni adresu, kterou nedostal od DmaBuffer::iova_of (IOMMU by pristup zablokovala)",
                if *write { "zapsalo" } else { "cetlo" }
            ),
            Self::FieldOverflow {
                reg,
                field,
                value,
                bits,
            } => write!(
                f,
                "hodnota {value:#x} se nevejde do pole {reg}.{field} ({bits} b)"
            ),
            Self::Timeout { what, ms } => {
                write!(f, "zarizeni nedokoncilo '{what}' do {ms} ms")
            }
            Self::Device(what) => write!(f, "vadne zarizeni: {what}"),
            Self::RingFull { ring } => write!(f, "kruh '{ring}' je plny"),
            Self::Config(what) => write!(f, "spatna konfigurace: {what}"),
            Self::ClientClosed => write!(f, "klient zavrel kanal"),
        }
    }
}

impl From<abi::Error> for DriverError {
    fn from(err: abi::Error) -> Self {
        if err == abi::Error::PeerClosed {
            return Self::ClientClosed;
        }
        Self::Os { op: "volani jadra", err }
    }
}
