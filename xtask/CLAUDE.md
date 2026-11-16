# xtask

`cargo xtask <příkaz>`: sestavení jádra, loaderu, userspace a initfs, obraz disku (GPT + ESP), spuštění v QEMU, testy, dokumentace jako web, živé USB, VMware, kostra ovladače. Výchozí `x86_64`, `debug`. Příkazy: `PRIKAZY.md`, `docs/cs/sestaveni.md`.

## Klíčové soubory

- `src/main.rs` - parsování voleb, `host_tests` (seznam crate pro host testy), `check_kernel`, dispatch příkazů.
- `src/qemu.rs`, `qmp.rs`, `testdisks.rs`, `datadisk.rs`, `image.rs` - QEMU, disky, vkládání vstupu přes QMP. `datadisk.rs` s `--wine`: Wine (`target/wine/<arch>/tree` z `cargo xtask wine`, `tools/wsl/build_wine.sh`) do `/apps/org.winehq.wine`, disk 512 MiB jako šablona podle obsahu stromu (kopie pro každý běh); testovací programy Windows (`target/wine/<arch>/tests`, `user/wintest`) do `/apps/org.magnumopus.wintest`.
- `src/mediadisks.rs` - flash disky pro sadu `vymenna media` (FAT32, exFAT, NTFS z `third_party/test-images/ntfs`, superfloppy FAT pro QMP `device_add`) a kontrola zápisu systému na hostiteli po testu (`fatfs`, nezávislý `exfat-fs`; příprava exFAT přes crate `volfs`).
- `src/boottest.rs`, `crashtest.rs`, `powertest.rs` - zvláštní testovací běhy (`--only release|zaznam-padu|opakovany-start|napajeni|zive-usb`).
- `src/docs.rs`, `docs_books.rs`, `docs_check.rs`, `docs_tests.rs`, `highlight.rs` - generátor webu dokumentace a kontrola odkazů, kotev a značek stavu.
- `src/usb/install_test.rs` - test instalátoru (`--only instalace`): instalace ze živého USB na prázdný NVMe a vedle "Windows", start bez flashky, kontrola disku na hostiteli (`image::open_fs`); `--only instalace-gui` totéž přes grafický instalátor se snímkem každého kroku (QMP `screendump`, háček `usb::boot::boot_with`).
- `src/stdsys.rs` - sysroot std: statický cíl a druhý cíl PIC `*-dyn` (`build_sysroot_dyn_locked`, `build_libc_so_locked` -> `target/std-dyn`); `cargo xtask libc-so` sestaví jen `libc.so`.
- `src/csysroot.rs` - sysroot C `target/c-sysroot/<arch>` (hlavičky cstd, `libc.a`, linker skript, `clang.cfg`; dynamická část `add_dynamic`: `libc.so`, `crt1.o`, `clang-dyn.cfg`); `src/cports/` - obecný mechanismus portů v C (`docs/cs/porty-c.md`): `fetch.rs` stažení HTTPS + SHA-256, `tar`, záplaty a overlay z `third_party/c-ports/<port>/`; způsoby `sources.rs` (seznam zdrojů: zlib, sqlite, HarfBuzz; `.cc`/`.cpp` přes clang++ a libc++), `cmake.rs` (CMake + Ninja, soubor nástrojů ze sysrootu `toolchain.cmake` a PIC nad libc.so `toolchain-dyn.cmake`: SDL), `configure.rs` (autotools, FFmpeg); `cxx.rs` běh C++ (LLVM 23.1.5 runtimes, dvě konfigurace: statická a sdílená `libc++.so` + samostatná `libunwind.so` (jeden unwinder na proces, kopie i do `lib/` sysrootu C; sdílená knihovna linkuje `-Wl,-Bdynamic` od začátku, ovladač baremetal jinak bere `-Bstatic`); `clang++.cfg`, `cpptest`), staví se před ostatními porty; `fetch::prepare_subset` rozbalí jen část archivu; `tools.rs` hledání nástrojů s radou k instalaci. Knihovny do `target/c-ports/<arch>/prefix`, programy a data portu (`CPort::data`, `cports::Output`) do initfs. Sysroot má prázdné `libm.a`, `libpthread.a`, `librt.a`.
- `src/dynlink.rs` - zavaděč `ld.so`, testy dynamického linkování, `build_ffdec` (proces dekodéru médií v C nad `libav*.so`).
- `src/image.rs` - obraz disku GPT + ESP, 256 MiB (initfs s knihovnami FFmpeg se do 128 MiB nevejde).
- `src/drivers.rs` - `new-driver` a hledání ovladačů z manifestů; `src/usb/`, `src/venus/`, `*helper.rs` - USB obraz, generátor Venus, pomocné servery pro testy sítě.

