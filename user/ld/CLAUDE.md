# user/ld

Zavaděč sdílených knihoven `ld.so` (v initfs `lib/ld.so`, `PT_INTERP` = `/system/lib/ld.so`): static-PIE v Rustu, nahraje `DT_NEEDED` z initfs, relokuje, statické TLS, RELRO, inicializátory, skočí na program. Logika bez systému je v `libs/dynload`. Popis: `docs/cs/dynamicke-linkovani.md`.

## Klíčové soubory

- `src/start.rs` - vstup v asembleru a samorelokace; `src/load.rs` - zpráva zavaděče, auxv, knihovny, TLS, relokace, RELRO, init; `src/tls.rs` - `__tls_get_addr` a resolver TLSDESC; `src/iface.rs` - tabulka zavaděče (TLS vláken, fini, `dl_iterate_phdr`) a `r_debug`; `src/bump.rs` - alokátor.

## Invarianty

- Před `relocate_self` žádný přístup k datům s ukazateli (vtabulky, statické ukazatele, formátování): jen čtení/zápis přes ukazatele.
- Zavaděč nečte startovní zprávu (patří programu): jen první zprávu `LOADER_MAGIC`. Initfs (handle i mapování) před skokem zavře; handle vlastního procesu si nechá (bloky TLS vláken).
- Tabulka `iface::INTERFACE` (`abi::loader`) se volá z vláken programu: bez alokací (alokátor `bump.rs` neuvolňuje), stav pod `iface::STATE`.
- Heap zavaděče nesmí být na pevné adrese heapu libos (`bump.rs`, adresu vybírá jádro); libos bez feature `rt`.
- Segmenty bez zápisu vždy zapečetit (W^X); TEXTREL a REL odmítá už `libs/elf`.
- Překládá ho `xtask/src/dynlink.rs` zvlášť (`relocation-model=pie`, `-pie --no-dynamic-linker`, vlastní adresář `target/user-pie`), ne `cargo_build`.

## Testy

- Host: `cargo test -p dynload`. QEMU: `cargo xtask test --only procesy` (testy `dynamicke linkovani: ...`, program `user/dltest`).
