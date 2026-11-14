# user/ld

Zavaděč sdílených knihoven `ld.so` (v initfs `lib/ld.so`, `PT_INTERP` = `/system/lib/ld.so`): static-PIE v Rustu, nahraje `DT_NEEDED` od libd, relokuje, statické TLS, pečeť kódu a RELRO, inicializátory, skočí na program; za běhu `dlopen`/`dlsym`/`dlclose`, líné TLS přes DTV. Logika bez systému je v `libs/dynload`. Popis: `docs/cs/dynamicke-linkovani.md`.

## Klíčové soubory

- `src/start.rs` - vstup v asembleru a samorelokace; `src/load.rs` - zpráva zavaděče, auxv, start programu; `src/module.rs` - modul: nahrání od libd, relokace v rozsahu, pečeť, init (chyby vrací); `src/dl.rs` - `dlopen`/`dlsym`/`dlclose`/`dlerror`; `src/tls.rs` - `__tls_get_addr` a resolvery TLSDESC (statický, dynamický v asm); `src/iface.rs` - stav (`STATE`), tabulka zavaděče, bloky TLS vláken, fini, `dl_iterate_phdr`, `r_debug`; `src/heap.rs` - alokátor.

## Invarianty

- Před `relocate_self` žádný přístup k datům s ukazateli (vtabulky, statické ukazatele, formátování): jen čtení/zápis přes ukazatele.
- Zavaděč nečte startovní zprávu (patří programu): jen první zprávu `LOADER_MAGIC` (proces, spojení s libd). Knihovny jen od `libd` (initfs nemá); kód knihoven, RELRO všech modulů i své vlastní zapečetí (`PROT_SEAL`).
- Tabulka `iface::INTERFACE` (`abi::loader`) se volá z vláken programu: stav pod `iface::STATE`, `dl*` navíc pod rekurzivním zámkem `dl::DL`. Inicializátory, callback `dl_iterate_phdr` a resolver IFUNC z `dlsym` běží **bez** `STATE` (smí volat zavaděč a sahat na TLS).
- `dlopen` nesmí nechat polovičatý stav: nové moduly se zapečetí a TLS zapíše do vláken až po úspěšné relokaci; při chybě `module::discard` a vrácení rezervy.
- Resolver dynamického TLSDESC (`tls.rs`) mění jen `rax`/`x0` a příznaky: pomalá cesta ukládá všechny registry, které smí měnit Rust (x86_64 GPR + `xmm0-15`, zavaděč bez AVX; aarch64 `x1-x18`, `q0-q31`).
- Čísla modulů TLS se neopakují a DTV má pevnou velikost (`tls::MAX_MODULES`): prázdná položka = líný blok ještě není. Moduly se neodstraňují.
- Heap zavaděče nesmí být na pevné adrese heapu libos (`heap.rs`, rezervaci vybírá jádro); libos bez feature `rt`.
- Segmenty bez zápisu vždy zapečetit (W^X); TEXTREL a REL odmítá už `libs/elf`.
- Překládá ho `xtask/src/dynlink.rs` zvlášť (`relocation-model=pie`, `-pie --no-dynamic-linker`, vlastní adresář `target/user-pie`), ne `cargo_build`.

## Testy

- Host: `cargo test -p dynload`. QEMU: `cargo xtask test --only procesy` (testy `dynamicke linkovani: ...`, programy `user/dltest` a `user/dltest-rs`; `dlopen` a líné TLS ve vláknech testuje `dltest-rs`, rezervu `dltest-dl`).
