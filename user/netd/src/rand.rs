//! Pseudonahodna cisla pro sit: semeno TCP (pocatecni poradova cisla),
//! docasne porty, ID dotazu DNS, identifikatory pingu.
//!
//! RIZIKO: neni to kryptograficky generator. Semeno je z RDRAND (x86_64, kdyz
//! ho procesor ma), casu a MAC. Bez RDRAND jsou ID dotazu DNS a pocatecni
//! cisla TCP odhadnutelna - utocnik na stejne siti by mohl podvrhnout
//! odpoved DNS nebo vlozit data do spojeni. Patri sem zdroj entropie od
//! jadra (TODO: syscall s generatorem z RDRAND/RNDR a sumu preruseni).

/// xorshift64* nad semenem ze splitmix64.
pub struct Rng(u64);

/// Hodnota z instrukce RDRAND, pokud ji procesor ma.
#[cfg(target_arch = "x86_64")]
fn hardware_random() -> Option<u64> {
    // CPUID.1:ECX bit 30 = RDRAND.
    let ecx = core::arch::x86_64::__cpuid(1).ecx;
    if ecx & (1 << 30) == 0 {
        return None;
    }
    for _ in 0..10 {
        let mut v = 0u64;
        // SAFETY: instrukci procesor podporuje (overeno pres CPUID).
        if unsafe { core::arch::x86_64::_rdrand64_step(&mut v) } == 1 {
            return Some(v);
        }
    }
    None
}

#[cfg(not(target_arch = "x86_64"))]
fn hardware_random() -> Option<u64> {
    None
}

fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Rng {
    /// Novy generator; `extra` = dalsi bajty do semene (MAC).
    pub fn new(extra: u64) -> (Self, bool) {
        let hw = hardware_random();
        let wall = libos::time::wall()
            .map(|t| t.secs as u64 ^ (t.nanos as u64) << 32)
            .unwrap_or(0);
        let seed = splitmix(hw.unwrap_or(0) ^ libos::time::now() ^ wall.rotate_left(17) ^ extra);
        (Self(seed | 1), hw.is_some())
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn next_u16(&mut self) -> u16 {
        (self.next_u64() >> 48) as u16
    }
}
