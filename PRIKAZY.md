# Taháky: sestavení a spuštění

Rychlý přehled příkazů (Windows, PowerShell nebo Git Bash, ve složce
repozitáře). Podrobnosti: `cargo xtask help`,
[docs/cs/sestaveni.md](docs/cs/sestaveni.md),
[docs/cs/zive-usb.md](docs/cs/zive-usb.md).

Výchozí architektura je `x86_64`, profil `debug`. `--release` = rychlejší
systém (doporučeno na zkoušení), `--arch aarch64` = ARM.

## QEMU

| Co | Příkaz |
|---|---|
| běžné spuštění s oknem (rychlé, WHPX, Full HD) | `cargo xtask run --release --accel whpx --res 1920x1080` |
| když WHPX spadne na IOMMU | `cargo xtask run --release --accel whpx --iommu none` |
| bez akcelerace (pomalé, ale vždy funguje) | `cargo xtask run --release` |
| 4K | `cargo xtask run --release --accel whpx --res 3840x2160` |
| bez okna, jen výpis do terminálu | `cargo xtask run --release --headless` |
| grafika virtio-gpu 2D (pak v okně View -> virtio-gpu-pci) | `cargo xtask run --release --gpu 2d` |
| Vulkan (Venus) ve WSL | `cargo xtask run --release --qemu-wsl --gpu venus` |
| disk NVMe / AHCI / USB místo virtio | `cargo xtask run --release --data-disk nvme` (`nvme4k`, `ahci`, `usb`) |
| se zvukem (Windows) | `cargo xtask run --release --audio dsound` |
| ARM (aarch64) | `cargo xtask run --release --arch aarch64` |
| čekat na ladicí GDB | `cargo xtask run --gdb` |
| krokovat jádro bez optimalizace (debug jádro je jinak `opt-level` 2) | `cargo xtask run --gdb --kernel-opt 0` |
| vlastní argumenty QEMU | `cargo xtask run --release -- -smp 8 -m 2G` |

Restart v okně QEMU s WHPX se zasekne (chyba QEMU/WHPX) - okno zavřít
a spustit znovu.

## VMware

| Co | Příkaz |
|---|---|
| sestavit a nahradit disk VM (VM musí být vypnutý) | `cargo xtask vmware --release --dir F:\Virtual\MOOS` |
| se složkou v proměnné | `$env:MO_VMWARE_DIR="F:\Virtual\MOOS"; cargo xtask vmware --release` |

Původní disk se zazálohuje (drží se 3 zálohy). Log: `F:\Virtual\MOOS\__serial_log`.
VM: firmware EFI, Secure Boot vypnutý, Guest OS „Windows 10/11 x64“
(dá síť e1000e, zvuk HD Audio, USB 3).

## Flashka (živé USB, skutečný počítač)

| Co | Příkaz |
|---|---|
| jen obraz `target/usb-x86_64-release.img` | `cargo xtask usb --release` |
| obraz jiné velikosti / jinam | `cargo xtask usb --release --size 2048 --out D:\mo.img` |
| seznam disků (najít flashku) | `cargo xtask usb --list` |
| zapsat na flashku (s potvrzením) | `cargo xtask usb --release --write <disk z --list>` |
| vyzkoušet obraz v QEMU jako flashku | `cargo xtask usb --release --run` |

Zapsat jde i Rufusem (režim DD) nebo `dd`. Logy a záznamy pádů jsou po
startu na oddílu MO-LOGY (ve Windows se ukáže jako disk), v systému
příkaz `report`.

## Obrazy a ostatní

| Co | Příkaz |
|---|---|
| jen sestavit | `cargo xtask build --release` |
| obraz disku pro QEMU (bez spuštění) | `cargo xtask image --release` |
| všechny testy (x86_64) | `cargo xtask test` |
| testy ARM | `cargo xtask test --arch aarch64` |
| jen vybrané sady testů | `cargo xtask test --only gui,sit` |
| jen start v release | `cargo xtask test --only release` |
| Wine na datovém disku (`/apps/org.winehq.wine`) a jeho test | `cargo xtask wine`, pak `cargo xtask test --only wine --wine` |
| složka hostitele jako flash disk v `/media` (jen čtení; `--share-rw` se zápisem, experimentální) | `cargo xtask run --share C:\data` |
| dokumentace (HTML do target/docs) | `cargo xtask docs` |
| přeložit adresy v záznamu pádu | `cargo xtask symbolize <soubor>` |
| kostra nového ovladače | `cargo xtask new-driver muj --pci 1234:5678` |

Když běží `cargo xtask run` a pustíš jiný `cargo xtask`, nepřeloží se
(`xtask.exe` je zamčený) - nejdřív zavři QEMU.
