# libs/osrt

Běhové prostředí `std` pro cíl `*-unknown-magnumopus` (popis `docs/cs/rust-std.md`). `std` (fork v `third_party/rust-std/std.patch`) obsahuje jen lepidlo, logika je tady: start procesu, heap (dlmalloc nad VMO), TLS, vlákna, futex, stdio, prostředí, čas, soubory (fsproto v2), síť (netproto), procesy (launchd) a roury.

## Klíčové soubory

- `src/start.rs` - `_start`, startovní zpráva, rozdělení handlů podle štítků `abi::params::tag`.
- `src/tls.rs` - bloky TLS (local-exec, varianta II x86_64 / I aarch64; posuny musí sedět s lld).
- `src/stdio.rs`, `src/pipe.rs` - protokol proudu bajtů; `src/process.rs` - `LAUNCH` ke launchd; `src/net.rs` - std::net (zámky do netproto v2).

## Invarianty

- Crate se překládá jako závislost std (`rustc-dep-of-std`): závislosti jen naše crates s toutéž feature a vendorované crates z rust-src; libos vždy s `default-features = false` (bez `rt`).
- Handle vlastního procesu se nesmí zavřít (`start::init` ho zapomene).
- Nové vlákno nastaví TP jako první věc, před jakýmkoli kódem std.

## Testy

- `cargo check -p osrt --target x86_64-unknown-none` (rychlá kontrola), QEMU: `cargo xtask test --only "std,std sit,std procesy,std porty"`.
- Změna lepidla std: upravit `target/std-src/<nightly>/library`, `cargo xtask std`, `cargo xtask std-patch`, commit `std.patch`.

## Pasti

- Cargo nesleduje změnu sysrootu: aplikace mají v příznacích otisk sysrootu (`stdsys::app_rustflags`), jinak by zůstaly se starým std.
- Rozhraní používá jen typy `core`/`alloc`/`abi`; převody na typy std dělá lepidlo.
