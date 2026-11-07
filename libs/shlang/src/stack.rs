//! Hlidani hloubky rekurze podle skutecne spotreby zasobniku (parser
//! i interpret): pocet urovni nestaci, ramec se lisi podle prekladu.

/// Adresa na zasobniku volajiciho (zasobnik roste dolu).
#[inline(never)]
pub fn addr() -> usize {
    let x = 0u8;
    core::hint::black_box(&x) as *const u8 as usize
}
