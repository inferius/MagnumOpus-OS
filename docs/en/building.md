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
- **CMake 3.24+ and Ninja 1.10+** for C ports built with CMake (SDL, see
  [C ports](c-ports.md)): Windows `winget install Kitware.CMake` and
  `winget install Ninja-build.Ninja`, Debian/Ubuntu `apt install cmake
  ninja-build`, macOS `brew install cmake ninja`. xtask checks the versions
  (`cmake --version`, `ninja --version`) and stops with install advice when
  a tool is missing or too old. Verified with CMake 4.4.4 and Ninja 1.13.2
  (Windows, winget). Elsewhere: the `CMAKE`, `NINJA` variables.
- **sh, make, nasm 2.14+, Meson and pkgconf** for the FFmpeg and dav1d ports
  ([Multimedia](multimedia.md), [C ports](c-ports.md)): on Windows from
  **MSYS2** (`winget install MSYS2.MSYS2`, then in MSYS2 `pacman -S make
  nasm meson`; pkgconf, ninja and python come as dependencies), only from
  the official MSYS2 repositories. xtask prefers the MSYS2 tools
  (`C:\msys64\usr\bin`, elsewhere the `MSYS2_ROOT` variable) and runs them in
  `MSYSTEM=MSYS` mode; a missing tool = an error with install advice.
  Debian/Ubuntu `apt install make nasm meson pkgconf`, macOS `brew install
  nasm meson pkgconf`. Verified with MSYS2 2026-09-27 (nasm 3.02, Meson
  1.12.1, Ninja 1.13.2). The `SH`, `MAKE`, `NASM`, `MESON`, `PKG_CONFIG`
  variables set a tool directly.
