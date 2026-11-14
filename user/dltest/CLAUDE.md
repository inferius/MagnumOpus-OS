# user/dltest

Testovací moduly v C pro dynamické linkování (DL-2): `crt.c` (minimální start programu PIE), `libdltest2.c` a `libdltest.c` (knihovny), `main.c` (program `dltest`), `missing.c` + `none.c` (program s knihovnou, která v obrazu není); pro `dlopen` (DL-4b) `plug.c` + `plug_gd.c` (dynamické TLS: `libdlplug` s TLSDESC, na x86_64 `-mtls-dialect=gnu2`; `libdlplug-gd` s `__tls_get_addr`), `static.c` (initial-exec do rezervy), `big.c` (4 KiB initial-exec, do výchozí rezervy se nevejde), `main_dl.c` (program `dltest-dl` pro test rezervy z voleb spuštění). Nejsou to crate: překládá je `xtask/src/dynlink.rs` přes `cbuild::Build::link`. Test v QEMU: `user/init/src/tests/dynlink.rs` (sada `procesy`). Popis: `docs/cs/dynamicke-linkovani.md`.

## Pasti

- Kód se překládá s `-Werror` a ochranou zásobníku: `__stack_chk_guard`/`__stack_chk_fail` definuje `libdltest2` (zatím není `libc.so`).
- Návratový kód `dltest`: bity 0-7 = chyby, od bitu 8 část adresy programu (test ASLR).
