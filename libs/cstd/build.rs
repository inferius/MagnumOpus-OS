//! Preda adresare hlavicek programum v C (`links = "cstd"` -> promenna
//! `DEP_CSTD_INCLUDE` v build.rs zavislych crate). Poradi: hlavicky POSIX
//! (cstd), pak zaklad (cstd-core); POSIX doplnuji zaklad pres `#include_next`.
//!
//! Na cili magnumopus navic prelozi cast knihovny psanou v C (`c/`):
//! funkce pro `long double`, jejichz ABI Rust neumi.

fn main() {
    let here = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let own = here.join("include");
    let core = here.join("../cstd-core/include");
    println!("cargo::metadata=include={};{}", own.display(), core.display());
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=c");
    println!("cargo::rerun-if-changed=include");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("magnumopus") {
        cbuild::Build::new("cstd_c")
            .trust(cbuild::Trust::Own)
            .file("c/math_ld.c")
            .include(own.to_str().unwrap())
            .include(core.to_str().unwrap())
            .compile();
    }
}
