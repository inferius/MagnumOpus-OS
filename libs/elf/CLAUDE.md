# libs/elf

Parser ELF64 (little endian, x86_64 a aarch64). `ElfFile::parse` = statický program (`ET_EXEC`) pro tuto architekturu: používá ho UEFI loader, jádro (init) a libos `spawn`. `ElfFile::parse_object` = i `ET_DYN` (sdílené knihovny, PIE) pro zadanou architekturu: dynamická sekce (`dynamic.rs`), symboly s GNU/SysV hashem (`symbols.rs`), relokace RELA a RELR (`reloc.rs`). Popis: `docs/cs/dynamicke-linkovani.md`.

## Invarianty

- Vstup je nedůvěryhodný: každé čtení přes `bytes`/`u*_at`/`at_vaddr` s kontrolou mezí a přetečení, nikdy panika ani nekonečná smyčka (řetězy hashů mají strop). Nová cesta čtení patří do testu `damaged_input_does_not_panic`.
- `DT_TEXTREL` / `DF_TEXTREL` = `ElfError::TextRelocations` (W^X), `DT_REL` = nepodporováno; nestandardní velikosti položek (RELA 24, RELR 8, SYM 24) = chyba.
- `parse` dál přijímá jen `ET_EXEC` nativní architektury (loader a jádro nesmí nahrát PIE na adresu 0).
- `no_std` bez alloc (loader a jádro): žádné `Vec`, jen řezy do souboru.

## Testy

- `cargo test -p elf` (`src/tests.rs`) proti souborům v `tests/fixtures/` (clang + ld.lld, `gen.sh`; výstupy commitnuté, binární v `.gitattributes`).

## Pasti

- Fixtures se generují v Git Bash: `gen.sh` vypíná převod cest MSYS (jinak se `/system/lib/ld.so` v `PT_INTERP` změní na cestu Windows).
- Knihovnu s TEXTREL pro aarch64 lld nevytvoří (absolutní MOVW); test je jen na x86_64.
