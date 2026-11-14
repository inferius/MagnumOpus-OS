//! Preda adresare hlavicek programum v C (`links = "cstd"` -> promenna
//! `DEP_CSTD_INCLUDE` v build.rs zavislych crate). Poradi: hlavicky POSIX
//! (cstd), pak zaklad (cstd-core); POSIX doplnuji zaklad pres `#include_next`.

fn main() {
    let here = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let own = here.join("include");
    let core = here.join("../cstd-core/include");
    println!("cargo::metadata=include={};{}", own.display(), core.display());
    println!("cargo::rerun-if-changed=build.rs");
}
