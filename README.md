# os

Nový operační systém psaný od nuly v Rustu. Pracovní název zatím není, v kódu
se používá `os` / `kernel`.

Aktuální stav: **milník 1 (boot)**. Vlastní UEFI loader nahraje kernel, předá
mu mapu paměti, framebuffer a ACPI RSDP, a kernel vypíše text na sériovou
linku i do framebufferu.

## Struktura

```
loader/            UEFI loader (target x86_64-unknown-uefi)
  src/arch/x86_64/   tabulky stránek, skok do kernelu
kernel/            kernel (target x86_64-unknown-none, no_std)
  linker.ld          rozložení kernelu v paměti
  src/arch/x86_64/   sériová linka, CPU
libs/boot-info/    sdílené rozhraní loader <-> kernel (#[repr(C)])
xtask/             build, obraz disku, spouštění QEMU
```

### Boot

1. OVMF spustí `\EFI\BOOT\BOOTX64.EFI` (loader) z EFI System oddílu.
2. Loader načte `\os\kernel.elf` (statický ELF64), segmenty nahraje do
   paměti typu `Kernel` a namapuje je s právy podle ELF (R-X / R-- / RW-, NX).
3. Postaví vlastní tabulky stránek:
   - identita (jen pro přechod, kernel ji později zruší),
   - přímá mapa fyzické paměti od `0xFFFF_8000_0000_0000`,
   - kernel od `0xFFFF_FFFF_8000_0000`.
4. Ukončí boot services, přeloží mapu paměti UEFI do vlastních typů
   (`Usable`, `Reclaimable`, `Kernel`, `BootData`, ...) a skočí do kernelu
   s ukazatelem na `BootInfo` v `rdi`.

## Požadované nástroje

- **Rust přes rustup.** Konkrétní nightly, komponenty a targety jsou
  v `rust-toolchain.toml`; rustup je při prvním buildu stáhne sám.
- **QEMU** (`qemu-system-x86_64`) - verze s firmwarem edk2/OVMF.
  - Windows: `winget install SoftwareFreedomConservancy.QEMU`
    (instaluje do `C:\Program Files\qemu`, OVMF je v `share\`)
  - Debian/Ubuntu: `apt install qemu-system-x86 ovmf`
  - Fedora: `dnf install qemu-system-x86 edk2-ovmf`
  - Arch: `pacman -S qemu-desktop edk2-ovmf`
  - macOS: `brew install qemu`
- Volitelně **GDB** (nebo `rust-gdb` / LLDB) pro ladění.

Žádné další nástroje (mtools, dosfstools, ...) nejsou potřeba, obraz disku
vytváří xtask sám.

xtask hledá QEMU v `PATH` a na obvyklých místech, OVMF vedle QEMU nebo
v `/usr/share/...`. Jinak lze cesty nastavit proměnnými:

| proměnná    | význam                                   |
|-------------|------------------------------------------|
| `QEMU`      | cesta k `qemu-system-x86_64`             |
| `OVMF_CODE` | OVMF code (pflash, jen pro čtení)        |
| `OVMF_VARS` | šablona OVMF vars (xtask si ji zkopíruje) |

## Spuštění

```bash
cargo xtask run
```

Otevře se okno QEMU s výstupem do framebufferu, výstup sériové linky jde do
terminálu. Kernel po výpisu zastaví CPU; QEMU se zavírá zavřením okna nebo
Ctrl+C v terminálu.

Další příkazy a přepínače:

```bash
cargo xtask build              # jen překlad kernelu a loaderu
cargo xtask image              # build + target/os-x86_64-debug.img
cargo xtask run --release      # release profil
cargo xtask run --headless     # bez okna, jen sériová linka
cargo xtask run --accel whpx   # akcelerace (Linux: kvm se volí sám, je-li /dev/kvm)
cargo xtask run -- -smp 8      # cokoli za -- jde přímo do QEMU
```

Výsledný obraz `target/os-x86_64-<profil>.img` je GPT disk s jedním EFI
System oddílem (FAT32), lze ho zapsat i na USB a bootovat na reálném PC
(zatím neotestováno).

## Ladění přes GDB

```bash
cargo xtask run --gdb
```

QEMU se zastaví před startem a čeká na GDB na `localhost:1234`. V druhém
terminálu:

```bash
gdb target/x86_64-unknown-none/debug/kernel -ex "target remote localhost:1234" -ex "break _start" -ex "continue"
```

Symboly kernelu jsou ve stejném ELF souboru, který loader nahrává, adresy
tedy sedí bez přepočtu.

## Konvence

- `unsafe` jen kde je nutné, každý blok má komentář `// SAFETY:` s odůvodněním.
- Architektura-specifický kód patří do `arch/<arch>/`.
- V kódu a komentářích jen ASCII interpunkce (`-`, `->`, `...`), diakritika je OK.
- Komentáře česky nebo anglicky, ale jednotně v rámci souboru.
