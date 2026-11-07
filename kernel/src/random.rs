//! Nahodna cisla jadra: zasobnik entropie a generator (crate `csprng`).
//!
//! Zdroje a kredit (odhad skutecne nahody):
//!
//! | zdroj | kdy | kredit |
//! |---|---|---|
//! | `arch::hw_random` (RDSEED/RDRAND) | start, kazdy reseed | 32 bitu z 64 |
//! | jitter citace cyklu | start | 0 |
//! | cas firmwaru, adresy BootInfo | start | 0 |
//! | casovani preruseni zarizeni | prubezne (bez zamku) | 0 |
//! | `RANDOM_ADD` (ovladac virtio-rng) | po startu ovladace a pak periodicky | podle ovladace, nejvys 8 bitu na bajt |
//!
//! Hardwarovy zdroj se pocita jen napul: RDRAND je DRBG uvnitr CPU, jehoz
//! kvalitu nejde overit. Reseed probehne nejpozdeji po [`RESEED_NS`] nebo
//! kdyz ceka kredit aspon `SEED_BITS`; pred nasazenim pri kazdem
//! pozadavku.
//!
//! RIZIKO: bez RDRAND/RDSEED a bez ovladace virtio-rng zustane generator
//! nenasazeny (vystup jen z casu a jitteru). `RANDOM_GET` pak vraci 0
//! a s priznakem `REQUIRE_SEEDED` odmita (`SHOULD_WAIT`).

use core::sync::atomic::{AtomicU64, Ordering};

use boot_info::BootInfo;
use csprng::{Generator, SEED_BITS};

use crate::arch;
use crate::log;
use crate::sync::SpinLock;

/// Nejdelsi interval mezi reseedy.
const RESEED_NS: u64 = 10_000_000_000;
/// Kredit jednoho slova z hardwaroveho generatoru CPU.
const HW_CREDIT_BITS: u32 = 32;

struct State {
    generator: Generator,
    last_reseed_ns: u64,
}

static STATE: SpinLock<State> = SpinLock::new(State {
    generator: Generator::new_const(),
    last_reseed_ns: 0,
});

/// Casovani preruseni: michane bez zamku (vola se v obsluze preruseni).
static IRQ_MIX: AtomicU64 = AtomicU64::new(0);
static IRQ_COUNT: AtomicU64 = AtomicU64::new(0);

/// Pridat slova z hardwaroveho generatoru CPU; vraci kolik se povedlo.
fn add_hw(g: &mut Generator, words: usize) -> usize {
    let mut n = 0;
    for _ in 0..words {
        if let Some(v) = arch::hw_random() {
            g.add(&v.to_le_bytes(), HW_CREDIT_BITS);
            n += 1;
        }
    }
    n
}

/// Prvni naplneni zasobniku pri startu (po inicializaci casovacu).
pub fn init(boot_info: &BootInfo) {
    let mut s = STATE.lock();
    let g = &mut s.generator;
    let hw = add_hw(g, 16);
    // Jitter: rozdily citace cyklu mezi tesne po sobe jdoucimi ctenimi.
    let mut prev = arch::cycle_counter();
    let mut jitter = [0u8; 256];
    for b in jitter.iter_mut() {
        for _ in 0..8 {
            core::hint::spin_loop();
        }
        let now = arch::cycle_counter();
        *b = (now.wrapping_sub(prev) as u8) ^ (now >> 8) as u8;
        prev = now;
    }
    g.add(&jitter, 0);
    g.add(&boot_info.firmware_time.secs.to_le_bytes(), 0);
    g.add(&boot_info.firmware_time.counter.to_le_bytes(), 0);
    g.add(&(boot_info as *const BootInfo as u64).to_le_bytes(), 0);
    g.add(&arch::now_ns().to_le_bytes(), 0);
    g.reseed();
    s.last_reseed_ns = arch::now_ns();
    if s.generator.is_seeded() {
        log::info!("nahodna cisla: generator nasazeny (hardwarovy zdroj CPU, {hw} slov)");
    } else {
        log::warn!(
            "nahodna cisla: CPU nema hardwarovy generator, generator ceka na entropii (virtio-rng)"
        );
    }
}

/// Casovani preruseni zarizeni (z `trap::device_irq`, bez zamku).
#[inline]
pub fn on_interrupt(irq: u32) {
    let t = arch::cycle_counter() ^ ((irq as u64) << 56);
    // Neatomicky souhrn staci: zalezi jen na tom, ze se cas premicha.
    let old = IRQ_MIX.load(Ordering::Relaxed);
    IRQ_MIX.store(old.rotate_left(7) ^ t, Ordering::Relaxed);
    IRQ_COUNT.fetch_add(1, Ordering::Relaxed);
}

fn maybe_reseed(s: &mut State) {
    let now = arch::now_ns();
    let g = &mut s.generator;
    let due = !g.is_seeded()
        || g.pending_bits() >= SEED_BITS
        || now.saturating_sub(s.last_reseed_ns) >= RESEED_NS;
    if !due {
        return;
    }
    add_hw(g, 4);
    let mix = IRQ_MIX.swap(0, Ordering::Relaxed);
    let count = IRQ_COUNT.swap(0, Ordering::Relaxed);
    g.add(&mix.to_le_bytes(), 0);
    g.add(&count.to_le_bytes(), 0);
    g.add(&now.to_le_bytes(), 0);
    let was = g.is_seeded();
    g.reseed();
    s.last_reseed_ns = now;
    if !was && s.generator.is_seeded() {
        log::info!("nahodna cisla: generator nasazeny");
    }
}

/// Naplni `out` nahodnymi bajty; vraci, zda je generator nasazeny.
pub fn fill(out: &mut [u8]) -> bool {
    let mut s = STATE.lock();
    maybe_reseed(&mut s);
    s.generator.fill(out);
    s.generator.is_seeded()
}

pub fn is_seeded() -> bool {
    STATE.lock().generator.is_seeded()
}

/// Entropie od ovladace (`RANDOM_ADD`) s kreditem `credit_bits`.
pub fn add(data: &[u8], credit_bits: u32) {
    let mut s = STATE.lock();
    s.generator.add(data, credit_bits);
    maybe_reseed(&mut s);
}
