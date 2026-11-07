# Kernel, loader and libos messages

Warnings and errors that may appear in the log, with their meaning and what
to do. The log format and levels are described in [kernel.md](kernel.md#logging).
Warnings (W) and errors (E) also carry the source file and line. The
messages themselves are in Czech; they are quoted here verbatim so they can
be searched for.

## Loader (UEFI)

The loader writes through the firmware console (screen and serial) as
`[ WARN]: loader\src\main.rs@NNN: ...`.

| Message | Meaning and fix |
|---|---|
| `initfs nenalezen (...), jadro nespusti zadny proces` | `\os\initfs.img` is missing on the ESP. The kernel boots and runs its tests but starts no process. Build the image with `cargo xtask image`. |
| `GOP neni k dispozici, framebuffer nebude` | The firmware has no graphics output. The kernel writes to serial only. |
| `GOP nelze otevrit, framebuffer nebude` | Graphics output exists but cannot be used. As above. |
| `nepodporovany format pixelu ..., framebuffer nebude` | The video mode is not 32-bit RGB/BGR. As above. |
| `ACPI 2.0 RSDP nenalezen` | The firmware did not provide ACPI 2.0. The kernel then fails to boot (`loader nepredal ACPI RSDP`). |
| `cas firmwaru: RRRR-MM-DD hh:mm:ss (pasmo neuvedeno, beru jako UTC)` | The time from UEFI `GetTime`. Firmware almost never gives the zone; when the machine keeps local time in its clock (Windows), the real time will be off by the zone. See [Real time](time.md). |
| `UEFI GetTime selhal: ...` / `UEFI vratil neplatny cas` / `UEFI cas mimo kalendar` | The firmware gave no time. The kernel tries the CMOS RTC. |
| panic `nelze nacist \os\kernel.elf` | The kernel is missing on the ESP. Rebuild the image. |
| panic `kernel neni platny ELF` | The kernel file is corrupted or in another format. |
| panic `kernel musi byt linkovany od KERNEL_BASE` | The kernel was linked without `kernel/linker.ld`. |

## Kernel: boot and platform

| Message | Level | Meaning and fix |
|---|---|---|
| `kernel: neplatny BootInfo od loaderu` | - | Loader and kernel use different `boot-info` interface versions (the kernel halts with no further output). Build both from the same version (`cargo xtask build`). |
| `loader nepredal ACPI RSDP` | E | Without ACPI the kernel knows neither CPUs nor timers and panics (`cteni ACPI selhalo`). The firmware must support ACPI 2.0. |
| `neplatny ACPI RSDP na ...` | E | The RSDP structure is corrupted. Firmware bug. |
| `korenova ACPI tabulka (XSDT/RSDT): CORRUPTED` | W | The root table has a bad checksum or length; ACPI cannot be read. |
| `ACPI tabulka na ... vynechana: CORRUPTED (-10)` | W | One table is corrupted. Unless it is the MADT or HPET, the kernel continues without it (this happens on real hardware). |
| `MADT: poskozena polozka na offsetu N, zbytek vynechan` | W | A MADT entry has an invalid length. CPUs described after it are not started. |
| `MADT: prilis kratka polozka typu T (N B), vynechana` | W | The entry is shorter than its format; it is skipped. |
| `tabulka HPET je prilis kratka, vynechana` | W | Without the HPET the kernel cannot calibrate clocks and panics (`platforma nema HPET`). |
| `chybi platna tabulka MADT ...` | E | Without the CPU list the kernel cannot boot. Firmware bug. |
| panic `cteni ACPI selhalo: ...` | panic | See the rows above. |
| panic `platforma nema HPET` | panic | The kernel currently requires an HPET to calibrate the TSC and APIC timer (QEMU q35 and modern PCs have one). |
| `casovace: ... (CPU nehlasi invariantni TSC)` | I | The CPU does not report a constant-rate TSC. Normal in QEMU with emulation (TCG); on real hardware time could drift in power-saving states. |
| `realny cas: ... UTC (zdroj UEFI\|CMOS RTC...)` | I | The real time at boot and its source. "pasmo neuvedeno - bran jako UTC" = see the loader message about the zone. |
| `realny cas neni znamy (firmware ani RTC ho nedaly)` | W | `CLOCK_REALTIME` returns `NOT_SUPPORTED`; `fsd` uses the time since boot (files will be dated 1970). On a PC this means a broken clock or a dead CMOS battery. |
| `chybi volna pamet pod 1 MiB, bezim jen na jednom CPU` | W | Starting other CPUs needs two free pages below 1 MiB. The system continues on one CPU. |
| `nedostatek pameti pro zasobnik CPU N` | E | The next CPU is not started. |
| `CPU s APIC ID N se neohlasilo, dalsi CPU nestartuji` | E | The CPU did not respond to INIT-SIPI-SIPI within ~400 ms. Its stack and startup code stay reserved (in case it comes up later); no further CPUs are started. |
| `citac cyklu CPU (APIC ID N) je posunuty o X tiku proti BSP ..., opravuji` | W | This CPU's TSC is not in sync with the first CPU. The kernel adjusts it via `IA32_TSC_ADJUST`. |
| `CPU neumi opravit citac cyklu (posun X tiku): cas na tomto CPU bude posunuty` | E | The offset is large and the CPU lacks `IA32_TSC_ADJUST`. Time on this CPU differs; wait deadlines may be inaccurate. |
| `mereni posunu citace cyklu neprobehlo, pokracuji bez nej` | W | The CPU came up late (after the timeout); its TSC was not checked. |
| `necekane preruseni, vektor N` | W | An interrupt arrived that nobody handles (device interrupts are not used yet). A single occurrence is harmless. |
| `prilis mnoho prepsani urovne logu, X ignorovan` | W | At most 16 modules can have their own log level. |

## Kernel: IOMMU

| Message | Level | Meaning and fix |
|---|---|---|
| `IOMMU VT-d na ADDRESS: segment S, N urovne tabulek, ...` | I | DMA translation is on; a device may access only its domain. |
| `IOMMU neni: zarizeni smi pres DMA zapisovat kamkoli do pameti ...` | W | The machine has neither VT-d nor AMD-Vi (no DMAR or IVRS table), or the run uses `--iommu none`. The system works, but a driver bug can corrupt memory through DMA. On a real PC enable VT-d / AMD-Vi (IOMMU) in the firmware settings. |
| `IOMMU na ADDRESS nejde zapnout: ...` | E | The unit does not support 39/48-bit tables (`NOT_SUPPORTED`), memory is missing, or it did not complete a command (`TIMED_OUT`). Devices under it are not protected. |
| `IOMMU: ... nedobehlo` | E | The unit did not complete a command or invalidation within 1 s (hardware or emulation fault). |
| `IOMMU AMD-Vi na ADDRESS: segment S, 4 urovne tabulek` | I | DMA translation through AMD-Vi is on. |
| `IOMMU AMD-Vi: prikazy nedobehly` | E | The unit did not complete the command queue within 1 s (hardware or emulation fault). |
| `IOMMU AMD-Vi: preruseni MSI nejde nastavit, ...` | W | The unit's PCI configuration has no MSI or is not in ECAM. Protection works, blocked accesses are just not reported. |
| `IOMMU: zablokovan pristup zarizeni BB:DD.F na adrese A (udalost AMD-Vi K)` | W | As for VT-d below; K = AMD-Vi event code (0x2 = page fault). |
| `IOMMU: dalsich N udalosti AMD-Vi vynechano` | W | One blocked transfer raised more events; only the first 4 are printed in detail. |
| `IOMMU: zablokovan zapis/cteni zarizeni BB:DD.F na adrese A (duvod R)` | W | A device accessed memory outside its domain: a driver bug (wrong address in a descriptor) or a faulty device. Memory stayed intact. Reason code per the VT-d specification (e.g. 0x5 = page missing in the domain, 0x6 = write to a read-only page). |

## Kernel: processes

| Message | Level | Meaning and fix |
|---|---|---|
| `loader nepredal initfs: zadny proces se nespusti` | W | See loader. |
| `initfs je poskozeny: ...` | E | The initfs image has the wrong format (`libs/initfs`). Rebuild. |
| `initfs neobsahuje program init` | E | The `init` file is missing from initfs. |
| `init neni platny program: ...` | E | `init` is not a static ELF64 for x86_64. |
| `init: poskozeny segment: ...` | E | An ELF segment lies outside the file. |
| `init se nepodarilo spustit: ...` | E | Summary with the error code ([errors.md](errors.md)). |
| `init spusten (proces N), vstup ...` | I | The first process is running. |
| `proces N (name), vlakno ...: vyjimka CPU V, kod ..., rip ..., adresa ...` | E | A program crashed (e.g. V = 14 page fault, address = what it accessed). The process ends with code -1,000,100 - V. The instruction address (`rip`) can be looked up in the program ELF (`llvm-objdump -d`). |
| `proces N (name) skoncil, kod C (description)` | I | A process ended, with the meaning of its code ([errors.md](errors.md#process-exit-codes)). |
| `init skoncil s kodem C (...)` | E | Init ended unsuccessfully; in tests this means userspace tests failed (details earlier in the log: `[SELHAL] ...`). |
| `init nebezi` | E | Init did not start (see previous errors). |
| `syscall X (N) selhal: ...` | D | A failed syscall (Debug level), for tracing program bugs. |

## Fatal errors

| Message | Meaning |
|---|---|
| `KERNEL PANIC na cpuN: panicked at file:line: message` | An unrecoverable kernel error. A stack trace follows; `cargo xtask run`/`test` translates it into function names after QEMU exits. Other CPUs are halted. |
| `KERNEL: neosetrena vyjimka NAME na cpuN, vektor V, chybovy kod ...` | A CPU exception in the kernel (kernel bug). For page faults the address and access type follow, then registers and the stack. |
| `Double Fault (#DF)` with `rsp` just below a stack | Typically a kernel stack overflow (guard page). |

Internal assertions (`expect`/`assert` in the kernel, e.g. `nedostatek pameti
pro PML4`, `neni misto pro bitmapy alokatoru`) mean a boot step the kernel
cannot run without has failed (typically too little RAM), or an internal
invariant was broken (a kernel bug).

## libos (programs)

| Message | Meaning |
|---|---|
| `libos: chybi nebo je prilis velka startovni zprava` | The process got no startup message (`abi::bootstrap`). It exits with code -1,000,001. |
| `libos: neplatna startovni zprava` | The message has the wrong magic number or version. |
| `libos: startovni zprava neobsahuje handle procesu` | `SELF_PROCESS` is missing. |
| `PANIC: ...` when out of memory | A panic is formatted into a stack buffer (max. 512 B), so it can be printed even without free memory. A panic during a panic just ends the process. |
| `libos: nedostatek pameti pro heap` | The program cannot start: the initial heap (4 MiB) cannot be created or mapped. The heap then grows on demand; when it can no longer grow (out of memory or the reserved 64 GiB range), the allocation fails and the program panics (`PANIC: ... alloc.rs ...`). |
| `PANIC: ...` | The program panicked; the process exits with code -1,000,001. |

## Drivers and device manager

| Message | Meaning and fix |
|---|---|
| `devmgr: pci BB:DD.F vvvv:dddd trida cc.ss.pp` | A PCI device was found (information). |
| `devmgr: ovladac X pro ... nelze spustit: ...` | The driver is missing from initfs or failed to start (error code). |
| `virtio-blk BB:DD.F: N MiB, flush ano/ne, discard ano/ne` | The disk is ready. `discard ne` = the device has no TRIM (in QEMU the drive lacks `discard=unmap`); the FS then skips TRIM. |
| `virtio-blk ...: inicializace selhala: ...` | The device does not follow virtio 1.x, did not finish its reset within a second (`TIMED_OUT`) or resources are missing (MSI-X, DMA memory, `NO_RESOURCES` = the restricted Resource quota is exhausted). |
| `virtio-net BB:DD.F: MAC xx:..., linka pripojena/odpojena` | The network card is ready ("link up/down"). The MAC comes from the device configuration, otherwise it is locally administered (`02:...`). |
| `virtio-net ...: inicializace selhala: ...` | As for the disk: the device does not follow virtio 1.x or resources are missing (MSI-X, DMA memory). |
| `virtio-net ...: zarizeni neodpovida spravne (...), koncim` | The device returned a nonsensical queue index or a buffer it did not own (`CORRUPTED`). The driver resets it (it stops using DMA) and exits; the network does not work until a restart. |
| `virtio-net ...: zahozeno N prijatych ramcu` | At exit: frames the network service could not receive in time (full channel queue) or invalid ones ("N received frames dropped"). |
| `virtio-rng BB:DD.F: 256 B entropie predano jadru` | The host randomness source is ready and the kernel got the first data ("256 B of entropy passed to the kernel"). More batches every minute. |
| `virtio-rng ...: zarizeni neodpovida (...), koncim` | The device did not complete a request within 5 s or returns empty data. The kernel keeps using its other sources. |
| `virtio-blk: zarizeni neodpovida spravne (...), dalsi pozadavky odmitnuty` | The device did not complete a request within 10 s (`TimedOut`) or returned an invalid queue entry (`Corrupted`). The driver resets the device and further requests return `IO_ERROR` (the device might still be writing the DMA buffer). The client (e.g. fsd) gets an I/O error. |
| `ps2: klavesnice`, `ps2: PS/2 mys ...` | The PS/2 driver found a keyboard, a mouse (kind from the ID: wheel, 5 buttons). |
| `devmgr: radic PS/2 nenalezen` | Port 0x64 reads `0xFF` (no controller) or no device was found (PS/2 controller not found). Normal on modern boards without PS/2. |
| `ps2: radic neodpovida spravne: ...` | The controller exists but failed its self-test or did not answer in time (`TIMED_OUT`). |
| `ps2: klavesnice neodpovida na reset`, `ps2: mys neodpovida (...)` | The port exists but no device answers on it (nothing connected). |
| `xhci BB:DD.F: N portu (M USB 3), S slotu, kontext K B` | The USB controller is ready (N ports, M of them USB 3, S slots, K-byte contexts). |
| `xhci: firmware radic nepustil, pokracuji` | The firmware did not release the controller within a second (USB Legacy Support); PS/2 emulation of USB may keep running. |
| `xhci ...: inicializace selhala: ...` | The controller did not finish halting, reset or start (`TIMED_OUT`), or resources are missing. |
| `xhci: port P: zarizeni vvvv:pppp (speed), slot S` / `zarizeni odpojeno` | A device on the port was set up / detached. |
| `xhci: prikaz typu T selhal (kod C)` | A controller command (Enable Slot, Address Device, Configure Endpoint ...) failed; the device is not used. |
| `usb-hub: root hub, N portu` | The hub driver runs on the root ports. |
| `usb-hub: port P: zarizeni nelze nastavit: ...` | Port reset or device setup failed (`TIMED_OUT`, `IO_ERROR`). |
| `usb-hub: externi hub [path], N portu` | The hub driver runs on an external hub. |
| `usb-hid: USB klavesnice/mys vvvv:pppp (port [...], speed)` | The HID driver introduced itself to the input service. |
| `usb-hid: rozhrani cc/ss/pp: deskriptoru reportu nerozumim` | The report descriptor is missing, broken or has nothing known, and the interface has no boot protocol. The interface is not used. |
| `usb-hid: rozhrani N nema interrupt endpoint IN` | A HID interface without an input endpoint; it is not used. |
| `devmgr: usb vvvv:pppp rozhrani cc/ss/pp: ovladac neni` | No driver for the interface class (e.g. a USB disk). |
| `devmgr: radic USB nedokoncil pruchod porty` | `usb-ready` did not arrive within 10 s; init gets `done` anyway. |
| `inputd: nove zarizeni '...' ceka na overeni`, `inputd: zarizeni '...' overeno` / `odmitnuto` | The new keyboard verification option (BadUSB): pending / trusted / rejected, see [USB](usb.md). |
| `init: supervisor bezi` | Start-up is done, init watches the services and devices ([Init](init.md)). |
| `init: sluzba X skoncila: ...` | A watched service exited (reason from the exit code). |
| `init: sluzba X znovu spustena` | The supervisor restarted the service. |
| `init: sluzba X spadla 6x za 60 s, vzdavam to` | Too many crashes; the service stays stopped. |
| `init: sluzbu X zatim neumim obnovit` | A service without restart crashed (`devmgr`, `fsd`, `inputd`); the system runs without it. |
| `init: spravce zarizeni skoncil, hotplug nefunguje` | `devmgr` closed its channel; devices attached later are not found. |
| `init: blokove zarizeni neodpovida: ...` | The driver did not answer `INFO`; the disk is not used. |
| `virtio-input BB:DD.F: 'name' (klavesnice/mys)` | An input device is ready; the kind (keyboard/mouse) comes from the supported events. `nezname zarizeni` (unknown device) = it has neither keys nor relative axes (a tablet, not supported yet). |
| `virtio-input ...: inicializace selhala: ...` | As for virtio-blk: the device does not follow virtio 1.x or resources are missing (MSI-X, DMA). |
| `virtio-input: prvni udalosti od zarizeni (N)` | The device sent events for the first time (information; confirms that interrupts and the queue work). |
| `virtio-input: sluzba nestiha, zahozeno N zprav` | The channel queue to the input service is full; the driver drops events so as not to block the device. Reported at 1, 2, 4, 8 ... messages. |
| `init: N blokovych a M vstupnich zarizeni` | What the device manager found (N block and M input devices). |

## Input service (inputd)

| Message | Meaning and fix |
|---|---|
| `inputd: bezim (rozlozeni us)` | The service is running with the default layout. |
| `inputd: zarizeni 'name' pripojeno` / `odpojeno` | A driver introduced itself (`IDEV`) or closed its channel (connected / disconnected). |
| `inputd: neplatna zprava od zdroje N` | A source sent a message that does not follow the device protocol (unknown tag, wrong length); the message is dropped. |
| `inputd: bezpecna zkratka, ale nikdo ji neposloucha` | Ctrl+Alt+Delete, but nobody holds the secure channel; the shortcut is only swallowed. |
| `inputd: bezpecna zkratka neprosla, kanal secure je plny` | The shortcut receiver is not reading; the shortcut is swallowed and secure mode does not start (input stays with the focus owner). |
| `inputd: bezpecna zkratka, ale kanal secure je zavreny` | The shortcut receiver has exited; the shortcut is swallowed. |
| `init: sluzba vstupu bezi (N zarizeni)` | Init started `inputd` and connected N drivers. |
| `init: sluzbu vstupu nelze spustit: ...` | `inputd` is missing from initfs or did not answer a command (`TIMED_OUT`). The input tests then fail (`sluzba vstupu nebezi`). |

## fsd service and file system

| Message | Meaning and fix |
|---|---|
| `init: souborovy system 'X' pripojen (generace N)` | Init found a disk with the file system and the service answers. |
| `init: zadny disk se souborovym systemem` | No disk has the signature `OSFS\0\0v2` in blocks 1-8. FS tests then fail (`sluzba fsd nebezi`). `cargo xtask run/test` always creates the data disk - check that QEMU got it. |
| `init: souborovy system nelze spustit: ...` | fsd is missing from initfs or did not answer `INFO` (see fsd messages before it). |
| `fsd: pripojeno 'X': N MiB, volno N MiB, generace N, soucty Xxh3/Blake3` | The volume is mounted. |
| `fsd: VAROVANI: nejnovejsi stav byl poskozeny, pouzit stav o N generaci starsi` | The newest commit could not be loaded (the disk lost writes although it acknowledged a flush, or it is damaged). Changes from the last N commits are lost; check the disk and run a check (`CHECK` with scrub). |
| `fsd: z intent logu prehrano N operaci` | The previous run ended without a commit (power loss); changes confirmed by `fsync` were restored. Normal after a crash. |
| `fsd: souborovy system nejde pripojit: ...` | `NOT_FOUND` = no superblock; `CORRUPTED` = none of the recent states can be loaded; `NOT_SUPPORTED` = different format version. The service exits with code 3. |
| `fsd: disk neodpovida: ...` | The disk driver did not answer (code 2). |
| `fsd: commit selhal: ...` | A disk write failed. The instance is then read-only (changes return `IO_ERROR`); the last completed commit is valid on disk. Restart the service. |
| `fsd: zaverecny commit selhal: ...` | As above, at shutdown (code 4); changes since the last commit or `fsync` are lost. |
| `fsd: defragmentace: ...` | A defragmentation step failed (typically `CORRUPTED` when reading a damaged block). Defragmentation pauses until further changes; data is not modified. |
| `fsd: chyba kanalu: ...` | Unexpected error on the client channel; the service commits and exits. |
| `fsd: chybi handly ve startovni zprave` | Wrong service start (code 1). |

## Network service netd

| Message | Meaning and fix |
|---|---|
| `netd: bezim, cekam na DHCP` | The service got `INFO` from the driver and started the DHCP client ("running, waiting for DHCP"). |
| `netd: DHCP: adresa A/P, brana G, DNS [...]` | The interface is configured (address, gateway, DNS); programs waiting in `wait_configured` continue. |
| `netd: DHCP: adresa ztracena` | The lease expired or the server removed the address ("address lost"). Connections get `UNREACHABLE` until DHCP renews it. |
| `netd: VAROVANI: generator jadra neni nasazeny, ...` | The kernel does not have enough entropy yet (no RDRAND and no virtio-rng): DNS query IDs and TCP numbers are predictable ("kernel generator not seeded"). See [Random numbers](syscalls.md#random-numbers). |
| `netd: nova relace s pravy ...` | Someone with the `RESTRICT` right (init) created a narrower session ("new session with rights"). |
| `netd: servery DNS nastaveny rucne: [...]` | `SET_DNS` (the `CONFIGURE` right); an empty list = back to the DHCP servers. |
| `netd: ovladac sitove karty neposlal INFO: ...` | The driver did not answer within 5 s (code 2); `init` then does not start the network. |
| `netd: ovladac skoncil (zahozeno ramcu: ...)` | The card driver exited (faulty device) and the service exits with it (code 3). |
| `netd: chybi handly ve startovni zprave` | The service was started incorrectly (code 1). |
| `init: sluzbu site nelze spustit: ...` | No network card (`NOT_FOUND`) or the service did not answer. |
| `ntpd: bezim, servery [...]` | Time synchronization runs; servers from `ntpd.toml` (the host server in tests). |
| `ntpd: prvni mereni (NTS\|neoverene): posun X ms, zpozdeni Y ms, timed ok:step\|ok:slew` | First successful measurement (authenticated NTS or unauthenticated NTP in fallback; "offset", "delay"); `step` = the time was set with a jump (offset over 128 ms), `slew` = smooth adjustment. |
| `ntpd: NTS-KE <server>: ...` | The NTS key exchange failed (connection, certificate, reply); the server is skipped this round. With an invalid certificate check the clock (RTC). |
| `ntpd: neoverene servery: Never\|Fallback` | Init changed the rule ("unauthenticated servers") by network profile. |
| `ntpd: zadny server neodpovedel (..., NTS: ...)` | No reply in this round (network, firewall, server). Next attempt after the interval; the clock keeps running at the last frequency. |
| `ntpd: konfigurace: ...` | The configuration from init is invalid; `ntpd` exits. Fix `ntpd.toml`. |
| `timed: prvni synchronizace: skok o N ms` | The first correction from `ntpd` was large: the time was set with a jump (the only allowed jump). |
| `timed: prvni synchronizace: dorovnani o N ms` | The first correction was small: it is applied smoothly. |
| `timed: klient synchronizace skoncil` | `ntpd` exited or closed the channel; the clock keeps running without corrections. |
| `init: ntpd.toml: ...`, `init: timed odmitl ntpd: ...`, `init: ntpd hlasi: ...` | `ntpd` did not start (invalid configuration, `timed` refused the channel, start-up error); the clock stays as set from the RTC. |
| `init: synchronizaci casu nelze spustit: ...` | Summary of the above; the system runs without synchronization. |
| `init: korenove certifikaty aktualizovany (N kB)` | The Mozilla CA set in `/system/config/certs/mozilla.pem` changed (first start, new system version). See [TLS and HTTP](tls-http.md#root-certificate-store). |
| `init: uloziste certifikatu nelze pripravit: ...` | No FS or no permissions: programs have no CA and `https` fails (`UnknownIssuer`). |
| `fetch: uloziste certifikatu: ...`, `fetch: /certs/F: ...` | `fetch` cannot read the store or a file (over 4 MiB, FS error); the other files are loaded. |
| `fetch: TLS: ...` | The TLS configuration cannot be created; `fetch` exits with code 2. |
| `revocd: bezim (OneCRL ano\|ne, N zaznamu, mezipamet na FS\|jen v pameti)` | The revocation service runs ("OneCRL yes/no, N entries, cache on the FS / in memory only"); N = OneCRL entries loaded from the FS. |
| `revocd: OneCRL: N zaznamu` | A new list of revoked CAs was downloaded and stored. |
| `revocd: OneCRL: neplatny dokument` | The downloaded JSON cannot be read ("invalid document"); the previous list stays, a new attempt after `retry_minutes`. |
| `revocd: <url>: ...` | Downloading a CRL or OneCRL failed (HTTP status, connection error); the URL is not retried for `retry_minutes`, the revocation status is unknown meanwhile. |
| `revocd: <url>: neni CRL` | The server returned something other than a CRL ("not a CRL"). |
| `netd: DNS pres TCP: spojeni se nenavazalo do 3 s, novy pokus (n)` | The TCP connection for a truncated DNS answer got stuck ("not established within 3 s, new attempt"); the resolver tries a new one (at most 3 times). |
| `netproto: sluzba site neodpovedela na pozadavek OP do N s (...)` | `netd` did not finish the request even after the limit plus margin (a bug or a stuck service). The program got `TIMED_OUT`, the channel is unusable. OP = operation code (`netproto::op`). |
| `dnsd: bezim, resolvery [...], DNSSEC ano\|ne, kotvy N, CA M` | The system resolver runs ("resolvers", "anchors"). |
| `dnsd: <resolver>: ...` | A resolver did not answer (connection, TLS, HTTP); `dnsd` tries the next one. |
| `dnsd: <name>: DNSSEC neprosel (...)` | The answer is spoofed or damaged ("DNSSEC failed", reason in brackets); the program gets `CORRUPTED`. |
| `netd: dnsd skoncil (...), preklad bez nej` | System resolution only by the profile rule ("dnsd exited"; public network = none). |
| `init: sifrovane DNS nelze spustit (...)` | `dnsd` did not start ("encrypted DNS cannot start"); resolution only by the profile rule. |
| `init: kontrolu odvolani certifikatu nelze spustit: ...` | `revocd` did not start; programs check OCSP stapling only. |
| `init: revocd bez FS (...)` | No FS: the cache is in memory only and there are no root CAs for OneCRL over https. |
| `netd: bez sluzby firewallu (verejny profil, dotazy se odmitaji)` | `init` did not pass the channel to `fwd` ("without the firewall service"). The public profile applies (nothing inbound), questions of applications with `ask` are denied right away. |
| `netd: sluzba firewallu skoncila, dotazy se odmitaji` | `fwd` exited. The last configuration applies, pending and further questions are denied. |
| `netd: profil site P` | `fwd` sent a configuration with another network profile (`public`, `home`, ...). See [Network profiles](firewall.md#network-profiles). |
| `netd: neplatna konfigurace firewallu: ...` | A `CONFIG` message from `fwd` cannot be read; the previous configuration applies. |
| `fwd: bezim (agent ano\|ne, ukladani na FS\|jen v pameti)` | The firewall service runs. "jen v pameti" (memory only) = no FS session, decisions are lost on restart. |
| `fwd: vychozi konfigurace ulozena do /config/firewall.toml` | First start: there was no configuration on the FS yet. |
| `fwd: ... je neplatna (...), pouzivam vychozi` | The file on the FS cannot be read or is invalid (line number in the description). Fix the file, `fwd` takes it on the next start. |
| `fwd: sit M (brana G), profil P (neznama sit)` | `netd` announced a network (gateway MAC). An unknown network gets `unknown_network` and the agent gets a question. |
| `fwd: A povolit\|zakazat vzdy: cil` | The user decided permanently (allow / deny always); the rule is in the configuration. |
| `fwd: dotaz N (A) vyprsel` | The agent did not answer within `ask_timeout`; `ask_default` applies. |
| `fwd: agent uzivatele skoncil, dotazy maji vychozi odpoved` | The agent channel is closed; further questions get `ask_default`. |
| `fwd: konfiguraci nejde ulozit: ...` | Writing to the FS failed; the configuration applies in memory. |
| `init: P (aplikace) v jobu: pamet ..., pamet jadra ...` | The program runs in a job with limits from its manifest. |
| `init: manifest P: ...` | The program manifest is invalid (error description); the program does not start. |
| `init: firewall bez FS (konfigurace se neulozi): ...` | An FS session for `fwd` cannot be prepared (no FS or rights). |
| `init: sluzbu firewallu nelze spustit: ...` | `fwd` did not start; `netd` sees a closed channel and denies questions. |

## Text console console

| Message | Meaning and fix |
|---|---|
| `console: bezim (ceka na displej a fokus)` | The console runs; it starts drawing after `display` and takes keys after `focus`. |
| `console: displej prevzat (SxR znaku)` | It took over the framebuffer; the character grid follows the resolution. |
| `console: displej vracen` | After `release`; the framebuffer is back with the kernel. |
| `console: ridici kanal zavren, koncim` | The owner of the console ended; the display goes back to the kernel. |
| `console: chybi kanal k termd` | Started without a channel to the session service; the console exits with code 1. |
| `framebuffer prevzal proces X N (log jadra dal na seriove lince)` | Kernel: `FB_CONTROL` `CLAIM`; it no longer draws the log on the screen. |
| `framebuffer vratil proces X N` | Kernel: `FB_CONTROL` `RELEASE`; the screen is cleared and the log is drawn again. |

## Session service termd

| Message | Meaning and fix |
|---|---|
| `termd: bezim (protokol vN, FS fsd/zadny, registr ano/ne, prevzato N)` | The broker is running. `zadny` (none) = it got no fsd channel; the shell then sees only an empty root (writes return `READ_ONLY`). `registr ne` (no registry) = sessions do not survive the end of termd. `prevzato N` = number of sessions taken over from the previous termd. |
| `termd: prevzata relace N` | A session from the registry moved to the new termd; the window can attach to it. |
| `termd\| ...` | Mirror of session output as text (VT100 adapter) so it can be read on the serial line. Blocks start with `> command`. |
| `termd: okno zavreno, koncim (N relaci bezi dal)` | The window closed; sessions detach and keep running in their processes. A new termd takes them over from the registry. |
| `termd: chyba cekani: ...` | Unexpected error waiting on channels; the broker exits with code 1 (sessions keep running). |
| `shsess N: bezim (znacka)` | The process of session N is running; the tag is in the names of its clones aside. |
| `shsess N: termd skoncil, cekam na prevzeti` | The main channel closed; the command and jobs finish, the session waits for `Adopt` from a new termd. |
| `shsess N: prevzato novym termd` | The session moved to a new termd. |
| `shsess N: nikdo neprevzal, koncim` | The registry closed the channel (or there is none) and no job waits; the session ends. |
| `shsess: chybi Init` / `prvni zprava neni Init` | The session process did not get the initial message from termd; it exits with code 1. |
