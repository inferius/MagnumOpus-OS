# loader

UEFI loader (`no_std`, `no_main`). Z ESP načte `\os\kernel.elf` a `\os\initfs.img`, postaví tabulky stránek (identita, přímá mapa fyzické paměti, jádro v horních 2 GiB), zjistí framebuffer (GOP) a ACPI RSDP, ukončí boot services a skočí do jádra s ukazatelem na `BootInfo`. Hned po převzetí řízení vykreslí startovní obraz. Popis startu: `docs/cs/jadro.md#start`, `docs/cs/start.md`.

## Klíčové soubory

- `src/main.rs` - postup loaderu (kroky 0-5 v hlavičce souboru).
- `src/arch/{x86_64,aarch64}/` - tabulky stránek a skok do jádra (architektonicky závislé části).
- `src/splash.rs` - startovní obraz; `src/logger.rs`; `src/pci_id.rs`.
- Rozhraní k jádru je v `libs/boot-info` (`BootInfo`, `PHYS_MAP_BASE`, `KERNEL_BASE`).

## Invarianty

- `BootInfo` a jeho podstruktury jsou `#[repr(C)]` jen s prostými daty; layout nesmí záviset na verzi kompilátoru. Změna = změna loaderu i jádra naráz (`libs/boot-info`).
- Rozložení adresního prostoru před skokem musí odpovídat `libs/boot-info` a `kernel/linker.ld`.
- Po ukončení boot services už nejsou UEFI služby (logování, alokace) k dispozici.

## Testy

- Loader nemá host testy. Ověření je start v QEMU: `cargo xtask run --headless` nebo `cargo xtask test --only zaklad` (obě architektury: `--arch aarch64`).

## Pasti

- Debug loader mimo QEMU/UEFI nejde; chyby před logem jádra se hledají přes sériový výstup loaderu (`logger.rs`).
- Splash běží před načtením jádra: neprodlužuj start zbytečnou prací.
