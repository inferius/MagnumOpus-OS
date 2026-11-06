# Sestavení, spuštění, testy, ladění

## Nástroje

- **Rust přes rustup.** Nightly, komponenty a targety jsou v
  `rust-toolchain.toml`; rustup je stáhne při prvním buildu sám.
- **QEMU** (`qemu-system-x86_64`) s firmwarem edk2/OVMF:
  - Windows: `winget install SoftwareFreedomConservancy.QEMU`
  - Debian/Ubuntu: `apt install qemu-system-x86 ovmf`
  - Fedora: `dnf install qemu-system-x86 edk2-ovmf`
  - Arch: `pacman -S qemu-desktop edk2-ovmf`
  - macOS: `brew install qemu`
- Volitelně **GDB** pro ladění.

xtask hledá QEMU v `PATH` a na obvyklých místech, OVMF vedle QEMU nebo
v `/usr/share/...`. Jinak proměnné prostředí `QEMU`, `OVMF_CODE`, `OVMF_VARS`.

## Příkazy

| Příkaz | Co dělá |
|---|---|
| `cargo xtask build` | přeloží jádro, loader a programy userspace, sestaví initfs |
| `cargo xtask image` | build + obraz disku `target/os-x86_64-<profil>.img` (GPT, ESP FAT32) |
| `cargo xtask run` | image + QEMU s oknem; sériová linka v terminálu |
| `cargo xtask test` | testy knihoven na hostiteli, pak jádro a userspace v QEMU bez okna |

Přepínače: `--release`, `--headless` (bez okna), `--gdb` (QEMU čeká na
GDB), `--accel <tcg|kvm|whpx|hvf>`, `-- <argumenty QEMU>` (např.
`-- -smp 8`, `-- -cpu max,-x2apic`).

## Obsah disku

```text
EFI/BOOT/BOOTX64.EFI   UEFI loader
os/kernel.elf          jádro (ELF s ladicími symboly)
os/initfs.img          initfs: programy init a echo
```

## Výstup a dohledání chyb

- Sériová linka se ukládá do `target/serial.log` (testy:
  `target/serial-test.log`).
- Při panice nebo fatální výjimce xtask po skončení QEMU přeloží adresy ze
  zásobníku na jména funkcí (`llvm-nm` z komponenty `llvm-tools`).
- Úroveň logu jádra: [jadro.md](jadro.md#logování); hlášky: [hlasky.md](hlasky.md).

## Ladění přes GDB

```bash
cargo xtask run --gdb
gdb target/x86_64-unknown-none/debug/kernel -ex "target remote localhost:1234" -ex "break _start" -ex "continue"
```

Programy userspace mají vlastní ELF v `target/user/x86_64-unknown-none/<profil>/`.

## Struktura repozitáře

```text
loader/          UEFI loader
kernel/          jádro
  src/arch/x86_64/  vše specifické pro x86_64
  src/mm/           paměť
  src/sched/        plánovač
  src/object/       objekty a handly
  src/syscall/      syscally
  src/log/          logování
  src/tests/        self-testy jádra
libs/abi/        rozhraní jádro <-> userspace (chyby, syscally, práva)
libs/boot-info/  rozhraní loader <-> jádro
libs/elf/        parser ELF
libs/initfs/     formát initfs
libs/libos/      knihovna pro programy
user/init/       první proces (zatím testy userspace)
user/echo/       testovací server pro IPC
xtask/           build, obraz disku, QEMU, testy
docs/            dokumentace (cs, en)
```

## Konvence kódu

- Soubor max. ~1500 řádků; do podmodulů dělit hned, když to dává smysl.
- Testy v oddělených souborech.
- `unsafe` jen kde je nutné, vždy s komentářem `// SAFETY:`.
- Architektura-specifický kód v `arch/<arch>/`.
- Komentáře česky; v kódu jen ASCII interpunkce (`-`, `->`, `...`).
- Nový kód chyby: přidat do `libs/abi/src/error.rs` (další volné číslo)
  a popsat v [chyby.md](chyby.md) i v anglické verzi - jinak selže test.
