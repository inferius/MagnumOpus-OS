//! DMA po kusech: mapovani pripnute pameti do domeny zarizeni (`DMA_MAP`,
//! `DMA_SEGMENTS`, `DMA_SYNC`).
//!
//! - Mapovat smi ovladac s omezenym prostredkem zarizeni (pravo `MANAGE`,
//!   identita zarizeni). Objekt pameti musi byt pripnuty (`VMO_PINNED` nebo
//!   souvisla pamet pro DMA) a handle musi mit pravo `DMA`; smer navic
//!   `READ` (zarizeni cte) nebo `WRITE` (zarizeni zapisuje).
//! - S IOMMU dostane mapovani jeden souvisly rozsah IOVA v okne nad koncem
//!   RAM (alokator jadra s ochrannou strankou za rozsahem, nebo napoveda
//!   ovladace overena na prekryv) a IOMMU vynucuje smer.
//! - Bez IOMMU jsou useky fyzicke adresy (sousedni stranky slouceny),
//!   napoveda nejde (`NOT_SUPPORTED`) a smer nikdo nevynuti; jadro to
//!   zaloguje jednou na prostredek.
//! - Zavreni handlu = odmapovani: zaznamy zmizi a IOTLB se vyprazdni
//!   synchronne drive, nez handle zmizi (zarizeni pak na stare adrese
//!   narazi na chybu prekladu, kterou jadro zaloguje). Po padu ovladace
//!   jadro nejdriv zastavi zarizeni (karantena DMA), pak odmapuje.
//!
//! Popis: `docs/cs/dma.md`, cisla v `docs/cs/syscally.md`.

/// Priznaky `DMA_MAP`.
pub mod map {
    /// Zarizeni smi z pameti cist (handle objektu pameti s `READ`).
    pub const DEVICE_READ: u64 = 1 << 0;
    /// Zarizeni smi do pameti zapisovat (handle objektu pameti s `WRITE`).
    pub const DEVICE_WRITE: u64 = 1 << 1;
    /// Jen s IOMMU: bez ni `NOT_SUPPORTED` misto fyzickych adres (ovladac,
    /// ktery bez ochrany nesmi pokracovat, nebo test ochrany).
    pub const REQUIRE_IOMMU: u64 = 1 << 2;
    /// Smer pristupu zarizeni.
    pub const ACCESS: u64 = DEVICE_READ | DEVICE_WRITE;
    /// Vsechny zname priznaky.
    pub const KNOWN: u64 = ACCESS | REQUIRE_IOMMU;
}

/// Smer `DMA_SYNC` (udrzba cache na nekoherentnich platformach).
pub mod sync {
    /// CPU zapsal data, zarizeni je bude cist (vycistit cache do pameti).
    pub const FOR_DEVICE: u64 = 0;
    /// Zarizeni zapsalo data, CPU je bude cist (zahodit stare radky cache).
    /// Jadro pritom vybere i cekajici hlaseni chyb IOMMU do logu.
    pub const FOR_CPU: u64 = 1;
}

/// Nejvyssi mozny pocet useku jednoho mapovani (bez IOMMU stranka na
/// usek): mapovani je nejvys `MAP_MAX` bajtu.
pub const MAP_MAX: u64 = 1 << 32;

/// Usek mapovani z pohledu zarizeni (16 B, vystup `DMA_SEGMENTS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct Segment {
    /// Adresa pro zarizeni (IOVA, bez IOMMU fyzicka).
    pub addr: u64,
    pub len: u64,
}

impl Segment {
    pub const SIZE: usize = 16;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[..8].copy_from_slice(&self.addr.to_le_bytes());
        b[8..].copy_from_slice(&self.len.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8; Self::SIZE]) -> Self {
        Self {
            addr: u64::from_le_bytes(b[..8].try_into().unwrap()),
            len: u64::from_le_bytes(b[8..].try_into().unwrap()),
        }
    }
}

/// Slouci serazene fyzicke stranky do souvislych useku (bez IOMMU).
/// `pages` jsou adresy stranek velikosti `page` v poradi mapovani.
pub fn merge_pages(pages: impl IntoIterator<Item = u64>, page: u64) -> impl Iterator<Item = Segment> {
    let mut it = pages.into_iter().peekable();
    core::iter::from_fn(move || {
        let first = it.next()?;
        let mut seg = Segment { addr: first, len: page };
        while let Some(&next) = it.peek() {
            if next != seg.addr + seg.len {
                break;
            }
            seg.len += page;
            it.next();
        }
        Some(seg)
    })
}
