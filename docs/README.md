# Dokumentace / Documentation

Stránky jsou rozdělené do skupin; text odkazu je název v menu webu
(`cargo xtask docs`). Pages are grouped; the link text is the name in the
site menu.

### Začínáme / Getting started

| Česky | English | Obsah / Contents |
|---|---|---|
| [Sestavení a ladění](cs/sestaveni.md) | [Building and debugging](en/building.md) | nástroje, příkazy, testy, ladění, struktura / tools, commands, tests, debugging, layout |
| [Architektura](cs/architektura.md) | [Architecture](en/architecture.md) | návrh systému, milníky a důvody / system design, milestones and reasons |
| [Vlastní, nebo převzaté](cs/vlastni-prevzate.md) | [Own or adopted](en/own-or-adopted.md) | co psát sami a co převzít hotové, hraniční případy, jak převzít / what to write ourselves and what to adopt, borderline cases, how to adopt |
| [Psaní dokumentace](cs/dokumentace.md) | [Writing documentation](en/documentation.md) | značky stavu, jeden zdroj pravdy, tvrzení a testy, měření s podmínkami / status marks, one source of truth, claims and tests, measurements with conditions |
| [Instalace](cs/instalace.md) | [Installation](en/installation.md) | instalace z živého USB na interní disk: příkaz install, celý disk / volné místo / přeinstalace, ochrana cizích oddílů, NVRAM, system.toml, test / installing from the live USB to an internal disk: the install command, whole disk / free space / reinstall, foreign partition protection, NVRAM, system.toml, test |
| [Živé USB](cs/zive-usb.md) | [Live USB](en/live-usb.md) | zkoušení na skutečném PC: obraz, zápis na flashku (xtask, Rufus, dd), start, rizika, logy, záznamy pádů, příkaz report, co dělat při problému / trying it on a real PC: image, writing the stick (xtask, Rufus, dd), boot, risks, logs, crash records, the report command, what to do when something goes wrong |

### Jádro a programy / Kernel and programs

| Česky | English | Obsah / Contents |
|---|---|---|
| [Jádro](cs/jadro.md) | [Kernel](en/kernel.md) | jak jádro funguje / how the kernel works |
| [Výjimky a kontext vlákna](cs/vyjimky.md) | [Exceptions and thread context](en/exceptions.md) | pozastavení vlákna, čtení a zápis registrů, kanály výjimek, doručení ve vlákně (SEH, signály, debugger) / suspending a thread, reading and writing registers, exception channels, in-thread delivery (SEH, signals, debugger) |
| [Správa paměti](cs/sprava-pameti.md) | [Memory management](en/memory-management.md) | paměť procesů: stránky na požádání, nulová stránka, commit a strop, piny; plán rezervací, CoW a JIT / process memory: demand paging, the zero page, commit and ceiling, pins; plan for reservations, CoW and JIT |
| [Plánovač](cs/planovac.md) | [Scheduler](en/scheduler.md) | třídy QoS, spravedlivé dělení, hybridní CPU / QoS classes, fair sharing, hybrid CPUs |
| [Syscally](cs/syscally.md) | [Syscalls](en/syscalls.md) | referenční popis syscallů / syscall reference |
| [libos](cs/libos.md) | [libos](en/libos.md) | knihovna pro programy / program library |
| [Reálný čas](cs/cas.md) | [Real time](en/time.md) | zdroje (UEFI, RTC), formát, pásmo / sources (UEFI, RTC), format, zone |
| [Kódy chyb](cs/chyby.md) | [Error codes](en/errors.md) | kódy chyb a návratové kódy procesů / error and exit codes |
| [Init a supervisor](cs/init.md) | [Init and supervisor](en/init.md) | start služeb, zařízení za běhu, restart spadlých služeb, testovací běh / service start-up, run-time devices, restarting crashed services, test run |
| [Hlášky v logu](cs/hlasky.md) | [Log messages](en/messages.md) | co znamenají a co dělat / what they mean and what to do |
| [Rust std](cs/rust-std.md) | [Rust std](en/rust-std.md) | návrh std pro aplikace: cíle magnumopus, osrt, sestavení sysrootu, mapování na libos, co chybí v jádře / std design for applications: magnumopus targets, osrt, sysroot build, mapping onto libos, what the kernel lacks |
| [Překlad C](cs/preklad-c.md) | [Building C code](en/c-build.md) | clang z build.rs (cbuild), knihovna C cstd, příznaky a ochrany, převzatý kód / clang from build.rs (cbuild), the cstd C library, flags and protections, vendored code |

### Spolehlivost / Reliability

