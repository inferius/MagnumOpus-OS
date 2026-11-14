//! Ochrana zasobniku pro kod v C (`-fstack-protector-strong`).
//!
//! Prekladac vlozi do funkci s poli na zasobniku "kanarek": na zacatku
//! zkopiruje `__stack_chk_guard` za lokalni promenne a pred navratem ho
//! porovna. Prepsany kanarek (preteceni pole na zasobniku) = volani
//! `__stack_chk_fail`, ktere proces ukonci driv, nez by se skocilo na
//! podvrzenou navratovou adresu.
//!
//! Hodnota kanarku je nahodna od [`init_stack_guard`] (program ho vola
//! pred prvnim kodem v C); vychozi hodnota je jen nouzova. Nejnizsi bajt je
//! vzdy nula: retezcove funkce (strcpy) kanarek neprepisou "spravnou"
//! hodnotou, protoze nulou konci.

/// Kanarek; zmenit jen pred prvnim volanim kodu v C (jinak by aktivni ramce
/// meli starou hodnotu a skoncily chybou).
#[cfg_attr(not(test), unsafe(no_mangle))]
#[allow(non_upper_case_globals)]
pub static mut __stack_chk_guard: usize = 0x2f8a_61c5_93e7_d400;

/// Nastavi kanarek z nahodneho cisla (jednou na zacatku programu).
pub fn init_stack_guard(seed: u64) {
    // SAFETY: vola se pred prvnim kodem v C (viz dokumentace), jedno vlakno.
    unsafe { __stack_chk_guard = (seed as usize) & !0xFF };
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn __stack_chk_fail() -> ! {
    crate::stdio::flush();
    panic!("cstd: preteceni pole na zasobniku v kodu C (kanarek prepsan)");
}
