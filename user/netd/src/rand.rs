//! Nahodna cisla pro sit (semeno TCP, docasne porty, ID dotazu DNS,
//! identifikatory pingu) z generatoru jadra (`RANDOM_GET`, ChaCha20).
//!
//! Kazde cislo jde primo z jadra: zadny vlastni stav, ze ktereho by slo
//! odhadnout dalsi hodnoty. Kdyz jadro jeste neni nasazene (zadny
//! hardwarovy zdroj ani virtio-rng), cisla jsou slabsi a sluzba to zaloguje.

pub struct Rng {
    warned: bool,
}

impl Rng {
    /// Novy generator; `true` = jadro je nasazene.
    pub fn new() -> (Self, bool) {
        let (_, seeded) = libos::random::u64();
        (Self { warned: !seeded }, seeded)
    }

    pub fn next_u64(&mut self) -> u64 {
        let (v, seeded) = libos::random::u64();
        if !seeded && !self.warned {
            libos::println!(
                "netd: VAROVANI: generator jadra neni nasazeny, nahodna cisla jsou slaba"
            );
            self.warned = true;
        }
        v
    }

    pub fn next_u16(&mut self) -> u16 {
        self.next_u64() as u16
    }
}
