# user/dltest

Testovací moduly v C pro dynamické linkování (DL-2): `crt.c` (minimální start programu PIE), `libdltest2.c` a `libdltest.c` (knihovny), `main.c` (program `dltest`), `missing.c` + `none.c` (program s knihovnou, která v obrazu není). Nejsou to crate: překládá je `xtask/src/dynlink.rs` přes `cbuild::Build::link`. Test v QEMU: `user/init/src/tests/dynlink.rs` (sada `procesy`). Popis: `docs/cs/dynamicke-linkovani.md`.

## Pasti

- Kód se překládá s `-Werror` a ochranou zásobníku: `__stack_chk_guard`/`__stack_chk_fail` definuje `libdltest2` (zatím není `libc.so`).
- Návratový kód `dltest`: bity 0-7 = chyby, od bitu 8 část adresy programu (test ASLR).
