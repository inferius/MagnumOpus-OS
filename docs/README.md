# Dokumentace / Documentation

| Česky | English | Obsah / Contents |
|---|---|---|
| [architektura.md](cs/architektura.md) | [architecture.md](en/architecture.md) | návrh systému a důvody / system design and reasons |
| [sestaveni.md](cs/sestaveni.md) | [building.md](en/building.md) | nástroje, příkazy, testy, ladění, struktura / tools, commands, tests, debugging, layout |
| [jadro.md](cs/jadro.md) | [kernel.md](en/kernel.md) | jak jádro funguje / how the kernel works |
| [syscally.md](cs/syscally.md) | [syscalls.md](en/syscalls.md) | referenční popis syscallů / syscall reference |
| [chyby.md](cs/chyby.md) | [errors.md](en/errors.md) | kódy chyb a návratové kódy procesů / error and exit codes |
| [hlasky.md](cs/hlasky.md) | [messages.md](en/messages.md) | hlášky v logu a co znamenají / log messages and their meaning |
| [cas.md](cs/cas.md) | [time.md](en/time.md) | reálný čas: zdroje (UEFI, RTC), formát, pásmo / real time: sources (UEFI, RTC), format, zone |
| [vstup.md](cs/vstup.md) | [input.md](en/input.md) | vstup: virtio-input, služba inputd, protokol událostí, rozložení, bezpečná zkratka / input: virtio-input, inputd service, event protocol, layouts, secure attention shortcut |
| [usb.md](cs/usb.md) | [usb.md](en/usb.md) | USB: řadič xHCI, root hub, huby, HID, ověření nové klávesnice / USB: xHCI controller, root hub, hubs, HID, new keyboard verification |
| [libos.md](cs/libos.md) | [libos.md](en/libos.md) | knihovna pro programy / program library |
| [souborovy-system.md](cs/souborovy-system.md) | [file-system.md](en/file-system.md) | souborový systém: formát, algoritmy, služba fsd, protokol / file system: format, algorithms, fsd service, protocol |
| [osfs.md](cs/osfs.md) | [osfs.md](en/osfs.md) | nástroj osfs: obraz FS, kontrola, připojení ve Windows / osfs tool: FS image, check, mounting on Windows |
| [osfs-api.md](cs/osfs-api.md) | [osfs-api.md](en/osfs-api.md) | rozšířené API FS pro aplikace ve Windows (DeviceIoControl) / extended FS API for Windows applications (DeviceIoControl) |
| [fs-verze.md](cs/fs-verze.md) | [fs-versions.md](en/fs-versions.md) | verze formátu FS: co se změnilo a proč / FS format versions: what changed and why |
| [navrh-fs-v3.md](cs/navrh-fs-v3.md) | [fs-v3-design.md](en/fs-v3-design.md) | návrh FS v3 (odsouhlasený, zavádí se po fázích) / FS v3 design (agreed, introduced in phases) |
| [fs-v2.md](cs/fs-v2.md) | [fs-v2.md](en/fs-v2.md) | archiv: formát FS v2 / archive: FS format v2 |

Česká verze je primární; anglická ji překládá. Úplnost popisu chyb
a syscallů v obou jazycích hlídá test (`cargo test -p abi`).

The Czech version is primary; the English one translates it. A test
(`cargo test -p abi`) checks that every error and syscall is documented in
both languages.
