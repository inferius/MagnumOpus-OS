# user/dltest-rs

Testovací program DL-3: Rust (`no_std` + libos) jako PIE se zavaděčem `ld.so` a knihovnami z `user/dltest`. Ověří vlákna libos s TLS knihoven, tabulku zavaděče, `dl_iterate_phdr`, `r_debug` a fini; návratový kód = maska chyb `E_*`. Běžné programy v Rustu zůstávají statické. Překládá ho `xtask/src/dynlink.rs` (`build_rust_test`). Test: `cargo xtask test --only procesy`. Popis: `docs/cs/dynamicke-linkovani.md`.

## Pasti

- Data knihoven jen přes funkce (`dl_fini_get`): `extern static` by v PIE mohl skončit relokací COPY, kterou zavaděč odmítá.
