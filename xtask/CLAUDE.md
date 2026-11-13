# xtask

`cargo xtask <příkaz>`: sestavení jádra, loaderu, userspace a initfs, obraz disku (GPT + ESP), spuštění v QEMU, testy, dokumentace jako web, živé USB, VMware, kostra ovladače. Výchozí `x86_64`, `debug`. Příkazy: `PRIKAZY.md`, `docs/cs/sestaveni.md`.

## Klíčové soubory

- `src/main.rs` - parsování voleb, `host_tests` (seznam crate pro host testy), `check_kernel`, dispatch příkazů.
- `src/qemu.rs`, `qmp.rs`, `testdisks.rs`, `datadisk.rs`, `image.rs` - QEMU, disky, vkládání vstupu přes QMP.
- `src/boottest.rs`, `crashtest.rs`, `powertest.rs` - zvláštní testovací běhy (`--only release|zaznam-padu|opakovany-start|napajeni|zive-usb`).
- `src/docs.rs`, `docs_books.rs`, `docs_check.rs`, `docs_tests.rs`, `highlight.rs` - generátor webu dokumentace a kontrola odkazů, kotev a značek stavu.
- `src/drivers.rs` - `new-driver` a hledání ovladačů z manifestů; `src/usb/`, `src/venus/`, `*helper.rs` - USB obraz, generátor Venus, pomocné servery pro testy sítě.

## Invarianty

- Každý běh má vlastní testovací a datový disk (`<pid>`), aby šlo pouštět víc vláken souběžně. Nesdílej pevné cesty v `target/`.
- `cargo xtask test` = host testy -> `check` ostatních architektur -> docs -> jádro a userspace v QEMU. Vybrané sady userspace: `--only <sada>[,<sada>]`; jména sad jsou `SUITES` v `user/init/src/tests/mod.rs` (mezery v názvech ponech, např. `"ovladace z manifestu"`).
- Chybný odkaz, kotva nebo neznámá značka stavu v docs je chyba sestavení (`docs_check.rs`).

## Testy

- Host testy xtasku: `cargo test -p xtask`. Cílená smyčka: `cargo xtask test --only <sada>`, ovladač `--driver <jmeno>`.
- Plný běh obou architektur pouští koordinátor.

## Pasti

- Nový crate s host testy se musí přidat do seznamu `--package` ve `host_tests` (`src/main.rs`) a do `members` kořenového `Cargo.toml`; jinak ho `cargo xtask test` nespustí.
- Běžící `cargo xtask run` zamyká `xtask.exe`; jiný xtask se nepřeloží, nejdřív zavři QEMU.
- Restart v okně QEMU s WHPX se zasekne (chyba QEMU/WHPX).
