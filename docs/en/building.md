# Building, running, testing, debugging

## Tools

- **Rust via rustup.** The nightly, components and targets are pinned in
  `rust-toolchain.toml`; rustup installs them on the first build.
- **QEMU** (`qemu-system-x86_64`, for aarch64 `qemu-system-aarch64`) with
  edk2 firmware (OVMF, for aarch64 AAVMF):
  - Windows: `winget install SoftwareFreedomConservancy.QEMU` (both
    architectures and firmware)
  - Debian/Ubuntu: `apt install qemu-system-x86 ovmf qemu-system-arm qemu-efi-aarch64`
  - Fedora: `dnf install qemu-system-x86 edk2-ovmf qemu-system-aarch64 edk2-aarch64`
  - Arch: `pacman -S qemu-desktop edk2-ovmf edk2-aarch64`
  - macOS: `brew install qemu`
- **clang and llvm-ar** (LLVM 17+) for vendored C code (ACPICA, see
  [Building C code](c-build.md)): Windows `winget install LLVM.LLVM`,
  Debian/Ubuntu `apt install clang llvm`, Fedora `dnf install clang llvm`,
  Arch `pacman -S clang llvm`, macOS `brew install llvm`. Elsewhere: the
  `CLANG` variable.
- Optionally **GDB** for debugging.

xtask looks for QEMU in `PATH` and common install locations, and for the
firmware next to QEMU or in `/usr/share/...`. Otherwise set `QEMU_X86_64`
(or `QEMU`), `QEMU_AARCH64`, `X86_64_FW_CODE`/`X86_64_FW_VARS` (or
`OVMF_CODE`/`OVMF_VARS`) and `AARCH64_FW_CODE`/`AARCH64_FW_VARS`.

## Commands

