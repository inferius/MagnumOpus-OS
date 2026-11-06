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
| `devmgr: ovladac pro ... nelze spustit: ...` | The driver is missing from initfs or failed to start (error code). |
| `virtio-blk BB:DD.F: N MiB, flush ano/ne, discard ano/ne` | The disk is ready. `discard ne` = the device has no TRIM (in QEMU the drive lacks `discard=unmap`); the FS then skips TRIM. |
| `virtio-blk ...: inicializace selhala: ...` | The device does not follow virtio 1.x, did not finish its reset within a second (`TIMED_OUT`) or resources are missing (MSI-X, DMA memory, `NO_RESOURCES` = the restricted Resource quota is exhausted). |
| `virtio-blk: zarizeni neodpovida spravne (...), dalsi pozadavky odmitnuty` | The device did not complete a request within 10 s (`TimedOut`) or returned an invalid queue entry (`Corrupted`). The driver resets the device and further requests return `IO_ERROR` (the device might still be writing the DMA buffer). The client (e.g. fsd) gets an I/O error. |
| `init: blokove zarizeni neodpovida: ...` | The driver did not answer `INFO`; the disk is not used. |

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
