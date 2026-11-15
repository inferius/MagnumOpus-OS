# libs/osrt

Běhové prostředí `std` pro cíl `*-unknown-magnumopus` (popis `docs/cs/rust-std.md`). `std` (fork v `third_party/rust-std/std.patch`) obsahuje jen lepidlo, logika je tady: start procesu, heap (dlmalloc nad VMO), TLS, vlákna, futex, stdio, prostředí, čas, soubory (fsproto v2), síť (netproto), procesy (launchd) a roury.

## Klíčové soubory

- `src/start.rs` - `_start`, startovní zpráva, rozdělení handlů podle štítků `abi::params::tag`; volá `main(argc, argv)` (z rustc nebo z programu v C, `c_argv`). Netagované handly (okno aplikace) drží pro `take_handles` (jednou; bere je `libs/mosdl`), `self_process` pro další kopii libos.
- `src/lib.rs` - `exit` (nejdřív funkce `at_exit` v obráceném pořadí), `exit_now` (bez nich, C `_Exit`), `at_exit` (max 1024; cstd sem dává vyprázdnění stdio a jednu funkci pro svůj seznam `atexit`/`__cxa_atexit`, v std `os::magnumopus::rt`); `fill_random_seeded` (C `getentropy`).
- `src/env.rs` - argumenty, proměnné, pracovní adresář; C `environ` (`#[no_mangle]`) se přestaví při startu a každé změně proměnných pod zámkem `VARS`, stará pole se neuvolňují.
- `src/tls.rs` - bloky TLS (local-exec, varianta II x86_64 / I aarch64; posuny musí sedět s lld).
- `src/fd.rs` - tabulka čísel fd pro cstd (popis otevření: soubor, stdio, roura; `dup` sdílí popis); std ji vystavuje jako `std::os::magnumopus::runtime` (re-export celého crate, nestabilní).
- `src/sock.rs` - sockety pro tabulku fd (stav nový/TCP/naslouchání/UDP nad netproto, neblokující = `Deadline::TRY`); `src/poll.rs` - čekání na více fd (signály kanálů, připravenost socketů, do 64 `OBJECT_WAIT_MANY`, víc port).
- `src/fs.rs` - soubory nad fsproto; `File::path` (absolutní cesta při otevření, `openat` v cstd), `volume_space` (C `statvfs`).
- `src/memfd.rs` - paměťový fd (`fd::Object::Memory`): zvětšitelný objekt paměti s pozicí, pečetě, `map` (MAP_SHARED přímo na objekt), `export`/`import` handlu (jen pro čtení = bez `WRITE` a `RESIZE`). Výchozí kapacita 64 MiB.
- `src/vm.rs` - re-export `libos::vm` a příznaků `abi::vm` pro `mmap` v cstd.
- `src/inherit.rs` - dědění fd potomkem `posix_spawn`: `export` (popis -> položka `abi::params::Fd` + handle; socket/`epoll` = `NotSupported`), `import` při startu po relaci FS (soubor `fsproto::v2::Identity` + `REOPEN`, nepovedený se přeskočí); `process::spawn_with_fds`.
- `src/stdio.rs`, `src/pipe.rs` - protokol proudu bajtů; `src/process.rs` - `LAUNCH` ke launchd, `wait_any` (C `waitpid(-1)`); `src/net.rs` - std::net (zámky do netproto v2), `interface` (C `getifaddrs`).
- `src/fs.rs` - soubory nad fsproto v2; `File::path` (cesta z doby otevření, C `fchdir`), `volume` (C `statfs`); `src/thread.rs` - `spawn`, `spawn_on` (vlákno na zásobníku volajícího, C `pthread_attr_setstack`).

## Invarianty

- Crate se překládá jako závislost std (`rustc-dep-of-std`): závislosti jen naše crates s toutéž feature a vendorované crates z rust-src; libos vždy s `default-features = false` (bez `rt`).
- Handle vlastního procesu se nesmí zavřít (`start::init` ho zapomene).
- Nové vlákno nastaví TP jako první věc, před jakýmkoli kódem std.
- `at_exit` volá funkce bez drženého zámku (smí registrovat další); `exit_now` je nevolá.
- Cíl `*-unknown-magnumopus-dyn` (`cfg(target_env = "dyn")`, std jako PIC v `libc.so`): žádný `_start` ani vlastní blok TLS (TLS dělá `ld.so`), start je `__libc_start(main, bootstrap, sp, magic)` z `crt1.o` programu, `exit` volá `libos::loader::run_fini`. Symboly linker skriptu (`__tls_*`) jen ve statickém cíli.
- `thread::exit_current` (C `pthread_exit`) ukončí vlákno syscallem bez destruktorů std; zásobník a TLS uvolní až `Thread::join`.

## Testy

- `cargo check -p osrt --target x86_64-unknown-none` (rychlá kontrola), QEMU: `cargo xtask test --only "std,std sit,std procesy,std porty,cstd"`.
- Změna lepidla std: upravit `target/std-src/<nightly>/library`, `cargo xtask std`, `cargo xtask std-patch`, commit `std.patch`.

## Pasti

- Cargo nesleduje změnu sysrootu: aplikace mají v příznacích otisk sysrootu (`stdsys::app_rustflags`), jinak by zůstaly se starým std.
- Rozhraní používá jen typy `core`/`alloc`/`abi`; převody na typy std dělá lepidlo.