| Česky | English | Obsah / Contents |
|---|---|---|
| [Obnova služeb](cs/obnova-sluzeb.md) | [Service recovery](en/service-recovery.md) | restart po pádu, registr služeb se závislostmi, lístky, knihovna svcclient, pravidla pro klienty, testy „zabij službu“ / restart after a crash, service registry with dependencies, leases, the svcclient library, rules for clients, "kill the service" tests |

### Uživatelé a zabezpečení / Users and security

| Česky | English | Obsah / Contents |
|---|---|---|
| [Uživatelské účty](cs/ucty.md) | [User accounts](en/accounts.md) | služba accountd, heslo Argon2id, PIN, bezpečnostní klíč FIDO2, relace, správa počítače bez roota, šifrování domova / accountd service, Argon2id password, PIN, FIDO2 security key, sessions, administration without root, home encryption |
| [Správa organizace](cs/sprava-organizace.md) | [Organization management](en/organization-management.md) | zápis počítače do správy organizace, firemní záloha klíče domova (escrow), sledování koncových bodů (EDR), rizika, pořadí **[navrženo]** / enrolling a computer, company home-key backup (escrow), endpoint monitoring (EDR), risks, order **[designed]** |

### Vstup a zařízení / Input and devices

| Česky | English | Obsah / Contents |
|---|---|---|
| [Vstup](cs/vstup.md) | [Input](en/input.md) | virtio-input, PS/2, služba inputd, protokol událostí, rozložení, bezpečná zkratka / virtio-input, PS/2, inputd service, event protocol, layouts, secure attention shortcut |
| [Disky](cs/disky.md) | [Disks](en/disks.md) | NVMe, AHCI (SATA), virtio-blk, ochrana cizích disků a GPT, VMware a skutečné PC / NVMe, AHCI (SATA), virtio-blk, foreign disk protection and GPT, VMware and real PC |
| [USB](cs/usb.md) | [USB](en/usb.md) | řadič xHCI, huby, HID, disky USB, ověření nového zařízení / xHCI controller, hubs, HID, USB disks, new device verification |
| [ACPI a napájení](cs/acpi.md) | [ACPI and power](en/acpi.md) | ACPICA v userspace, vypnutí S5, restart, tlačítko napájení, teploty, baterie, příkazy shutdown/reboot / ACPICA in userspace, S5 power-off, reboot, power button, temperatures, batteries, shutdown/reboot commands |

### Ovladače / Drivers

| Česky | English | Obsah / Contents |
|---|---|---|
| [Ovladače](cs/ovladace.md) | [Drivers](en/drivers.md) | příručka autora ovladače: architektura, bezpečnost, sada driverkit, první ovladač krok za krokem, manifest, simulace hardwaru, zdroje a licence, Rust nebo převzaté C, ladění, kontrolní seznam / driver author's guide: architecture, security, the driverkit, first driver step by step, manifest, hardware simulation, sources and licences, Rust or adopted C, debugging, checklist |
| [Ovladač edu](cs/ovladac-edu.md) | [Driver edu](en/driver-edu.md) | ukázkový ovladač pro zařízení QEMU edu / sample driver for the QEMU edu device |
| [Jednotná rozhraní](cs/rozhrani-zarizeni.md) | [Unified interfaces](en/device-interfaces.md) | návrh: model ovladačů podle tříd, displej bez kopie, systémové služby pro aplikace (kamera, hovory, poloha...) / design: class-based driver model, display without a copy, system services for apps (camera, calls, location...) |

### Zvuk / Audio

| Česky | English | Obsah / Contents |
|---|---|---|
| [Zvuk](cs/zvuk.md) | [Audio](en/audio.md) | ovladač Intel HDA, kodeky, cesty k výstupům a vstupu, HDMI, služba audiod, protokol se sdílenými buffery, mix, nahrávání a převod frekvence, latence, obnova po pádu, příkaz `audio` / Intel HDA driver, codecs, output and input paths, HDMI, audiod service, shared-buffer protocol, mixing, recording and rate conversion, latency, crash recovery, the `audio` command |

### Grafika / Graphics

