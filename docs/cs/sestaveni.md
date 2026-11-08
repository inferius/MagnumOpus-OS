# Sestavení, spuštění, testy, ladění

## Nástroje

- **Rust přes rustup.** Nightly, komponenty a targety jsou v
  `rust-toolchain.toml`; rustup je stáhne při prvním buildu sám.
- **QEMU** (`qemu-system-x86_64`, pro aarch64 `qemu-system-aarch64`)
  s firmwarem edk2 (OVMF, pro aarch64 AAVMF):
  - Windows: `winget install SoftwareFreedomConservancy.QEMU` (obě
    architektury i firmware)
  - Debian/Ubuntu: `apt install qemu-system-x86 ovmf qemu-system-arm qemu-efi-aarch64`
  - Fedora: `dnf install qemu-system-x86 edk2-ovmf qemu-system-aarch64 edk2-aarch64`
  - Arch: `pacman -S qemu-desktop edk2-ovmf edk2-aarch64`
  - macOS: `brew install qemu`
- Volitelně **GDB** pro ladění.

xtask hledá QEMU v `PATH` a na obvyklých místech, firmware vedle QEMU nebo
v `/usr/share/...`. Jinak proměnné prostředí `QEMU_X86_64` (nebo `QEMU`),
`QEMU_AARCH64`, `X86_64_FW_CODE`/`X86_64_FW_VARS` (nebo `OVMF_CODE`/`OVMF_VARS`)
a `AARCH64_FW_CODE`/`AARCH64_FW_VARS`.

## Příkazy

