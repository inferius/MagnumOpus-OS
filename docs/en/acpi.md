# ACPI and power

The **`acpid`** service (`user/acpid`) runs **ACPICA** – Intel's reference
ACPI implementation – in userspace and gives the system power management:
power-off (S5), reboot, the power button event, thermal zone temperatures
(`_TZ`), battery status and discovery of what the firmware offers for CPU
frequency control (`_PSS`, CPPC). It works on x86_64 (QEMU q35, ordinary
PCs) and aarch64 (QEMU `virt` with ACPI from UEFI).

> **Status:** done: power-off, reboot, the power button (x86 fixed event,
> ARM Generic Event Device), reading temperatures and batteries, the shell
> commands `shutdown` and `reboot`. CPU frequency control is only
> **prepared** (the service discovers `_PSS`/`_CPC`, changes nothing).
> Suspend (S3) is not there.

## Why ACPICA and why in userspace

Besides static data (MADT, MCFG, HPET – the kernel reads those itself, see
[Kernel](kernel.md)) the ACPI tables contain **AML**: firmware bytecode
that has to be interpreted (methods `_PTS`, `_S5`, `_TMP`, `_BST`, `_EVT`,
...). Writing our own AML interpreter makes no sense – ACPICA is proven on
thousands of boards and is used by Linux, FreeBSD, Haiku and Zephyr. We
took it unchanged (the only edit: platform selection in `acenv.h`).