## Invarianty

- Programy v userspace se překládají pro JSON cíle `third_party/rust-std/targets/*-unknown-magnumopus.json` (hard-float, x86-64-v2 / NEON) s `-Zbuild-std=core,alloc` (`cargo_build`, `user = true`); jádro a loader mají vestavěné cíle bez FPU. Std aplikace sdílí tytéž JSON cíle s vlastním sysrootem (`stdsys.rs`).
- Každý běh má vlastní testovací a datový disk (`<pid>`), aby šlo pouštět víc vláken souběžně. Nesdílej pevné cesty v `target/`: soubor jen pro běh přes `run_file` (`<pid>` v názvu, smaže se na konci `main`), sdílený výstup (sysroot std, web dokumentace) jen pod zámkem (`lock_shared`) a přepsat jen při změně obsahu.
- `cargo xtask test` = host testy -> `check` ostatních architektur -> docs -> jádro a userspace v QEMU. Vybrané sady userspace: `--only <sada>[,<sada>]`; jména sad jsou `SUITES` v `user/init/src/tests/mod.rs` (mezery v názvech ponech, např. `"ovladace z manifestu"`).
- Obraz USB má dva initfs: `os/initfs.img` (živé, vývojový profil) a `os/instfs.img` (pro instalaci) ze **samostatného sestavení** bez `DEV_PROFILE` a `install-answers` (`usb::install_initfs`). Instalovaný systém nikdy nedostane automatické přihlášení ani režim vývojáře.
- Feature `dev-mode` (režim vývojáře, `src/devmode.rs`) dostávají programy userspace jen při vývojovém profilu; sestavení bez něj hledá v programech `devmode::MARKERS` a nález je chyba. Initfs s profilem a bez něj mají různá jména souboru běhu (`-dev`).
- Chybný odkaz, kotva nebo neznámá značka stavu v docs je chyba sestavení (`docs_check.rs`).
- Jádro má i v profilu `dev` `opt-level = 2` (kořenový `Cargo.toml`); `--kernel-opt <n>` ho pro jeden běh přepíše přes `cargo --config` (`cargo_build`), např. `0` pro krokování v gdb.

## Testy

- Host testy xtasku: `cargo test -p xtask`. Cílená smyčka: `cargo xtask test --only <sada>`, ovladač `--driver <jmeno>`.
- Plný běh obou architektur pouští koordinátor.

## Pasti

- Nový crate s host testy se musí přidat do seznamu `--package` ve `host_tests` (`src/main.rs`) a do `members` kořenového `Cargo.toml`; jinak ho `cargo xtask test` nespustí.
- CMake portů zkouší překlad jen do statické knihovny (`CMAKE_TRY_COMPILE_TARGET_TYPE`), takže `check_library_exists` vždy uspěje: knihovny, které nejsou (libgcc, libdl, libatomic, samostatné `-lc`), musí port vypnout výslovně (viz `cxx.rs`), jinak link `.so` selže nebo do ní vtáhne `libc.a`.
- `libc.so` a dynamická část sysrootu (`add_dynamic`) se staví před porty (sdílené knihovny portů, `libc++.so`).
- `git apply` záplat portů běží v `target/` uvnitř repozitáře: bez `GIT_CEILING_DIRECTORIES` by git našel náš repozitář a záplatu tiše přeskočil (`cports/fetch.rs`).
- Běžící `cargo xtask run` zamyká `xtask.exe`; jiný xtask se nepřeloží, nejdřív zavři QEMU.
- Restart v okně QEMU s WHPX se zasekne (chyba QEMU/WHPX).
- QMP (`qmp.rs`): značky testů se v logu hledají jen za poslední nalezenou (`Log::pos`), jinak by `--repeat` v dalších kolech spouštěl úseky hned nebo vůbec. Nová značka začátku úseku patří do `STARTS` a nesmí být podřetězcem jiné (test `qmp_tests.rs`); chybějící značka uvnitř úseku úsek vzdá, ne celé vkládání.
