//! Preklad testu v C (`c/ctest.c`) s hlavickami cstd (`DEP_CSTD_INCLUDE`).

fn main() {
    let includes = std::env::var("DEP_CSTD_INCLUDE").expect("cstd predava adresare hlavicek");
    let mut b = cbuild::Build::new("ctest").trust(cbuild::Trust::Own).file("c/ctest.c").file("c/posix.c").file("c/threads.c").file("c/timecheck.c").file("c/mathcheck.c").file("c/netcheck.c").file("c/mmapcheck.c").file("c/sigcheck.c").file("c/lockcheck.c").file("c/wcharcheck.c").file("c/spawncheck.c");
    for dir in includes.split(';') {
        b = b.include(dir);
        println!("cargo::rerun-if-changed={dir}");
    }
    b.compile();
    println!("cargo::rerun-if-changed=c");
}
