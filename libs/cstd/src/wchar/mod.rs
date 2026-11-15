//! `wchar.h` a `wctype.h`: siroke znaky (`wchar_t` = 32 bitu, UTF-32).
//!
//! - [`conv`]: vicebajtove <-> siroke (`mbrtowc`, `wcrtomb`, `mbsrtowcs`,
//!   ...) podle kodovani locale (`LC_CTYPE`, [`crate::locale`]),
//! - [`wcs`]: retezce `wcs*` a `wmem*`, [`num`]: `wcstol`, `wcstod`,
//!   `wcsftime`,
//! - [`wctype`]: tridy znaku a velikost pismen podle Unicode,
//! - [`wio`]: `fgetwc`, `fputwc`, ... nad `FILE`, [`wprintf`] a
//!   [`wscanf`]: formatovany siroky vstup a vystup.

pub mod conv;
pub mod num;
pub mod wcs;
pub mod wctype;
pub mod wio;
pub mod wprintf;
pub mod wscanf;

/// `wchar_t` jako na Linuxu: x86_64 se znamenkem, aarch64 bez.
#[cfg(not(target_arch = "aarch64"))]
pub type wchar_t = i32;
#[cfg(target_arch = "aarch64")]
pub type wchar_t = u32;

/// `wint_t` (32 bitu, `WEOF` = 0xFFFFFFFF).
pub type wint_t = u32;
/// `wctype_t`: cislo tridy (`wctype`).
pub type wctype_t = crate::c_ulong;
/// `wctrans_t`: znacka prevodu (`wctrans`), v C `const int *`.
pub type wctrans_t = *const crate::c_int;

pub const WEOF: wint_t = 0xFFFF_FFFF;

/// `mbstate_t` (8 B, nuly = pocatecni stav).
pub use cstd_core::mb::MbState as mbstate_t;

/// Delka siroheho retezce zakonceneho nulou.
///
/// # Safety
///
/// `s` je platny siroky retezec zakonceny nulou.
pub(crate) unsafe fn wlen(s: *const wchar_t) -> usize {
    let mut n = 0;
    // SAFETY: kontrakt funkce.
    while unsafe { *s.add(n) } != 0 {
        n += 1;
    }
    n
}

/// Siroky retezec jako rez (bez koncove nuly).
///
/// # Safety
///
/// Jako [`wlen`].
pub(crate) unsafe fn wslice<'a>(s: *const wchar_t) -> &'a [wchar_t] {
    // SAFETY: kontrakt funkce.
    unsafe { core::slice::from_raw_parts(s, wlen(s)) }
}
