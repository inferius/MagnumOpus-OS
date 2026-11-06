fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let target = std::env::var("TARGET").unwrap();
    // Linker skript je stejny pro vsechny architektury (rozlozeni obrazu
    // jadra je obecne, viz linker.ld).
    if target == "x86_64-unknown-none" || target == "aarch64-unknown-none-softfloat" {
        println!("cargo:rustc-link-arg-bins=-T{dir}/linker.ld");
        println!("cargo:rerun-if-changed=linker.ld");
    }
}
