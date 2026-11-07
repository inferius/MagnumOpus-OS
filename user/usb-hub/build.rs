fn main() {
    // Spolecny linker skript programu v userspace (jen pro target jadra OS).
    if std::env::var("TARGET").unwrap() == "x86_64-unknown-none" {
        let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        println!("cargo:rustc-link-arg-bins=-T{dir}/../user.ld");
        println!("cargo:rerun-if-changed=../user.ld");
    }
}
