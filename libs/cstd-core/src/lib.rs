//! cstd - zaklad vlastni knihovny C systemu.
//!
//! Funkce standardni knihovny C s ABI C, napsane v Rustu: kod v C
//! (prevzaty jako ACPICA, pozdeji prenesene ovladace DRM, vlastni libc)
//! je vola pres hlavicky v `libs/cstd-core/include`. Zatim jen to, co nepotrebuje
//! sluzby systemu (soubory, vlakna, cas):
//!
//! - `string.h`: retezce (`strlen`, `strcmp`, ...) a `memchr`; `memcpy`,
//!   `memmove`, `memset` a `memcmp` dodava `compiler_builtins` (je v kazdem
//!   programu v Rustu, stejne symboly by se tloukly),
//! - `ctype.h`: tridy znaku ASCII,
//! - `stdlib.h`: `malloc`/`calloc`/`realloc`/`free` nad alokatorem programu,
//!   `strtoul` a spol., `abort`,
//! - `stdio.h`: `printf`, `vprintf`, `snprintf`, `vsnprintf`, `puts`,
//!   `putchar` - vystup jde po radcich do logu programu ([`set_output`]),
//! - ochrana zasobniku (`__stack_chk_guard`, `__stack_chk_fail`) pro
//!   priznak `-fstack-protector-strong`, se kterym se vsechno C preklada.
//!
//! Zamerne chybi funkce bez kontroly delky (`sprintf`, `gets`, `strtok`):
//! kod, ktery je potrebuje, se musi upravit, ne prelozit.
//!
//! Crate nezavisi na libos (testy bezi na hostiteli). Program nastavi vystup
//! a seminko kanarku na zacatku `main`, pred prvnim volanim kodu v C:
//!
//! ```ignore
//! cstd::set_output(|b| libos::log::write_bytes(b));
//! cstd::init_stack_guard(libos::random::u64());
//! ```
//!
//! `#![no_builtins]`: LLVM jinak smycku ve `strlen` "optimalizuje" na volani
//! `strlen` (nekonecna rekurze) - stejne jako `compiler_builtins`.

#![cfg_attr(not(test), no_std)]
#![no_builtins]
// `char` bereme jako `u8` (znamenko `char` se mezi x86_64 a aarch64 lisi,
// na ABI nema vliv); rustc by jinak varoval u `strlen` a `memchr`.
#![allow(suspicious_runtime_symbol_definitions)]

extern crate alloc;

pub mod ctype;
mod float;
pub mod format;
pub mod mb;
pub mod ssp;
pub mod stdio;
pub mod stdlib;
pub mod string;

pub use ssp::init_stack_guard;
pub use stdio::{flush, set_output};

/// Typy jazyka C na nasich cilech (LP64: `long` ma 64 bitu).
#[allow(non_camel_case_types)]
pub mod ctypes {
    pub type c_char = u8;
    pub type c_int = i32;
    pub type c_uint = u32;
    pub type c_long = i64;
    pub type c_ulong = u64;
    pub type size_t = usize;
}

#[cfg(test)]
mod format_tests;
#[cfg(test)]
mod string_tests;
#[cfg(test)]
mod stdlib_tests;
#[cfg(test)]
mod alloc_tests;
#[cfg(test)]
mod mb_tests;
