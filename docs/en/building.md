# Building, running, testing, debugging

## Tools

- **Rust via rustup.** The nightly, components and targets are pinned in
  `rust-toolchain.toml`; rustup installs them on the first build.
- **QEMU** (`qemu-system-x86_64`) with edk2/OVMF firmware:
  - Windows: `winget install SoftwareFreedomConservancy.QEMU`
  - Debian/Ubuntu: `apt install qemu-system-x86 ovmf`
  - Fedora: `dnf install qemu-system-x86 edk2-ovmf`
  - Arch: `pacman -S qemu-desktop edk2-ovmf`
  - macOS: `brew install qemu`
- Optionally **GDB** for debugging.

xtask looks for QEMU in `PATH` and common install locations, and for OVMF
next to QEMU or in `/usr/share/...`. Otherwise set `QEMU`, `OVMF_CODE`,
`OVMF_VARS`.

## Commands

| Command | What it does |
|---|---|
| `cargo xtask build` | builds the kernel, loader and userspace programs, creates initfs |
| `cargo xtask image` | build + disk image `target/os-x86_64-<profile>.img` (GPT, FAT32 ESP) |
| `cargo xtask run` | image + QEMU with a window; serial output in the terminal |
| `cargo xtask test` | host library tests (`abi`, `initfs`, `fs`), documentation, then kernel and userspace in QEMU without a window |

Options: `--release`, `--headless` (no window), `--gdb` (QEMU waits for
GDB), `--accel <tcg|kvm|whpx|hvf>`, `-- <QEMU args>` (e.g. `-- -smp 8`,
`-- -cpu max,-x2apic`).

The file system has its own host tests (fast, no QEMU):

```text
cargo test -p fs --features std                                   # everything, default number of random cases
PROPTEST_CASES=150000 cargo test -p fs --features std --release   # thorough
cargo test -p fs --features std --release -- --ignored --nocapture perf   # performance and overhead
```

## Disk contents

```text
EFI/BOOT/BOOTX64.EFI   UEFI loader
os/kernel.elf          kernel (ELF with debug symbols)
os/initfs.img          initfs: init, echo, devmgr, virtio-blk, fsd
```

QEMU gets three disks (all virtio-blk):

| Disk | File | Contents |
|---|---|---|
| system | `target/os-x86_64-<profile>.img` | GPT + the ESP above |
| scratch | `target/scratch.img` | 1 MiB of zeros; driver tests write to it |
| data | `target/data.img` | 64 MiB, our file system (label `data`); created by xtask with the `fs` library, containing `/vitejte.txt` and `/dokumenty/navod.md`; TRIM enabled (`discard=unmap`) |

The scratch and data disks are recreated on every `run`/`test`.

## Output and tracking down errors

- Serial output is saved to `target/serial.log` (tests:
  `target/serial-test.log`).
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
  src/arch/x86_64/  everything x86_64-specific
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
libs/fs/         file system (no_std; std for tools)
  src/space/        free space: allocation groups, space logs
  src/fs/           files, directories, snapshots, intent log, defrag, fsck
  src/tests/        FS tests (model, power loss, performance)
libs/fsproto/    file system protocol (client <-> fsd)
user/init/       first process: starts services, userspace tests
user/echo/       IPC test server
user/devmgr/     device manager (PCI, starts drivers)
user/virtio-blk/ disk driver
user/fsd/        file system service
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