- **WSL Ubuntu 24.04 with LLVM 23** only for the Windows layer (`cargo
  xtask wine`, [Windows layer](windows-layer.md)): Wine builds in WSL
  because its `configure` and tools want a Unix host. In WSL, the official
  LLVM repository `apt.llvm.org` (`llvm-toolchain-noble-23` with its
  signing key) and `sudo apt install clang-23 lld-23 llvm-23 flex bison
  make gcc`. LLVM 23 has the same major release as the host clang (ABI of
  the headers and the C sysroot). xtask checks the tools in WSL
  (`clang-23 --version`, at least 23.1) and otherwise stops with advice.
  Verified on 2026-11-15 with clang and LLD 23.1.6 (Ubuntu 24.04.4).
  Another distribution: the variable `WINE_WSL_DISTRO` (default
  `Ubuntu`). The build runs on the WSL ext4 in `~/.cache/magnumopus-wine`,
  the result goes to `target/wine/<arch>`.
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
| `cargo xtask test --only sit --repeat 20` | only the selected userspace test suites (names as in the log, comma separated), n times in a row - for finding flaky tests; every extra round adds 60 s to the run limit. QMP input (keys, mouse, screenshots) is sent again in every round: xtask looks for test markers in the log only after the last one it found. A test longer than 20 s is marked `[pomaly]` ("slow") in the log |
| `cargo xtask test --only napajeni` | only the separate power test runs (power-off with the button through ACPI, reboot; they always run after the main run, `--no-power` disables them), see [ACPI](acpi.md#tests) |
| `cargo xtask test --net-online` | also the tests against the real host network (host resolver through slirp); skipped without the switch |
| `cargo xtask usb` | live USB: image `target/usb-<arch>-<profile>.img`; `--list`, `--write <disk>`, `--run`, `--test`, `--size <MiB>`, `--out <file>`, see [Live USB](live-usb.md) |
| `cargo xtask vmware` | live USB image as a VMware Workstation disk: builds, converts to VMDK (`qemu-img`), backs up (last 3) and replaces `<dir>/magnumopus.vmdk`; directory `--dir <dir>` or `MO_VMWARE_DIR`; refuses a running VM (`*.lck` locks). VM: UEFI firmware without Secure Boot, SATA or NVMe disk, `e1000e` network |
| `cargo xtask test --only release` | only the boot in the release profile (kernel, init, services, ACPI and reboot; always runs after the main run unless the test itself runs with `--release`) - release changes optimisation and timing and shows bugs that debug hides |
| `cargo xtask test --only zaznam-padu` | only the crash record test: kernel panic, QEMU reboot with the same RAM, the log service saving the record and the developer bundle ([Live USB](live-usb.md#diagnostics)); it always runs after the main run |
| `cargo xtask test --only opakovany-start` | only the repeated start: the system 3 times in a row on one data disk, each start writes, commits and checks the previous start's data; after the last one xtask checks the disk on the host. Catches "first start vs. later" bugs (normal tests always get a new disk); it always runs after the main run |
| `cargo xtask symbolize <file>` | addresses in a crash record, bundle or log to function names (kernel and programs) |
| `cargo xtask test --only zive-usb` | only the boot from the USB image as a flash drive (it always runs after the main run) |
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
`--data-disk <virtio|nvme|nvme4k|ahci|usb>` (data disk controller, default `virtio`; `nvme4k` = NVMe with 4 KiB sectors; `usb` = a flash
drive on xHCI; on NVMe, AHCI and USB the disk gets a GPT with our partition, see [Disks](disks.md)),
`--gdb` (QEMU waits for GDB), `--accel <tcg|kvm|whpx|hvf>` (on Windows `whpx` = Windows Hypervisor Platform, much faster than the default emulation), `--res <WxH>` (x86_64: default resolution, e.g. `--res 1920x1080`; with a window the pointer is a tablet, so it matches even after resizing the window), `-- <QEMU args>` (e.g. `-- -smp 8`,
`-- -cpu max,-x2apic`).

The file system has its own host tests (fast, no QEMU):

```text
cargo test -p fs --features std                                   # everything, default number of random cases
PROPTEST_CASES=150000 cargo test -p fs --features std --release   # thorough
cargo test -p fs --features std --release -- --ignored --nocapture perf   # performance and overhead
```

## Development build and developer mode

**[implemented]** (`xtask/src/devmode.rs`). An image with the development
profile (the file `accounts-dev` in initfs: `cargo xtask run`,
`cargo xtask usb`, `cargo xtask vmware`) is built with the cargo feature
`dev-mode` of the programs `accountd`, `compd`, `shsess` and `init` (it
turns on the protocols' feature too). It adds
[developer mode](accounts.md#developer-mode): administration actions
without a dialog, a pill and a strip in the bar and the `devmode` command.
The image for users (`cargo xtask image`), the tests (`cargo xtask test`)
and the initfs for installing to a disk are built without it - the mode's
code is not in them at all.

- **USB image = two builds:** the live `os/initfs.img` with the feature
  and `os/instfs.img` (installd copies it to the disk) from a separate
  build without the development profile and without the test's answers
  file (`usb::install_initfs`). Cargo keeps both variants of the programs
  side by side; the first image after a code change compiles the affected
  crates twice, the next run just reads them.
- **Release safeguard:** a build without the development profile searches
  its programs (Rust, not the C ports) for texts that exist only in the
  code behind the feature (`devmode::MARKERS`: an accountd log line, the
  strip's text, the signature of the `devmode` command). A hit = a build
  error. So it guards `instfs.img` too.
- Host tests of the feature: `cargo test -p comp --features dev-mode`,
  `cargo test -p shlang --features dev-mode`, `cargo test -p accountproto
  --features dev-mode`.

## Documentation as a website

`cargo xtask docs` (and also `cargo xtask test`) builds a site with two books: the
**User guide** (`docs/prirucka`, index `docs/prirucka/README.md`, the default page)
and **For developers** (below). The developer book converts `docs/cs` and
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
  The initfs (`initfs-<arch>-<profile>-<pid>.img`), the test image
  (`os-<arch>-<profile>-test-<pid>.img`) and the copy of the firmware
  variables (`fw-<arch>-vars-<pid>.fd`, QEMU writes to it) are per run too.
  Runs prepare the shared std sysroot and the documentation site
  (`target/docs`) one at a time under the `target/std.lock` and
  `target/docs.lock` locks; the sysroot is rewritten only when its content
  changes. Concurrent runs used to delete the sysroot under each other
  ("Directory not empty", os error 145).
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
