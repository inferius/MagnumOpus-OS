# Kernel: core functions

How the kernel works today (milestone 4). Why it is designed this way is
described in [architecture.md](architecture.md).

## Boot

1. The **UEFI loader** (`loader/`) loads `\os\kernel.elf` and
   `\os\initfs.img` from the ESP, builds page tables (identity, direct map
   of physical memory, kernel in the top 2 GiB), finds the framebuffer and
   ACPI RSDP, exits boot services and jumps into the kernel with a pointer to
   `BootInfo` (`libs/boot-info`).
2. **Kernel on the first CPU** (`kernel/src/main.rs`): console, GDT/TSS/IDT,
   memory management (own page tables), ACPI (MADT, HPET), clock (TSC) and
   APIC timer calibration, reclaiming firmware memory, switching to a stack
   with a guard page.
3. **Scheduler and other CPUs**: CPU registration, starting the other CPUs
   (INIT-SIPI-SIPI, TSC offset check and correction), the `boot` thread.
4. **The `boot` thread**: kernel self-tests, taking over initfs, starting the
   `init` process. In test mode (`cargo xtask test`) it waits for `init` to
   finish and exits QEMU with the result.

## Memory

**Virtual address space** (each process has its own lower half and shares
the upper half with the kernel):

| From | What |
|---|---|
| `0x0000_0000_0001_0000` | process memory (nothing below - a null pointer always faults) |
| `0x0000_0000_0040_0000` | programs (static ELF) |
| `0x0000_1000_0000_0000` | kernel-chosen mappings (heap, stacks, ...) |
| `0xFFFF_8000_0000_0000` | direct map of all physical memory (RW, NX) |
| `0xFFFF_FD00_0000_0000` | device registers (MMIO, uncached) |
| `0xFFFF_FE00_0000_0000` | kernel stacks (each with a guard page) |
| `0xFFFF_FFFF_8000_0000` | kernel image (code R-X, constants R--, data RW-) |

- **Physical memory**: buddy allocator (`mm/phys.rs`), blocks of 4 KiB to
  4 MiB. Memory below 1 MiB is reserved for starting CPUs.
- **Kernel heap** (`mm/heap.rs`): size classes 16 B - 2 KiB, larger
  allocations straight from the buddy allocator (max 4 MiB at once).
- **Protection**: NX for data, W^X for programs, SMEP (the kernel never
  executes process code) and SMAP (the kernel never touches process memory
  through process addresses - syscall data is copied through the direct map
  using the mapping list, so no page fault on a process address can happen
  in the kernel).
- **Memory objects** (`object/vmo.rs`): physical pages that can be read,
  written and mapped; currently allocated up front.
- **Process address space** (`mm/address_space.rs`): a list of mappings
  (object + offset + rights). Unmapping flushes the TLB on all CPUs
  (`mm/tlb.rs`) before the pages are freed.

## Scheduler

- Each CPU has thread queues per QoS class (`realtime`, `interactive`,
  `normal`, `background`): strict priority between classes, round-robin with
  a 10 ms quantum within a class.
- **Tickless**: the timer is set only to the end of the quantum or the next
  wake-up; an idle CPU sleeps in `hlt`.
- **Load balancing**: new and woken threads go to idle CPUs; a preempted
  thread moves to an idle CPU; an idle CPU steals work from other queues.
- **Preemption** never puts a thread to sleep or ends it, it only requeues
  it - a thread can be interrupted anywhere, even while preparing to wait.
- **Wake-ups**: each wait has a sequence number; a wake-up with an old
  number is ignored. Spurious wake-ups are allowed; the waiting code
  re-checks its condition.
- **Switching to a process thread** activates its address space (CR3) and
  the kernel stack for entries from userspace (TSS.RSP0); kernel threads run
  in the kernel address space.

## Objects, handles, rights

- Objects: `Event`, `Channel` (channel end), `MemoryObject`, `Process`,
  `Thread`. Each has **signals** (`SIGNALED`, `TERMINATED`, `READABLE`,
  `PEER_CLOSED`) that can be waited on (one or several objects, with a
  deadline).
