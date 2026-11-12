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

The entry point, heap and panic handler are behind the `rt` feature (on by
default) **[implemented]**. Libraries depend on libos with
`default-features = false`; applications with `std` have their own
runtime (`libs/osrt`, see [Rust std](rust-std.md)) and read the startup
message through `libos::rt::startup`.

The kernel (for `init`) or the parent (for everything else) starts the first
thread at libos's `_start` symbol with the bootstrap channel handle in
`rdi`. libos then:

1. reads the startup message (`abi::bootstrap`): handles and their meaning,
2. maps a heap (starts at 4 MiB at a fixed address and grows on demand: when an allocation fails it maps more memory right after the end, up to 64 GiB) and enables the global allocator (`alloc::vec::Vec` etc.),
3. calls the program function from `entry!` with a [`Startup`]:
   - `process` - handle to its own process (memory mapping, threads),
   - `initfs` - the initfs image (`init` only),
   - `handles` - further handles from the parent,
   - `framebuffer` - the firmware framebuffer (`init` from the kernel, the
     screen owner from init; kind `FRAMEBUFFER`, geometry and take-over via
     `sys::fb_info` / `sys::fb_claim`, see [System calls](syscalls.md#fb_control)),
   - `acpi` - ACPI data from the kernel (`init` only, kind `ACPI`,
     `abi::acpi::Info`; init passes it to the `acpid` service),
   - `bootstrap` - channel to the parent for further communication,
   - `flags` - start flags from the `SELF_PROCESS` argument
     (`abi::bootstrap::flags`; the kernel sets them only for init, e.g. `TEST_RUN`),
   - `params` - the process parameter block (kind `PARAMS`, see below),
4. uses the return value as the process exit code (`PROCESS_EXIT`).

A panic in the program logs the message and exits with code -1,000,001.

### Heap allocator

- **Small allocations** (up to 4 KiB, alignment up to 16 B) go to 16 size
  classes (`libs/sizeclass`): each class has its own list of free objects,
  allocation and free are O(1). An empty class takes a 4-32 KiB chunk and
  splits it into objects. A freed object stays in its class.
- **Large allocations** and chunks for the classes go to
  `linked_list_allocator` (first-fit). Its list of free ranges thus stays
  short.
- **Why:** first-fit alone walks the whole list of free ranges on every
  allocation. After the heap fragmented (fsd with a large node cache after
  the FS tests) the list had thousands of entries: one B-tree insert cost
  ~10 ms and a shell line with a write 2-4 s. With the classes 0.3 s.
- **Overhead:** rounding up to the class (at most ~1.5x) and at most one
  unused chunk per class (under 0.3 MiB in total).

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
| `hw::Resource` | hardware access (drivers) | `restrict`, `restrict_device` (with an IOMMU domain), `device_memory`, `dma_memory`, `phys_addr`, `interrupt`, `interrupt_line`, `port_read8`, `port_write8` (ranges incl. `hw::io_ports`, `hw::irq_lines`) |
| `hw::Interrupt` | device interrupt (MSI or legacy line) | `msi`, `wait(deadline)`, `ack` |

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
`time::deadline_after`, `time::sleep`, `time::wall`, `time::local` (local
time in a zone from `time::zone`), `exit(code)`, `println!` (kernel log). See
[Real time](time.md).

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

`spawn` creates an empty process in the same job as the caller, maps the
ELF segments into it (with rights by segment, W^X) and a 256 KiB stack,
sends the startup message and starts the first thread. It returns the
process and the channel end to the child. The program memory (segments,
stack) is charged to the child's job.

`spawn_in(&job, ...)` starts a program in another job, for example a
sub-job with limits:

```rust
use libos::job::{Job, pool};
let job = Job::create(&libos::job::current()?)?;
job.set_limit(pool::USER, 64 << 20)?;
let (process, channel) = libos::spawn_in(&job, "echo", image, Vec::new())?;
```

The child gets its job in the startup message without the `WRITE` right,
so it cannot change the limits (see [Jobs](syscalls.md#jobs-resource-budgets)).

## Process parameter block

The parent passes arguments, environment variables and the initial working
directory in a read-only memory object in the startup message
(`kind::PARAMS`, format `abi::params`) **[verified in QEMU]** (test `std:
hello-std: argumenty, vlakna, TLS, soubory`). A memory object instead of
message data: the size is not limited by the channel message limit.
Standard handles travel in the same message as `kind::USER` with tags from
`abi::params::tag` (`STDIN`, `STDOUT`, `STDERR`, `FS`, `NET`).

```rust
use libos::params::Params;
use libos::abi::params::tag;
let params = Params {
    cwd: b"/".to_vec(),
    args: vec![b"hello-std".to_vec(), b"alfa".to_vec()],
    env: vec![b"HELLO=svet".to_vec()],
};
let (out, out_child) = Channel::create()?;
let handles = vec![
    params.start_handle()?,
    StartHandle::tagged(tag::STDOUT, out_child.into_handle()),
];
let (process, _) = libos::spawn("hello-std", image, handles)?;
```

`stdout`, `stderr` and `stdin` are channels with the byte-stream protocol:
every message = a chunk of bytes, closing the channel = end of stream.

## Futex-based lock

`libos::sync::Mutex` (API like `spin::Mutex`: `lock` returns a guard
without an error) waits in the kernel via `FUTEX_WAIT`, so a waiting thread
does not take CPU from the lock holder. It spins briefly first (100
tries). Used by `fsproto` and `svcclient` **[verified in QEMU]** (suites
`souborovy system`, `obnova sluzeb`, `shell`).

## Page size

`libos::vm::page_size()` returns the page size of the own process (asks the
kernel once, `VM_PAGE_SIZE`), `libos::vm::page_size_of(&process)` that of
another process. It is not a constant: the ABI only has the bounds
`MIN_PAGE_SIZE` (4 KiB) and `MAX_PAGE_SIZE` (64 KiB). Alignment for
mappings and resource ranges follows this value:

```rust
let page = libos::vm::page_size();
let len = data_len.next_multiple_of(page);
```

## Barriers for drivers

`libos::barrier` orders accesses to memory shared with a device (DMA) and
to device registers. `core::sync::atomic::fence` is not enough: it only
orders accesses between processors, and on ARM a write to a device
register can overtake earlier writes to memory.

| function | when | x86_64 | aarch64 |
|---|---|---|---|
| `dma_wmb()` | data and descriptors in DMA memory before writing the index | `fence(SeqCst)` | `dmb oshst` |
| `dma_rmb()` | index read from the device before reading its data | `fence(SeqCst)` | `dmb oshld` |
| `mmio_wmb()` | all memory writes before writing a register ("doorbell") | `fence(SeqCst)` | `dsb st` |

```rust
write_descriptors();
libos::barrier::dma_wmb();  // descriptors before the index
write_avail_index();
libos::barrier::mmio_wmb(); // index before notifying the device
notify.write::<u16>(offset, queue_index);
```

This is what `libs/virtio` does (`Virtqueue::submit`, `take_used`,
`Device::notify`).

## Low level

`libos::sys` has the raw syscall wrappers (one function per syscall, see
[syscalls.md](syscalls.md)). It is public for interface tests; programs
should use the types above.