AML is firmware code with access to everything it declares. A bug in it or
in the interpreter would crash the system if it ran in the kernel; in
userspace only the service crashes. The kernel also makes sure AML cannot
touch the RAM of the kernel or of processes (see [Security](#security)).

<figure class="diagram">
<svg viewBox="0 0 780 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="ACPI service architecture">
<defs><marker id="ae1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="180" height="70" rx="6"/><text class="t b" x="100" y="45" text-anchor="middle">shell (shsess)</text><text class="m" x="100" y="63" text-anchor="middle">shutdown, reboot</text><text class="m" x="100" y="79" text-anchor="middle">channel from termd</text>
<path class="ln" d="M190 55 L268 55" marker-end="url(#ae1)"/>
<rect class="acc" x="270" y="20" width="220" height="70" rx="6"/><text class="t b" x="380" y="45" text-anchor="middle">init (supervisor)</text><text class="m" x="380" y="63" text-anchor="middle">stops services (FS before disks)</text><text class="m" x="380" y="79" text-anchor="middle">then Request::Shutdown</text>
<path class="ln" d="M380 90 L380 128" marker-end="url(#ae1)"/>
<path class="ln" d="M560 150 Q520 70 492 60" marker-end="url(#ae1)"/>
<text class="m" x="600" y="95" text-anchor="middle">Event::PowerButton</text>
<rect class="ok" x="200" y="130" width="360" height="120" rx="6"/><text class="t b" x="380" y="152" text-anchor="middle">acpid (userspace)</text>
<rect class="box" x="215" y="162" width="150" height="74" rx="4"/><text class="t" x="290" y="182" text-anchor="middle">ACPICA (C)</text><text class="m" x="290" y="200" text-anchor="middle">AML, tables,</text><text class="m" x="290" y="216" text-anchor="middle">events, S5</text>
<rect class="box" x="395" y="162" width="150" height="74" rx="4"/><text class="t" x="470" y="182" text-anchor="middle">OS layer (Rust)</text><text class="m" x="470" y="200" text-anchor="middle">AcpiOs* over libos</text><text class="m" x="470" y="216" text-anchor="middle">cstd (libc)</text>
<path class="ln" d="M365 199 L393 199" marker-end="url(#ae1)"/>
<path class="ln" d="M470 250 L470 278" marker-end="url(#ae1)"/>
<rect class="acc" x="200" y="280" width="360" height="44" rx="6"/><text class="t b" x="380" y="298" text-anchor="middle">kernel</text><text class="m" x="380" y="314" text-anchor="middle">mapping (registers, ACPI memory), ports, lines, SYSTEM_POWER</text>
<rect class="box" x="600" y="160" width="170" height="90" rx="6"/><text class="t b" x="685" y="183" text-anchor="middle">firmware</text><text class="m" x="685" y="201" text-anchor="middle">tables in ACPI memory</text><text class="m" x="685" y="217" text-anchor="middle">PM1, GPE, GED, reset</text><text class="m" x="685" y="233" text-anchor="middle">PSCI (ARM)</text>
<path class="ln" d="M560 199 L598 199" marker-end="url(#ae1)"/>
</svg>
<figcaption>The shell asks init, init stops the services cleanly and only then asks acpid; acpid reaches the firmware and hardware through ACPICA and the OS layer, only via the kernel.</figcaption>
</figure>

## Parts

| Part | Where | What it does |
|---|---|---|
| ACPICA | `third_party/acpica` | vendored sources, version **20250404** (tag `R2025_04_04`), BSD-3 licence (file `THIRD_PARTY_LICENSES.md` at the repository root) |
| platform configuration | `libs/acpica/include/acmagnum.h` | 64-bit, C library = cstd, ACPICA's own object cache, global lock |
| build and interface | `libs/acpica` | `build.rs` (via [cbuild](c-build.md)), `ffi.rs` (declarations), `ns.rs` (namespace) |
| C glue | `libs/acpica/c/glue.c` | `AcpiOsPrintf`, the FACS global lock, reading FADT and `_CRS` |
| OS layer | `user/acpid/src/osl` | all 37 `AcpiOs*` functions ACPICA calls |
| service | `user/acpid/src` | ACPICA start-up, events, power, sensors |
| protocol | `libs/powerproto` | requests, replies, events |
| init | `user/init/src/power.rs` | starting acpid, clean power-off, shell requests |

## Service start-up

Init starts `acpid` right after the time service (ACPICA initialisation
runs in parallel with the start of the other services) and hands it three
handles:

1. **A Resource with ranges for ACPI** (`MANAGE`): the whole physical space
   (the kernel allows only device registers and ACPI memory out of it, see
   below), on x86 all I/O ports and ISA lines 0-15, on aarch64 SPI lines
   32-1023; at most 16 interrupt objects, no DMA memory.
2. **A Resource with only the `POWER` right** (no `MANAGE`) for the
   `SYSTEM_POWER` syscall (power-off and reset through the firmware).
3. **ACPI data from the kernel** (`abi::acpi::Info`, bootstrap
   `kind::ACPI`): the RSDP address and a copy of it, and the list of
   firmware memory regions holding the tables.

Then `acpid` initialises ACPICA in the order from the reference manual:

```text
AcpiInitializeSubsystem   -> global state, object caches
AcpiInitializeTables      -> RSDP (from the kernel's copy), XSDT, table list
(MCFG -> ECAM windows for PCI configuration)
AcpiLoadTables            -> DSDT and SSDTs into the namespace
AcpiEnableSubsystem       -> ACPI mode (SMI_CMD), SCI handler (x86)
AcpiInitializeObjects     -> _REG, _INI
AcpiUpdateAllGpes         -> GPEs with _Lxx/_Exx methods
```

and logs e.g.:

```text
acpid: SCI na lince 9
acpid: MCFG: 1 oken ECAM
acpid: ACPI pripraveno (FADT rev 3, OEM BOCHS, S5, tlacitko, zony 0, baterie 0, CPU 4)
```

If initialisation fails, the service keeps running and answers requests
with `NOT_SUPPORTED` (init then cannot power off via ACPI; the kernel reset
remains).

## OS layer (`AcpiOs*`)

ACPICA reaches the system only through the `AcpiOs*` functions
(`acpiosxf.h`). They are in Rust with the C ABI (`user/acpid/src/osl`),
except two in C (`glue.c`).

| Group | Functions | How |
|---|---|---|
| memory | `AcpiOsAllocate`, `AcpiOsFree` | `malloc`/`free` from cstd over the program heap |
| mapping | `AcpiOsMapMemory`, `AcpiOsUnmapMemory` | page by page via `VMO_CREATE_PHYSICAL` + `VM_MAP`; the RSDP from the kernel's copy |
| registers in memory | `AcpiOsReadMemory`, `AcpiOsWriteMemory` | temporary mapping, `volatile` access of 8-64 bits |
| ports | `AcpiOsReadPort`, `AcpiOsWritePort` | `IOPORT_READ`/`IOPORT_WRITE` (an error on aarch64) |
| PCI | `AcpiOsReadPciConfiguration`, `...Write...` | ECAM windows from MCFG (a page per function, mapped once); x86 without MCFG uses ports 0xCF8/0xCFC |
| interrupts | `AcpiOsInstallInterruptHandler` | `INTERRUPT_CREATE_LINE` + a thread: wait, call the ACPICA handler, then acknowledge (a level line is unmasked once the handler cleared the status) |
| locks | `AcpiOsCreateLock`, `Acquire`, `Release` | an atomic flag; instead of spinning with interrupts off it yields the CPU |
| semaphores | `AcpiOsCreateSemaphore`, `Wait`, `Signal` | a counter under a lock + a kernel event signalled while the counter is non-zero; ACPICA builds its mutexes from them too |
| deferred work | `AcpiOsExecute`, `AcpiOsWaitEventsComplete` | two queues with their own thread: `Notify` separate from GPE work (as in Linux, otherwise deadlocks are possible) |
| time | `AcpiOsSleep`, `AcpiOsStall`, `AcpiOsGetTimer` | `time::sleep`, a short busy wait, monotonic time in 100 ns |
| threads | `AcpiOsGetThreadId` | by stack: every thread calling ACPICA registers with its stack top (libos has no TLS) |
| output | `AcpiOsPrintf`, `AcpiOsVprintf` | in C, cstd `vprintf` -> log line by line with the `acpid:` prefix |
| global lock | `ACPI_ACQUIRE_GLOBAL_LOCK` | in C, the atomic algorithm from the specification over the FACS field |
| other | table and name overrides, `AcpiOsSignal`, `AcpiOsEnterSleep` | unused (the firmware is taken as it is), AML `Fatal` only logged |

## Button events

| Platform | Source | How it is detected |
|---|---|---|
| x86 (q35, PC) | PM1 fixed event (`PWRBTN_STS`) | SCI -> ACPICA -> `AcpiInstallFixedEventHandler` |
| `PNP0C0C`/`PNP0C0E` devices | `Notify(button, 0x80)` from AML | notify handler on the namespace root |
| aarch64 (HW-reduced) | Generic Event Device `ACPI0013` | interrupt from `_CRS` (SPI) -> `_EVT(number)` -> `Notify` |

The handlers run in threads (SCI, notify queue, GED thread); they pass the
event to the service's main thread through a counter and an event. The
service sends init `Reply::Event(PowerButton)` and init powers the system
off **cleanly**.

## Power-off and reboot

**Clean power-off** (init, `power::shutdown`):

1. stop termd and the fallback console (shell sessions end by themselves),
2. stop the services in reverse registration order (clients before
   services), then `fsd`, finally the device manager (disk drivers) and the
   time service (`acpid` keeps running - it powers the computer off);
   wait at most 1.5 s for each; no more restarts,
3. ask `acpid` for `Shutdown` or `Reboot`.

**Power-off in acpid:** `AcpiEnterSleepStatePrep(5)` (methods `_PTS`,
`_SST`), then `AcpiEnterSleepState(5)` – writing the S5 sleep type and
`SLP_EN` into PM1 (x86) or into the Sleep Control register (HW-reduced with
the registers). A HW-reduced platform without sleep registers (ARM) powers
off through the kernel: PSCI `SYSTEM_OFF`.

**Reboot:** the FADT reset register (`AcpiReset`; q35: port 0xCF9 = 6),
otherwise through the kernel (PSCI `SYSTEM_RESET`, on x86 0xCF9 and the
keyboard controller).

```text
init: tlacitko napajeni, vypinam system
init: vypnuti: ukoncuji sluzby
init: sluzba fsd ukoncena
init: vypnuti: sluzby ukonceny, zadam acpid
acpid: vstupuji do S5 (vypnuti)
```

### Shell commands

```text
shutdown        # asks "Vypnout pocitac?" and powers off cleanly
shutdown -y     # without asking
reboot -y       # reboot
```

A shell session (`shsess`) gets a power channel to init from termd
(`SessMsg::Power`; termd creates one per session and sends the other end to
init). Background tasks (`async`) have no power access. Declining the
question = error `SH_REJECTED`, nothing is powered off.

## Protocol (`libs/powerproto`)

Message = `u32` code + fields (`libs/wire`, little-endian); an invalid
message = `INVALID_ARGS`.

| Request | Reply | Note |
|---|---|---|
| `Status` | `Status { features, fadt_revision, oem, thermal_zones, batteries, cpus }` | `features`: `SHUTDOWN`, `REBOOT`, `POWER_BUTTON`, `HW_REDUCED` |
| `Shutdown`, `Reboot` | none (the computer goes off) / `Failed(code)` | init sends them only after stopping the services |
| `Thermal` | zones: path, `_TMP`, `_CRT`, `_PSV` in tenths of K | 2982 = 25.0 °C |
| `Battery` | `PNP0C0A` batteries: presence, state (`_BST`), remaining, full and design capacity (`_BIX`/`_BIF`) | units mWh/mW or mAh/mA |
| `CpuPerf` | processors: number of `_PSS` states, highest frequency, `_CPC` | discovery only, no control |
| – | `Event(PowerButton | SleepButton)` | any time, unrequested |

Example (init):

```rust
let st = power::wait_ready(Duration::from_secs(60))?;
if st.features & powerproto::feature::POWER_BUTTON != 0 { /* ... */ }
match power::request(&Request::Thermal, Duration::from_secs(10))? {
    Reply::Thermal(zones) => for z in zones {
        println!("{}: {}", z.name, z.temp_dk.map(powerproto::celsius_text).unwrap_or_default());
    },
    _ => {}
}
```

## Security

AML may touch any ports, registers and PCI configuration it declares – so
`acpid` has a **broad** resource. That is the same model as Linux, where
ACPICA runs right inside the kernel; here it is additionally isolated in a
process.

| What | Protection |
|---|---|
| kernel and process RAM | `VMO_CREATE_PHYSICAL` allows only device registers (outside RAM) and ACPI memory (`AcpiReclaimable`, `AcpiNvs`) entirely inside a region; a range overlapping RAM = `ACCESS_DENIED` (test `acpi: pamet ACPI jde mapovat, RAM ne`) |
| DMA | `acpid` has no DMA memory (`dma_max = 0`) |
| power-off | `SYSTEM_POWER` only with the `POWER` right: the root resource has it, init hands it only to acpid (restricted resources never have it) |
| shell | only a session with a power channel from init (a terminal) may power off; background tasks may not |
| C code | stack canary, zeroed variables, no `sprintf`/`%n` (see [C build](c-build.md)) |

> ⚠️ **Risk:** the `acpid` process may write **any device register and
> port** (including another driver's) and ACPI firmware memory. A bug in
> or an exploit of the service = control of the hardware (not of RAM).
> Mitigation: the service reads nothing from the network or from users
> apart from 6 fixed requests from init. **Recommended narrowing** (next
> step): after loading the tables, walk the AML operation regions
> (`OperationRegion`) and create a narrower resource from them only;
> dynamically loaded tables (`Load`) would then have to ask init.

## Limitations

- **S3/S4 (suspend)** is not there: the CPU caches must be flushed before
  writing the sleep type (`wbinvd` is privileged) and state saved – that
  belongs in the kernel (a syscall). `ACPI_FLUSH_CPU_CACHE` is therefore
  empty; that does not matter for S5 and reboot.
- **SCI above line 15** (some boards have the SCI on GSI 20+): the kernel
  so far routes only ISA lines 0-15; the service then reports that it
  cannot get the SCI.
- **Embedded Controller** (`PNP0C09`, laptops: battery, temperatures, lid)
  has no driver: AML reading the EC fails and the battery is not shown.
- **CPU frequency control**: only discovery of `_PSS`/`_CPC`. Control needs
  MSR writes (x86) or CPPC registers – a kernel syscall and a policy.
- **Clean service stop:** services are terminated (`PROCESS_KILL`); `fsd`
  writes each change as a commit (CoW), so the disk is consistent, but
  there is no "finish writing and exit" protocol yet.
- Removing the SCI handler (`AcpiOsRemoveInterruptHandler`) does nothing –
  the service never shuts ACPICA down.

## Tests

- **On the host:** `cargo test -p powerproto` (message encoding), `-p cstd`
  (libc), `-p cbuild` (flags), `-p abi` (`acpi_info_roundtrip`), `-p shlang`
  (the `shutdown`/`reboot` commands with their own host).
- **In QEMU, suite `acpi`:** acpid answers and reports power-off, reboot
  and the button (x86 without HW-reduced, ARM with it), sensors and CPUs,
  ACPI memory can be mapped and RAM cannot, `SYSTEM_POWER` only with the
  `POWER` right, SPI lines in a resource.
- **Separate runs after the main test** (`xtask/src/powertest.rs`; disable
  with `--no-power`, skipped with `--only`):
  - `button`: init prints `init: test napajeni: cekam na tlacitko napajeni`,
    xtask sends QMP `system_powerdown`, init powers off cleanly and QEMU
    must exit **by itself with code 0** (not via `isa-debug-exit` or
    semihosting) and the log must contain the whole sequence
    (`acpid: udalost PowerButton`, `init: vypnuti: sluzby ukonceny`,
    `acpid: vstupuji do S5`),
  - `reboot`: reboot (QEMU with `-no-reboot` exits), log `acpid: restartuji`.

  On aarch64 the press goes through the Generic Event Device and the
  power-off through PSCI.

## Updating ACPICA

1. download a new tag from `github.com/acpica/acpica`,
2. replace `third_party/acpica/source/{include,components}` (without
   `debugger` and `disassembler`) and `LICENSE`,
3. re-add the two lines to `include/platform/acenv.h` (see
   `third_party/acpica/README.md`),
4. update the version in the README and in this document; the
   `_Static_assert`s in `glue.c` catch structure layout changes.