- **Process handle table**: handle = 20-bit index + 12-bit generation. A slot
  that runs out of generations is retired, so a stale handle never becomes
  valid for another object.
- **Rights** (`READ`, `WRITE`, `EXECUTE`, `MAP`, `DUPLICATE`, `TRANSFER`,
  `WAIT`, `SIGNAL`, `MANAGE`) can only be removed when duplicating.
- **Channels**: messages with data and handles; transferring a handle moves
  it (the sender loses it). Closing an end sets `PEER_CLOSED` on the other.

## Processes and threads

- A process starts empty (`PROCESS_CREATE`); the parent maps memory into it
  and starts the first thread (`PROCESS_START`) with a bootstrap channel. The
  executable format is libos's job; the kernel only loads `init`.
- **Process termination**: `PROCESS_EXIT`, `PROCESS_KILL`, a crash (CPU
  exception) or the last thread ending. All threads get a termination
  request (waiting ones end with `CANCELED`, ones running elsewhere are
  interrupted by an IPI) and exit on return to userspace. The last thread
  finishes the process: handles are closed (objects freed) and the process
  gets `TERMINATED`. The address space goes away with the last reference to
  the process.
- **An exception in a program** (page fault, invalid opcode, ...) ends only
  that process, not the kernel: code -1,000,100 - vector, address and fault
  type in the log.
- **Entering userspace:** a thread's entry point and stack must lie in the
  process memory (`USER_MIN`..`USER_MAX`); the last page of the lower half
  of the address space is always unmapped, so a return from a syscall never
  targets a non-canonical address (which would fault while still in the
  kernel).
- **FPU/SSE state:** each thread has its own 512 B FXSAVE area; a context
  switch always saves and restores it. A new thread starts with the default
  state (FCW 0x037F, MXCSR 0x1F80), so data of another thread cannot leak.
  The kernel itself does not use FPU/SSE (soft-float). AVX is not enabled
  (its instructions fault with #UD).
- **Limits:** handles, threads, mappings, channel queues and the kernel
  memory reserve - see [syscalls.md](syscalls.md#resource-limits).

## Syscalls

Entered with `syscall`, returned through `iretq` (avoiding `sysret` pitfalls;
to be optimized later). Interface: [syscalls.md](syscalls.md), errors:
[errors.md](errors.md).

## Logging

A record:

```text
[    0.247482] cpu0 W tests::log: message (kernel/src/tests/log.rs:35)
 time since boot CPU level module     text      location (W and E only)
```

- Levels: `E` error, `W` warning, `I` info, `D` debug, `T` trace.
- Outputs: serial (everything enabled), framebuffer (up to Info), a 64 KiB
  ring buffer (`log::snapshot`, for a future log service).
- Filter: global level (`log::set_level`), level for a module and its
  submodules (`log::set_target_level("sched", Level::Trace)`). Release builds
  do not compile Trace at all (feature `log-trace` brings it back). A
  disabled level costs one atomic load. Logging does not allocate and works
  in interrupt handlers.
- Errors with context: `result.context("what was being done")` logs the
  error with the call site (Debug), `warn_context` does the same as a warning.
- Programs log via `DEBUG_LOG` (`println!` in libos); records are prefixed
  with `[name id]`.
- Message catalog: [messages.md](messages.md).

## Tests

- **Kernel** (`kernel/src/tests/`): memory, platform, log, scheduler,
  objects, userspace mechanisms. Run in the `boot` thread on every boot.
- **Userspace** (`user/init/src/tests/`): syscalls through libos including
  error codes, memory, in-process IPC, processes (spawn, ping-pong with
  `echo`, handle transfer, exit codes, kill, crash, second thread).
- **Libraries on the host**: `cargo test -p abi -p initfs --features initfs/std`
  (initfs format, ABI, completeness of the error and syscall docs).
- `cargo xtask test` runs everything and returns the result as its exit code.
