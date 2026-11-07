//! Kryptograficky generator nahodnych cisel (CSPRNG) pro jadro.
//!
//! - **Zasobnik entropie:** vstupy (RDSEED/RDRAND, virtio-rng, casovani
//!   preruseni a jitter citace) se michaji hashem BLAKE2s. Kazdy vstup ma
//!   "kredit" v bitech = odhad, kolik skutecne nahody nese (jitter 0).
//! - **Generator:** ChaCha20 s 256bitovym klicem. Pri **reseedu** je novy
//!   klic `BLAKE2s(stary klic || hash zasobniku)`, takze novy vstup nikdy
//!   nesnizi kvalitu (spatny zdroj nepomuze, ale neuskodi).
//! - **Mazani klice po kazdem pozadavku** ("fast key erasure"): blok 0
//!   proudu se stane novym klicem a vystup jde z bloku 1 a dal. Kdo pozdeji
//!   ziska stav jadra, nespocita drivejsi vystupy.
//! - **Nasazeno** (`seeded`) je generator po reseedu se souctem kreditu
//!   aspon [`SEED_BITS`]. Pred tim vystup zavisi jen na zdrojich bez kreditu
//!   (cas, jitter) a nema se pouzivat na klice.
//!
//! Crate je cisty (bez zamku a hardwaru), testuje se na hostiteli
//! (`cargo test -p csprng`) s vektory z RFC 8439 a RFC 7693.

#![no_std]

mod blake2s;
mod chacha;

#[cfg(test)]
mod tests;

pub use blake2s::Blake2s;
pub use chacha::block as chacha20_block;

/// Kolik bitu kreditu je treba, aby byl generator nasazeny.
pub const SEED_BITS: u32 = 256;

pub struct Generator {
    key: [u8; 32],
    pool: Blake2s,
    /// Kredit vstupu od posledniho reseedu.
    pending_bits: u32,
    seeded: bool,
    reseeds: u64,
}

impl Default for Generator {
    fn default() -> Self {
        Self::new()
    }
}

impl Generator {
    pub const fn new_const() -> Self {
        Self {
            key: [0; 32],
            pool: Blake2s::new(),
            pending_bits: 0,
            seeded: false,
            reseeds: 0,
        }
    }

    pub fn new() -> Self {
        Self::new_const()
    }

    /// Prida vstup do zasobniku s kreditem `credit_bits` (nejvys 8 bitu na
    /// bajt).
    pub fn add(&mut self, data: &[u8], credit_bits: u32) {
        self.pool.update(&(data.len() as u32).to_le_bytes());
        self.pool.update(data);
        let max = (data.len() as u32).saturating_mul(8);
        self.pending_bits = self.pending_bits.saturating_add(credit_bits.min(max));
    }

    /// Kredit cekajici v zasobniku.
    pub fn pending_bits(&self) -> u32 {
        self.pending_bits
    }

    /// Premicha zasobnik do klice. Kdyz zasobnik nesl aspon `SEED_BITS`
    /// kreditu, generator je od ted nasazeny.
    pub fn reseed(&mut self) {
        let pool = core::mem::take(&mut self.pool).finalize();
        let mut h = Blake2s::new();
        h.update(b"MagnumOpus reseed");
        h.update(&self.key);
        h.update(&pool);
        h.update(&self.reseeds.to_le_bytes());
        self.key = h.finalize();
        if self.pending_bits >= SEED_BITS {
            self.seeded = true;
        }
        self.pending_bits = 0;
        self.reseeds += 1;
    }

    pub fn is_seeded(&self) -> bool {
        self.seeded
    }

    pub fn reseeds(&self) -> u64 {
        self.reseeds
    }

    /// Naplni `out` a hned smaze klic, ze ktereho vystup vznikl.
    pub fn fill(&mut self, out: &mut [u8]) {
        const NONCE: [u8; 12] = [0; 12];
        let next_key = chacha::block(&self.key, 0, &NONCE);
        for (i, chunk) in out.chunks_mut(64).enumerate() {
            let b = chacha::block(&self.key, i as u32 + 1, &NONCE);
            chunk.copy_from_slice(&b[..chunk.len()]);
        }
        self.key.copy_from_slice(&next_key[..32]);
    }
}