| Command | What it does |
|---|---|
| `cargo xtask build` | builds the kernel, loader and userspace programs, creates initfs |
| `cargo xtask image` | build + disk image `target/os-<arch>-<profile>.img` (GPT, FAT32 ESP) |
| `cargo xtask run` | image + QEMU with a window; serial output in the terminal. Init does not run the tests and runs as the supervisor ([Init](init.md)). Without `--gpu` the GUI draws through the firmware framebuffer (`--gpu none`, [GUI](gui.md)) so that it shows in the window. xtask switches the Windows console to UTF-8 |
| `cargo xtask test` | host library tests (`abi`, `initfs`, `fs`), kernel build check for the other architectures, documentation, then kernel and userspace in QEMU without a window |
| `cargo xtask test --only sit --repeat 20` | only the selected userspace test suites (names as in the log, comma separated), n times in a row - for finding flaky tests; every extra round adds 60 s to the run limit. A test longer than 20 s is marked `[pomaly]` ("slow") in the log |
| `cargo xtask test --only napajeni` | only the separate power test runs (power-off with the button through ACPI, reboot; they always run after the main run, `--no-power` disables them), see [ACPI](acpi.md#tests) |
| `cargo xtask test --net-online` | also the tests against the real host network (host resolver through slirp); skipped without the switch |
| `cargo xtask test --driver edu` | only one manifest driver: its host tests (simulation) and in QEMU the `ovladace z manifestu` suite for it only; without checking the other architectures' kernels and the documentation (a fast loop, see [Drivers](drivers.md)) |
| `cargo xtask new-driver <name> --pci <vendor:device>` | a driverkit driver skeleton: code, manifest, device model and simulation test, documentation page cs+en ([Drivers](drivers.md#the-first-driver-step-by-step)) |
| `cargo xtask check` | only checks that the kernel builds for the given architecture; see [architecture interface](architecture.md#architecture-interface) |
| `cargo xtask docs` | HTML documentation from `docs/` into `target/docs` (see below) |

Options: `--arch <x86_64|aarch64>` (all commands, default `x86_64`; aarch64
= QEMU `virt`, see [the aarch64 implementation](architecture.md#the-aarch64-implementation-qemu-virt)),
`--release`, `--headless` (no window), `--iommu <type>` (emulated IOMMU:
x86_64 `intel` (default), `amd`, `none`; aarch64 `smmu` (default), `none`; `none` checks running without DMA
translation - the test "IOMMU: zarizeni nezapise mimo prideleny DMA" and the
"IOMMU chrani pamet mimo DMA" tests of the `disky` suite then fail as expected),
`--data-disk <virtio|nvme|nvme4k|ahci>` (data disk controller, default `virtio`; `nvme4k` = NVMe with 4 KiB sectors; on NVMe
and AHCI the disk gets a GPT with our partition, see [Disks](disks.md)),
`--gdb` (QEMU waits for GDB), `--accel <tcg|kvm|whpx|hvf>`, `-- <QEMU args>` (e.g. `-- -smp 8`,
`-- -cpu max,-x2apic`).

The file system has its own host tests (fast, no QEMU):

```text
cargo test -p fs --features std                                   # everything, default number of random cases
PROPTEST_CASES=150000 cargo test -p fs --features std --release   # thorough
cargo test -p fs --features std --release -- --ignored --nocapture perf   # performance and overhead
```

## Documentation as a website

`cargo xtask docs` (and also `cargo xtask test`) converts `docs/cs` and
`docs/en` into a static website in `target/docs`: navigation according to
the table in `docs/README.md`, page contents, full-text search, language
switch, light and dark themes, diagrams (inline SVG) and syntax
highlighting of code blocks tagged `rust`, `c`, `cpp`, `toml` and `bash`
(generated by `xtask/src/highlight.rs`, no scripts in the browser). The
generator checks links and anchors between pages; a broken link is a build
error.

Viewing (search needs a server, not `file://`):

```bash
python -m http.server 8765 --directory target/docs
```

## Disk contents

```text
EFI/BOOT/BOOTX64.EFI   UEFI loader
os/kernel.elf          kernel (ELF with debug symbols)
os/initfs.img          initfs: init, echo, devmgr, virtio-blk, virtio-input, inputd, ps2, xhci, usb-hub, usb-hid, fsd, timed, termd, shsess, console,
                       virtio-net, virtio-rng, netd, fwd, netprobe, manifests, firewall.toml
                       (+ net-test with `cargo xtask test`: helper server ports)
```

QEMU gets an emulated IOMMU (`intel-iommu`, with `--iommu amd`
`amd-iommu,dma-remap=on`; without interrupt remapping) and three disks (all
virtio-blk in modern mode with `iommu_platform=on`; under AMD-Vi the system
disk without it, because the OVMF firmware cannot boot from it otherwise).
Each IOMMU mode has its own copy of the firmware NVRAM
(`target/ovmf-x86_64-vars-<mode>.fd`): the AMD IOMMU takes a PCI slot and
stored boot entries would not match.

| Disk | File | Contents |
|---|---|---|
| system | `target/os-x86_64-<profile>.img` | GPT + the ESP above |
| scratch | `target/scratch-<pid>.img` | 1 MiB of zeros; driver tests write to it |
| data | `target/data-<pid>.img` | 64 MiB, our file system (label `data`); created by xtask with the `fs` library, containing `/vitejte.txt` and `/dokumenty/navod.md`; TRIM enabled (`discard=unmap`) |

The scratch and data disks are recreated on every `run`/`test`.

QEMU also gets `VGA,id=video0` graphics (instead of the default
`-vga std`), a virtio keyboard and mouse (`virtio-keyboard-pci`,
`virtio-mouse-pci`, with `iommu_platform=on`, bound to `display=video0`), a second graphics
card `secondary-vga,id=video1`, a `qemu-xhci` controller with a `usb-kbd`
keyboard (bound to `video1`) and a QMP server on a random
localhost port (`-qmp tcp:127.0.0.1:<port>`). Xtask injects events for the
input test over QMP, see [Input](input.md#tests).

The `virtio-rng-pci` randomness source (data from the host) adds entropy to
the kernel. The network card is `virtio-net-pci` (modern mode, `iommu_platform=on`, no
option ROM) in QEMU user-mode networking (slirp): the guest has 10.0.2.15,
sees the gateway and the host as 10.0.2.2, and the slirp DNS is 10.0.2.3.
`cargo xtask test` also starts helper servers on the host for the network
tests (echo, DNS, NTP, on `127.0.0.1` and free ports) and forwards one
port from the host into the guest (`hostfwd`). Details:
[network.md](network.md#tests).

## Output and tracking down errors

- Serial output is saved to `target/serial-<arch>.log`, for tests to
  `target/serial-test-<arch>-<pid>.log` (xtask prints the path at the end).
- Every run has its **own** scratch and data disk (`<pid>` = the xtask
  process id) and deletes them when it ends. Concurrent runs (several
  threads on one machine) therefore do not overwrite each other's disks, and
  the QMP helper does not read test markers from someone else's log.
- On a panic or fatal exception, xtask translates the stack trace addresses
  into function names after QEMU exits (`llvm-nm` from `llvm-tools`).
- Kernel log levels: [kernel.md](kernel.md#logging); messages: [messages.md](messages.md).

## Debugging with GDB

```bash
cargo xtask run --gdb
gdb target/x86_64-unknown-none/debug/kernel -ex "target remote localhost:1234" -ex "break _start" -ex "continue"
```

Userspace programs have their own ELF files in
`target/user/x86_64-unknown-none/<profile>/`.

## Repository layout

```text
loader/          UEFI loader
kernel/          kernel
  src/arch/api/     architecture contract (traits), see architecture.md
  src/arch/x86_64/  everything x86_64-specific
  src/arch/aarch64/ everything aarch64-specific (QEMU virt)
  src/trap.rs       reverse direction: architecture -> kernel
  src/mm/           memory
  src/sched/        scheduler
  src/object/       objects and handles
  src/syscall/      syscalls
  src/log/          logging
  src/tests/        kernel self-tests
libs/abi/        kernel <-> userspace interface (errors, syscalls, rights)
libs/boot-info/  loader <-> kernel interface
libs/elf/        ELF parser
libs/initfs/     initfs format
libs/libos/      library for programs
libs/pci/        PCI Express (ECAM, BAR, MSI-X)
libs/virtio/     virtio 1.x over PCI (virtqueue)
libs/blockproto/ block device protocol (client <-> driver)
libs/inputproto/ input protocol (driver -> inputd -> focus owner)
libs/keymap/     keyboard layouts (US, CZ), dead keys
libs/i8042/      PS/2: scan codes and mouse packets (no hardware, host tests)
libs/usbproto/   USB: xhci <-> class driver protocol, descriptors, hub, HID (boot and report parser)
libs/usbclient/  USB channel client for class drivers
libs/fs/         file system (no_std; std for tools)
  src/space/        free space: allocation groups, space logs
  src/fs/           files, directories, snapshots, intent log, defrag, fsck
  src/tests/        FS tests (model, power loss, performance)
libs/fsproto/    file system protocol (client <-> fsd)
libs/shlang/     shell language: lexer, B and C parsers, tree, formatter, interpreter
libs/term/       smart terminal: protocol, session service, VT100, screen
libs/csprng/     kernel random number generator (ChaCha20, BLAKE2s)
libs/wire/       shared message encoding of protocols (fsproto, netproto)
libs/netproto/   network protocol (programs <-> netd <-> driver), DNS, NTP, permissions
libs/netfilter/  network packet filter (profiles, UDP flows, rate limits)
libs/conf/       reading and writing TOML configuration
libs/manifest/   application manifest (identity, budget, network)
user/init/       first process: starts services, userspace tests
user/echo/       IPC test server
user/devmgr/     device manager (PCI, starts drivers)
user/virtio-blk/ disk driver
user/virtio-input/ keyboard and mouse driver
user/inputd/     input service
user/ps2/        PS/2 controller driver (keyboard, mouse)
user/xhci/       xHCI USB controller driver and USB core (enumeration, root hub)
user/usb-hub/    hub class driver (root hub and external)
user/usb-hid/    USB keyboard and mouse driver (boot protocol)
user/fsd/        file system service
user/timed/      time service (clock setting, time zone)
user/termd/      terminal broker (routes messages, session registry)
user/shsess/     process of one shell session (interpreter, draft, undo)
user/console/    text and rescue console (framebuffer, keys from inputd)
user/virtio-net/ network card driver
user/virtio-rng/ randomness source driver (entropy for the kernel)
user/netd/       network service (TCP/IP, DHCP, DNS, firewall)
user/fwd/        firewall service (configuration, asking the user)
user/netprobe/   network test application with a manifest
libs/osfs-proto/ protocol of the extended FS API for Windows (DeviceIoControl) + client
tools/osfs/      host tool for the FS: format, check, mount on Windows (WinFsp)
xtask/           build, disk image, QEMU, tests
docs/            documentation (cs, en)
```

## Code conventions

- Files max ~1500 lines; split into submodules as soon as it makes sense.
- Tests in separate files.
- `unsafe` only where needed, always with a `// SAFETY:` comment.
- Architecture-specific code under `arch/<arch>/`.
- Comments in Czech; code uses ASCII punctuation only (`-`, `->`, `...`).
- New error code: add it to `libs/abi/src/error.rs` (next free number) and
  document it in [errors.md](errors.md) and the Czech version - otherwise a
  test fails.
