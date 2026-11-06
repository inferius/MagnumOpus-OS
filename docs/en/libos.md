# libos - the program library

`libs/libos` is the library every userspace program uses. It is the
**stable interface for applications**; the syscalls underneath may change.

## Minimal program

```rust
#![no_std]
#![no_main]

extern crate alloc;

use libos::{Startup, println};

libos::entry!(main);

fn main(startup: Startup) -> i64 {
    println!("hello");
    0 // process exit code
}
```

A program needs a `build.rs` that adds the `user/user.ld` linker script
(see `user/init/build.rs`) and is built by `cargo xtask build` (same target
as the kernel, different compiler flags). Add the program name to
`USER_PROGRAMS` in `xtask/src/main.rs` so it ends up in initfs.

## Process startup

The kernel (for `init`) or the parent (for everything else) starts the first
thread at libos's `_start` symbol with the bootstrap channel handle in
`rdi`. libos then:

1. reads the startup message (`abi::bootstrap`): handles and their meaning,
2. maps a heap (starts at 4 MiB at a fixed address and grows on demand: when an allocation fails it maps more memory right after the end, up to 64 GiB) and enables the global allocator (`alloc::vec::Vec` etc.),
3. calls the program function from `entry!` with a [`Startup`]:
   - `process` - handle to its own process (memory mapping, threads),
   - `initfs` - the initfs image (`init` only),
   - `handles` - further handles from the parent,
   - `bootstrap` - channel to the parent for further communication,
4. uses the return value as the process exit code (`PROCESS_EXIT`).

A panic in the program logs the message and exits with code -1,000,001.

## Object types

Each type owns a handle and closes it when dropped.

| Type | Purpose | Main methods |
|---|---|---|
| `Handle` | generic handle | `duplicate(rights)`, `into_raw`, `from_raw` |
| `Event` | signal between threads and processes | `create`, `signal`, `reset` |
| `Channel` | messages with data and handles | `create`, `write`, `try_read`, `read(deadline)` |
| `Vmo` | memory object | `create`, `read`, `write` (any length; the kernel moves 1 MiB per call, libos splits) |
| `Process` | process | `create`, `map`, `unmap`, `start`, `kill`, `wait_exit` |
| `Thread` | thread | `create`, `start`, `wait_exit` |
| `Mapping` | a memory object mapped into the own address space (unmapped when dropped) | `new`, `read::<T>`, `write::<T>`, `read_bytes`, `write_bytes` |
| `hw::Resource` | hardware access (drivers) | `restrict`, `restrict_device` (with an IOMMU domain), `device_memory`, `dma_memory`, `phys_addr`, `interrupt` |
| `hw::Interrupt` | device interrupt (MSI) | `msi`, `wait(deadline)` |

`Mapping::read`/`write` are volatile (suitable for device registers too)
and accept only integers (`libos::Pod`: `u8`..`u64`, `i8`..`i64`) - arbitrary
device bytes can therefore never form an invalid value (`bool`, reference,
enum). The offset must be aligned to the type size and lie inside the
mapping (checked against overflow as well); otherwise a panic.

```rust
let regs = resource.device_memory(bar_phys, 4096)?;          // only within the Resource's ranges
let map = Mapping::new(&regs, 0, 4096, MAP_READ | MAP_WRITE)?;
let status: u32 = map.read(0x14);
map.write::<u32>(0x14, status | 1);
```

Functions: `wait_one`, `wait_many` (waiting for signals), `time::now`,
`time::deadline_after`, `time::sleep`, `exit(code)`, `println!` (kernel log).

Errors are `libos::Error` (= `abi::Error`), described in [errors.md](errors.md).
Interface constants (mapping flags, process exit codes) are in `libos::abi`.

## Starting a program

```rust
let fs = libos::initfs::Initfs::open(&startup)?;          // init only
let image = fs.find("echo").ok_or(Error::NotFound)?;
let (process, channel) = libos::spawn("echo", image, Vec::new())?;
channel.write(b"ping:1", Vec::new())?;
let reply = channel.read(None)?;
let code = process.wait_exit(None)?;
```

`spawn` creates an empty process, maps the ELF segments into it (with
per-segment rights, W^X) plus a 256 KiB stack, sends the startup message and
starts the first thread. It returns the process and the channel end to the
child.

## Low level

`libos::sys` has the raw syscall wrappers (one function per syscall, see
[syscalls.md](syscalls.md)). It is public for interface tests; programs
should use the types above.
