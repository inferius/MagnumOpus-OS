# Dokumentace / Documentation

Stránky jsou rozdělené do skupin; text odkazu je název v menu webu
(`cargo xtask docs`). Pages are grouped; the link text is the name in the
site menu.

### Začínáme / Getting started

| Česky | English | Obsah / Contents |
|---|---|---|
| [Sestavení a ladění](cs/sestaveni.md) | [Building and debugging](en/building.md) | nástroje, příkazy, testy, ladění, struktura / tools, commands, tests, debugging, layout |
| [Architektura](cs/architektura.md) | [Architecture](en/architecture.md) | návrh systému, milníky a důvody / system design, milestones and reasons |

### Jádro a programy / Kernel and programs

| Česky | English | Obsah / Contents |
|---|---|---|
| [Jádro](cs/jadro.md) | [Kernel](en/kernel.md) | jak jádro funguje / how the kernel works |
| [Syscally](cs/syscally.md) | [Syscalls](en/syscalls.md) | referenční popis syscallů / syscall reference |
| [libos](cs/libos.md) | [libos](en/libos.md) | knihovna pro programy / program library |
| [Reálný čas](cs/cas.md) | [Real time](en/time.md) | zdroje (UEFI, RTC), formát, pásmo / sources (UEFI, RTC), format, zone |
| [Kódy chyb](cs/chyby.md) | [Error codes](en/errors.md) | kódy chyb a návratové kódy procesů / error and exit codes |
| [Init a supervisor](cs/init.md) | [Init and supervisor](en/init.md) | start služeb, zařízení za běhu, restart spadlých služeb, testovací běh / service start-up, run-time devices, restarting crashed services, test run |
| [Hlášky v logu](cs/hlasky.md) | [Log messages](en/messages.md) | co znamenají a co dělat / what they mean and what to do |

### Spolehlivost / Reliability

| Česky | English | Obsah / Contents |
|---|---|---|
| [Obnova služeb](cs/obnova-sluzeb.md) | [Service recovery](en/service-recovery.md) | restart po pádu, registr služeb se závislostmi, lístky, knihovna svcclient, pravidla pro klienty, testy „zabij službu“ / restart after a crash, service registry with dependencies, leases, the svcclient library, rules for clients, "kill the service" tests |

### Vstup a zařízení / Input and devices

| Česky | English | Obsah / Contents |
|---|---|---|
| [Vstup](cs/vstup.md) | [Input](en/input.md) | virtio-input, PS/2, služba inputd, protokol událostí, rozložení, bezpečná zkratka / virtio-input, PS/2, inputd service, event protocol, layouts, secure attention shortcut |
| [USB](cs/usb.md) | [USB](en/usb.md) | řadič xHCI, huby, HID, ověření nového zařízení / xHCI controller, hubs, HID, new device verification |

### Grafika / Graphics

| Česky | English | Obsah / Contents |
|---|---|---|
| [Grafika a okna](cs/gui.md) | [Graphics and windows](en/gui.md) | virtio-gpu, kompozitor, protokol okna, strom vrstev a přístupnosti, snímky a transakce, knihovna UI, písmo, systémové dialogy, API akcí / virtio-gpu, compositor, window protocol, layer and accessibility trees, frames and transactions, UI library, fonts, system dialogs, action API |

### Souborový systém / File system

| Česky | English | Obsah / Contents |
|---|---|---|
| [Souborový systém](cs/souborovy-system.md) | [File system](en/file-system.md) | formát, algoritmy, oprávnění, služba fsd, protokol / format, algorithms, permissions, fsd service, protocol |
| [Historie a verze souborů](cs/fs-historie.md) | [File history](en/fs-history.md) | žurnál změn, koš, verze, commity, větve, merge / change journal, trash, versions, commits, branches, merge |
| [Data FS](cs/fs-data.md) | [FS data](en/fs-data.md) | komprese, okamžité kopie, deduplikace, předalokace, metadata 2x, scrub / compression, instant copies, deduplication, preallocation, metadata 2x, scrub |
| [Verze formátu](cs/fs-verze.md) | [Format versions](en/fs-versions.md) | co se mezi verzemi změnilo a proč / what changed between versions and why |
| [Nástroj osfs](cs/osfs.md) | [osfs tool](en/osfs.md) | obraz FS, kontrola, připojení ve Windows / FS image, check, mounting on Windows |
| [API pro Windows](cs/osfs-api.md) | [Windows API](en/osfs-api.md) | rozšířené API FS pro aplikace ve Windows / extended FS API for Windows applications |
| [Návrh FS v3](cs/navrh-fs-v3.md) | [FS v3 design](en/fs-v3-design.md) | odsouhlasený návrh, zavádí se po fázích / agreed design, introduced in phases |
| [Archiv: FS v2](cs/fs-v2.md) | [Archive: FS v2](en/fs-v2.md) | původní formát v2 / the original v2 format |

### Shell a terminál / Shell and terminal

| Česky | English | Obsah / Contents |
|---|---|---|
| [Shell a terminál](cs/navrh-shell.md) | [Shell and terminal](en/shell-design.md) | jazyk (zápisy B a C, gramatika, sémantika), undo a náhled, služba relací, protokol terminálu, kódy chyb shellu / language (notations B and C, grammar, semantics), undo and preview, session service, terminal protocol, shell error codes |

### Vzhled a ovládání / Look and feel

| Česky | English | Obsah / Contents |
|---|---|---|
| [Vzhled a ovládání](cs/vzhled.md) | [Look and feel](en/appearance.md) | design systém a tokeny, okna a odznak důvěry, lišta, systémové dialogy, správce úloh, terminál, soubory, nastavení, přihlášení, přístupnost; prototypy a příručka grafického jazyka v `design/` / design system and tokens, windows and trust badge, bar, system dialogs, task manager, terminal, files, settings, sign-in, accessibility; prototypes and visual language handbook in `design/` |

### Síť / Network

| Česky | English | Obsah / Contents |
|---|---|---|
| [Síť](cs/sit.md) | [Network](en/network.md) | virtio-net, služba netd, TCP/IP, DNS, NTP, oprávnění, API / virtio-net, netd service, TCP/IP, DNS, NTP, permissions, API |
| [Firewall](cs/firewall.md) | [Firewall](en/firewall.md) | manifest aplikace, dotazy uživateli, paketový filtr, profily sítí, audit, filtr DNS / application manifest, asking the user, packet filter, network profiles, audit, DNS filter |
| [TLS a HTTP](cs/tls-http.md) | [TLS and HTTP](en/tls-http.md) | TLS 1.3, kořenové certifikáty, HTTP/1.1 a HTTP/2, program fetch, rizika / TLS 1.3, root certificates, HTTP/1.1 and HTTP/2, the fetch program, risks |

Česká verze je primární; anglická ji překládá. Úplnost popisu chyb
a syscallů v obou jazycích hlídá test (`cargo test -p abi`).

The Czech version is primary; the English one translates it. A test
(`cargo test -p abi`) checks that every error and syscall is documented in
both languages.