| Česky | English | Obsah / Contents |
|---|---|---|
| [Grafika a okna](cs/gui.md) | [Graphics and windows](en/gui.md) | virtio-gpu, kompozitor, protokol okna, strom vrstev a přístupnosti, snímky a transakce, knihovna UI, písmo, systémové dialogy, API akcí / virtio-gpu, compositor, window protocol, layer and accessibility trees, frames and transactions, UI library, fonts, system dialogs, action API |
| [Správce souborů](cs/spravce-souboru.md) | [File manager](en/file-manager.md) | aplikace Soubory: místa, výpis s řazením a hledáním, náhled, tři stavy hodnot, koš, verze, kopírování jako úloha s obnovou, asociace typů a služba „otevři cestu“ / the Files app: places, listing with sorting and search, preview, three value states, trash, versions, copying as a recoverable task, type associations and the "open a path" service |
| [Vulkan přes Venus](cs/venus.md) | [Vulkan over Venus](en/venus.md) | vlastní ovladač Venus, render uzel, kódování proudu, generátor z vk.xml / own Venus driver, render node, stream encoding, generator from vk.xml |
| [Startovní obrazovka](cs/start.md) | [Boot screen](en/boot-screen.md) | obraz od loaderu po plochu, prstenec průběhu, přechod do kompozitoru, ladicí start, chyby startu, výkon / picture from the loader to the desktop, progress ring, hand-over to the compositor, debug boot, boot failures, performance |
| [Nastavení obrazu](cs/displej.md) | [Display configuration](en/display.md) | EDID a DisplayID, režimy a časování, DisplayPort a HDMI, protokol displeje, změna rozlišení, hotplug, page flip, bochs-display, Intel Gen9 až Xe2, bezpečný návrat na obraz firmwaru / EDID and DisplayID, modes and timings, DisplayPort and HDMI, display protocol, resolution changes, hotplug, page flips, bochs-display, Intel Gen9 to Xe2, safe return to the firmware image |

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
| [Příkazy shellu](cs/prikazy-shellu.md) | [Shell commands](en/shell-commands.md) | referenční nápověda všech příkazů (generováno z metadat, totéž co `help <příkaz>`) / reference of all commands (generated from metadata, same as `help <command>`) |
| [Shell a terminál](cs/navrh-shell.md) | [Shell and terminal](en/shell-design.md) | jazyk (zápisy B a C, gramatika, sémantika), undo a náhled, služba relací, protokol terminálu, kódy chyb shellu / language (notations B and C, grammar, semantics), undo and preview, session service, terminal protocol, shell error codes |

### Vzhled a ovládání / Look and feel

| Česky | English | Obsah / Contents |
|---|---|---|
| [Vzhled a ovládání](cs/vzhled.md) | [Look and feel](en/appearance.md) | design systém a tokeny, okna a odznak důvěry, lišta, systémové dialogy, správce úloh, terminál, soubory, nastavení, přihlášení, přístupnost; prototypy a příručka grafického jazyka v `design/` / design system and tokens, windows and trust badge, bar, system dialogs, task manager, terminal, files, settings, sign-in, accessibility; prototypes and visual language handbook in `design/` |

### Síť / Network

| Česky | English | Obsah / Contents |
|---|---|---|
| [Síť](cs/sit.md) | [Network](en/network.md) | virtio-net, služba netd, TCP/IP, DNS, NTP, oprávnění, API / virtio-net, netd service, TCP/IP, DNS, NTP, permissions, API |
| [Síťové karty](cs/sitove-karty.md) | [Network cards](en/network-cards.md) | ovladače Intel e1000e a igc, Realtek r8169, společná smyčka, DMA a přerušení, testy nad simulovanými registry, co je ověřené / Intel e1000e and igc, Realtek r8169 drivers, shared loop, DMA and interrupts, tests on simulated registers, what is verified |
| [Firewall](cs/firewall.md) | [Firewall](en/firewall.md) | manifest aplikace, dotazy uživateli, paketový filtr, profily sítí, audit, filtr DNS / application manifest, asking the user, packet filter, network profiles, audit, DNS filter |
| [TLS a HTTP](cs/tls-http.md) | [TLS and HTTP](en/tls-http.md) | TLS 1.3, kořenové certifikáty, HTTP/1.1 a HTTP/2, program fetch, rizika / TLS 1.3, root certificates, HTTP/1.1 and HTTP/2, the fetch program, risks |

### Agent AI / AI agent

| Česky | English | Obsah / Contents |
|---|---|---|
| [Agent AI v systému](cs/agent.md) | [AI agent in the system](en/agent.md) | rozhraní modelu, nástroje služeb, práva a pracovní prostor, pomocník v shellu, diagnostika, bezpečnost a limity útraty / model interface, service tools, rights and workspace, shell helper, diagnostics, security and spending limits |

Česká verze je primární; anglická ji překládá. Úplnost popisu chyb
a syscallů v obou jazycích hlídá test (`cargo test -p abi`).

The Czech version is primary; the English one translates it. A test
(`cargo test -p abi`) checks that every error and syscall is documented in
both languages.