| Příkaz | Co dělá |
|---|---|
| `cargo xtask build` | přeloží jádro, loader a programy userspace, sestaví initfs |
| `cargo xtask image` | build + obraz disku `target/os-<arch>-<profil>.img` (GPT, ESP FAT32) |
| `cargo xtask run` | image + QEMU s oknem; sériová linka v terminálu. Init testy nespouští a běží jako supervisor ([Init](init.md)). Bez `--gpu` GUI kreslí přes framebuffer firmwaru (`--gpu none`, [GUI](gui.md)), aby bylo vidět v okně. Konzoli Windows xtask přepne na UTF-8 |
| `cargo xtask test` | testy knihoven na hostiteli (`abi`, `initfs`, `fs`), kontrola překladu jádra pro ostatní architektury, dokumentace, pak jádro a userspace v QEMU bez okna |
| `cargo xtask test --only sit --repeat 20` | jen vybrané sady testů userspace (jména jako v logu, oddělená čárkou), n-krát za sebou – hledání nestabilních testů; každé další kolo přidá 60 s k limitu běhu. Test delší než 20 s se v logu označí `[pomaly]` |
| `cargo xtask test --net-online` | navíc testy proti skutečné síti hostitele (resolver hostitele přes slirp); bez volby se přeskočí |
| `cargo xtask check` | jen kontrola, že se jádro přeloží pro danou architekturu; viz [rozhraní architektury](architektura.md#rozhraní-architektury) |
| `cargo xtask docs` | HTML dokumentace z `docs/` do `target/docs` (viz níže) |

Přepínače: `--arch <x86_64|aarch64>` (všechny příkazy, výchozí `x86_64`;
aarch64 = QEMU `virt`, viz [implementace aarch64](architektura.md#implementace-aarch64-qemu-virt)),
`--release`, `--headless` (bez okna), `--iommu <typ>` (emulovaná IOMMU:
x86_64 `intel` (výchozí), `amd`, `none`; aarch64 `smmu` (výchozí), `none`; `none` ověří běh bez překladu DMA - test
„IOMMU: zarizeni nezapise mimo prideleny DMA“ a testy „IOMMU chrani pamet mimo DMA“ sady `disky` pak podle očekávání selžou),
`--data-disk <virtio|nvme|nvme4k|ahci>` (řadič datového disku, výchozí `virtio`; `nvme4k` = NVMe se sektorem 4 KiB; na NVMe a AHCI dostane disk
GPT s naším oddílem, viz [Disky](disky.md)), `--gdb` (QEMU čeká na
GDB), `--accel <tcg|kvm|whpx|hvf>`, `-- <argumenty QEMU>` (např.
`-- -smp 8`, `-- -cpu max,-x2apic`).

Souborový systém má vlastní testy na hostiteli (rychlé, bez QEMU):

```text
cargo test -p fs --features std                                   # vše, výchozí počet náhodných případů
PROPTEST_CASES=150000 cargo test -p fs --features std --release   # důkladně
cargo test -p fs --features std --release -- --ignored --nocapture perf   # výkon a režie
```

## Dokumentace jako web

`cargo xtask docs` (a také `cargo xtask test`) převede `docs/cs` a `docs/en`
na statický web v `target/docs`: navigace podle tabulky v `docs/README.md`,
obsah stránky, fulltextové hledání, přepínání jazyka, světlý a tmavý motiv,
diagramy (vložené SVG) a zvýraznění syntaxe bloků kódu s jazykem `rust`,
`c`, `cpp`, `toml` a `bash` (generuje `xtask/src/highlight.rs`, bez
skriptů v prohlížeči). Generátor kontroluje odkazy a kotvy mezi stránkami;
chybný odkaz je chyba sestavení.

Prohlížení (hledání potřebuje server, ne `file://`):

```bash
python -m http.server 8765 --directory target/docs
```

## Obsah disku

```text
EFI/BOOT/BOOTX64.EFI   UEFI loader
os/kernel.elf          jádro (ELF s ladicími symboly)
os/initfs.img          initfs: init, echo, devmgr, virtio-blk, virtio-input, inputd, ps2, xhci, usb-hub, usb-hid, fsd, timed, termd, shsess, console,
                       virtio-net, virtio-rng, netd, fwd, netprobe, manifesty, firewall.toml
                       (+ net-test u `cargo xtask test`: porty pomocných serverů)
```

QEMU dostane emulovanou IOMMU (`intel-iommu`, s `--iommu amd`
`amd-iommu,dma-remap=on`; bez překladu přerušení) a tři disky (všechny
virtio-blk v moderním režimu s `iommu_platform=on`; pod AMD-Vi systémový
disk bez něj, protože z něj firmware OVMF jinak nenabootuje). Každý režim
IOMMU má vlastní kopii NVRAM firmwaru (`target/ovmf-x86_64-vars-<režim>.fd`):
AMD IOMMU zabere slot PCI a uložené bootovací položky by neseděly.

| Disk | Soubor | Obsah |
|---|---|---|
| systémový | `target/os-x86_64-<profil>.img` | GPT + ESP výše |
| testovací | `target/scratch-<pid>.img` | 1 MiB nul; testy ovladače do něj zapisují |
| datový | `target/data-<pid>.img` | 64 MiB, náš souborový systém (jméno `data`); vytvoří ho xtask knihovnou `fs` se soubory `/vitejte.txt` a `/dokumenty/navod.md`; TRIM povolen (`discard=unmap`) |

Testovací a datový disk se vytvoří znovu při každém `run`/`test`.

Dál QEMU dostane grafiku `VGA,id=video0` (místo výchozí `-vga std`),
klávesnici a myš virtio (`virtio-keyboard-pci`, `virtio-mouse-pci`,
s `iommu_platform=on`, navázané na `display=video0`), druhou grafiku
`secondary-vga,id=video1`, řadič `qemu-xhci` s klávesnicí `usb-kbd`
(navázanou na `video1`) a server QMP na náhodném portu
localhostu (`-qmp tcp:127.0.0.1:<port>`). Přes QMP xtask vkládá události
pro test vstupu, viz [Vstup](vstup.md#testy).

Zdroj náhody `virtio-rng-pci` (data z hostitele) doplňuje entropii jádra.
Síťová karta je `virtio-net-pci` (moderní režim, `iommu_platform=on`, bez
option ROM) v user-mode síti QEMU (slirp): host má 10.0.2.15, bránu
a hostitele vidí jako 10.0.2.2, DNS slirp je 10.0.2.3. `cargo xtask test`
navíc spustí na hostiteli pomocné servery pro testy sítě (echo, DNS, NTP,
na `127.0.0.1` a volných portech) a přesměruje jeden port z hostitele do
hosta (`hostfwd`). Podrobnosti: [sit.md](sit.md#testy).

## Výstup a dohledání chyb

- Sériová linka se ukládá do `target/serial-<arch>.log`, u testů do
  `target/serial-test-<arch>-<pid>.log` (cestu xtask na konci vypíše).
- Každý běh má **vlastní** testovací a datový disk (`<pid>` = číslo procesu
  xtask) a po skončení je smaže. Souběžné běhy (víc vláken na jednom stroji)
  si tak nepřepisují disky a pomocník QMP nečte značky testů z cizího logu.
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
  src/arch/api/     smlouva architektury (traity), viz architektura.md
  src/arch/x86_64/  vše specifické pro x86_64
  src/arch/aarch64/ vše specifické pro aarch64 (QEMU virt)
  src/trap.rs       zpětný směr: architektura -> jádro
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
libs/pci/        PCI Express (ECAM, BAR, MSI-X)
libs/virtio/     virtio 1.x přes PCI (virtqueue)
libs/blockproto/ protokol blokového zařízení (klient <-> ovladač)
libs/inputproto/ protokol vstupu (ovladač -> inputd -> vlastník fokusu)
libs/keymap/     rozložení klávesnice (US, CZ), mrtvé klávesy
libs/i8042/      PS/2: scan kódy a pakety myši (bez hardwaru, testy na hostiteli)
libs/usbproto/   USB: protokol xhci <-> třídní ovladače, deskriptory, hub, HID (boot i parser reportu)
libs/usbclient/  klient kanálu USB pro třídní ovladače
libs/fs/         souborový systém (no_std; std pro nástroje)
  src/space/        volné místo: alokační skupiny, space logy
  src/fs/           soubory, adresáře, snapshoty, intent log, defrag, fsck
  src/tests/        testy FS (model, výpadky, výkon)
libs/fsproto/    protokol souborového systému (klient <-> fsd)
libs/shlang/     jazyk shellu: lexer, parsery B a C, strom, formátovač, interpret
libs/term/       chytrý terminál: protokol, služba relací, VT100, obrazovka
libs/csprng/     generátor náhodných čísel jádra (ChaCha20, BLAKE2s)
libs/wire/       společné kódování zpráv protokolů (fsproto, netproto)
libs/netproto/   protokol sítě (programy <-> netd <-> ovladač), DNS, NTP, oprávnění
libs/netfilter/  paketový filtr sítě (profily, toky UDP, omezení rychlosti)
libs/conf/       čtení a zápis konfigurace TOML
libs/manifest/   manifest aplikace (identita, rozpočet, síť)
user/init/       první proces: start služeb a testy userspace
user/echo/       testovací server pro IPC
user/devmgr/     správce zařízení (PCI, spouští ovladače)
user/virtio-blk/ ovladač disku
user/virtio-input/ ovladač klávesnice a myši
user/inputd/     služba vstupu
user/ps2/        ovladač řadiče PS/2 (klávesnice, myš)
user/xhci/       ovladač řadiče USB xHCI a jádro USB (enumerace, root hub)
user/usb-hub/    ovladač třídy hub (root hub i externí)
user/usb-hid/    ovladač klávesnice a myši USB (boot protokol)
user/fsd/        služba souborového systému
user/timed/      služba času (nastavení hodin, pásmo)
user/termd/      prostředník terminálu (směruje zprávy, registr relací)
user/shsess/     proces jedné relace shellu (interpret, koncept, undo)
user/console/    textová a záchranná konzole (framebuffer, klávesy z inputd)
user/virtio-net/ ovladač síťové karty
user/virtio-rng/ ovladač zdroje náhody (entropie pro jádro)
user/netd/       služba sítě (TCP/IP, DHCP, DNS, firewall)
user/fwd/        služba firewallu (konfigurace, dotazy uživateli)
user/netprobe/   testovací aplikace sítě s manifestem
libs/osfs-proto/ protokol rozšířeného API FS pro Windows (DeviceIoControl) + klient
tools/osfs/      nástroj pro FS na hostiteli: format, check, mount ve Windows (WinFsp)
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
