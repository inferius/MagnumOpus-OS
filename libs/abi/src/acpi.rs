//! Udaje o ACPI od jadra pro sluzbu ACPI (polozka bootstrapu
//! [`crate::bootstrap::kind::ACPI`]).
//!
//! Jadro samo cte jen staticke tabulky, ktere potrebuje (MADT, MCFG, HPET,
//! IOMMU). Interpretaci AML dela sluzba `acpid` (ACPICA) v userspace; od
//! jadra dostane jen tohle: adresu RSDP, jeho kopii (nektere firmwary maji
//! RSDP v pameti, kterou jadro po startu uvolni) a seznam oblasti pameti
//! firmwaru s tabulkami (AcpiReclaimable, AcpiNvs) - ty smi namapovat
//! (`VMO_CREATE_PHYSICAL` s cache), jinou RAM ne.
//!
//! Format objektu pameti (little endian):
//!
//! | offset | delka | obsah |
//! |--------|-------|-------|
//! | 0      | 4     | magic `"ACPI"` |
//! | 4      | 4     | verze ([`VERSION`]) |
//! | 8      | 8     | fyzicka adresa RSDP |
//! | 16     | 36    | kopie RSDP (ACPI 2.0+, 36 B; ACPI 1.0 jen 20 B, zbytek nuly) |
//! | 52     | 4     | pocet oblasti `n` (nejvys [`MAX_RANGES`]) |
//! | 56     | 16*n  | oblasti: zacatek (8 B), delka (8 B), zarovnane na stranky |

/// "ACPI" v ASCII.
pub const MAGIC: u32 = u32::from_le_bytes(*b"ACPI");
pub const VERSION: u32 = 1;
/// Delka RSDP ve verzi ACPI 2.0 a novejsi.
pub const RSDP_LEN: usize = 36;
/// Nejvic oblasti pameti ACPI v udajich (vic jich firmware nema; nad limit
/// se zbytek vynecha a jadro to zaloguje).
pub const MAX_RANGES: usize = 64;
const HEADER: usize = 56;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub rsdp_phys: u64,
    pub rsdp: [u8; RSDP_LEN],
    /// Oblasti pameti firmwaru (zacatek, delka).
    pub ranges: alloc_free::Ranges,
}

/// Pevne pole oblasti bez alokace (abi je `no_std` bez `alloc`).
pub mod alloc_free {
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Ranges {
        pub items: [(u64, u64); super::MAX_RANGES],
        pub len: usize,
    }

    impl Ranges {
        pub const fn new() -> Self {
            Self { items: [(0, 0); super::MAX_RANGES], len: 0 }
        }

        /// Prida oblast; plne pole = `false`.
        pub fn push(&mut self, r: (u64, u64)) -> bool {
            if self.len == self.items.len() {
                return false;
            }
            self.items[self.len] = r;
            self.len += 1;
            true
        }

        pub fn as_slice(&self) -> &[(u64, u64)] {
            &self.items[..self.len]
        }
    }

    impl Default for Ranges {
        fn default() -> Self {
            Self::new()
        }
    }
}

impl Info {
    /// Velikost zakodovanych udaju.
    pub fn encoded_len(&self) -> usize {
        HEADER + 16 * self.ranges.len
    }

    /// Zakoduje do `out` (delka aspon [`Info::encoded_len`]); vraci delku.
    pub fn encode(&self, out: &mut [u8]) -> Option<usize> {
        let n = self.encoded_len();
        let out = out.get_mut(..n)?;
        out.fill(0);
        out[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        out[4..8].copy_from_slice(&VERSION.to_le_bytes());
        out[8..16].copy_from_slice(&self.rsdp_phys.to_le_bytes());
        out[16..52].copy_from_slice(&self.rsdp);
        out[52..56].copy_from_slice(&(self.ranges.len as u32).to_le_bytes());
        for (i, &(s, l)) in self.ranges.as_slice().iter().enumerate() {
            let o = HEADER + 16 * i;
            out[o..o + 8].copy_from_slice(&s.to_le_bytes());
            out[o + 8..o + 16].copy_from_slice(&l.to_le_bytes());
        }
        Some(n)
    }

    /// Dekoduje; spatny magic, verze nebo delka = `None`.
    pub fn decode(b: &[u8]) -> Option<Self> {
        let u32_at = |o: usize| Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?));
        let u64_at = |o: usize| Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?));
        if u32_at(0)? != MAGIC || u32_at(4)? != VERSION {
            return None;
        }
        let count = u32_at(52)? as usize;
        if count > MAX_RANGES {
            return None;
        }
        let mut info = Info { rsdp_phys: u64_at(8)?, rsdp: b.get(16..52)?.try_into().ok()?, ranges: alloc_free::Ranges::new() };
        for i in 0..count {
            let o = HEADER + 16 * i;
            info.ranges.push((u64_at(o)?, u64_at(o + 8)?));
        }
        Some(info)
    }
}
