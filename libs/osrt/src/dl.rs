//! Tabulka zavadece ld.so (`dlopen`, `dlsym`, ... pro `dlfcn.h` v cstd):
//! obalka `libos::loader` (cstd nema libos primo). Program bez zavadece
//! (staticky) tabulku nema.

pub use abi::loader::{DlInfo, Interface, PhdrCallback};
pub use libos::loader::interface;
