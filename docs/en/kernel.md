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
- **Ports** (`kernel/src/object/port.rs`): one-shot registrations "when the
  object's signals intersect the mask, queue a packet with a key". Waiting
  on any number of objects without the `OBJECT_WAIT_MANY` limit. An object
  keeps a registration in its waiter list like a thread (`ObjectCore`),
  with only a weak reference to the port. Room for the packet is reserved
  in advance, so delivery from an interrupt does not allocate. Lock order:
  object, port queue, port signals. See [Ports](syscalls.md#ports).
- **Random numbers** (`kernel/src/random.rs`, crate `libs/csprng`): an
  entropy pool (BLAKE2s) and a ChaCha20 generator with key erasure after
  every request. Sources: `arch::hw_random` (x86 RDSEED/RDRAND), counter
  jitter, interrupt timing (lock-free in `trap::device_irq`) and the
  virtio-rng driver (`RANDOM_ADD`). See
  [Random numbers](syscalls.md#random-numbers).

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
- **Budgets and limits:** kernel memory and user memory are charged to
  the budget of the process job (a tree of jobs, limits in bytes), the
  kernel memory reserve and structural limits - see
  [syscalls.md](syscalls.md#jobs-resource-budgets).

## IOMMU (DMA protection)

A device writes into memory on its own (DMA), to the addresses its driver
puts into descriptors. Without an IOMMU a buggy driver can therefore make
the device overwrite the kernel. The kernel therefore enables the IOMMU when
the machine has one: **Intel VT-d** (ACPI DMAR table) or **AMD-Vi** (ACPI
IVRS table). Both work the same way and the rest of the kernel does not
distinguish them (`arch::iommu::Domain`):

<figure class="diagram">
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="IOMMU">
<defs><marker id="io1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="30" width="150" height="60" rx="6"/><text class="t b" x="85" y="55" text-anchor="middle">device 00:03.0</text><text class="m" x="85" y="74" text-anchor="middle">DMA to address X</text>
<path class="ln" d="M160 60 L218 60" marker-end="url(#io1)"/>
<rect class="acc" x="220" y="20" width="200" height="80" rx="6"/><text class="t b" x="320" y="45" text-anchor="middle">IOMMU (VT-d / AMD-Vi)</text><text class="m" x="320" y="64" text-anchor="middle">root → context 00:03.0</text><text class="m" x="320" y="81" text-anchor="middle">→ domain → page tables</text>
<path class="ln" d="M420 45 L498 45" marker-end="url(#io1)"/>
<rect class="ok" x="500" y="20" width="250" height="50" rx="6"/><text class="t" x="625" y="42" text-anchor="middle">X is in the domain (driver DMA memory)</text><text class="m" x="625" y="60" text-anchor="middle">→ access to RAM allowed</text>
<path class="ln" d="M420 80 L498 120" marker-end="url(#io1)"/>
<rect class="err" x="500" y="95" width="250" height="60" rx="6"/><text class="t" x="625" y="118" text-anchor="middle">X outside the domain</text><text class="m" x="625" y="136" text-anchor="middle">→ blocked + interrupt → kernel log</text>
<text class="m" x="10" y="190">A device without a domain (no driver) may not DMA at all. The domain comes from a restricted Resource</text>
<text class="m" x="10" y="208">with the device identity; only memory from `dma_memory` of that Resource is mapped (device address = physical).</text>
</svg>
<figcaption>DMA translation: each device may access only its own domain.</figcaption>
</figure>

- **Default: denied.** After enabling, no device has a domain, so DMA does
  not pass (a device without a driver writes nothing).
- **A device domain** is created when the device manager creates a
  restricted Resource with the device identity (`RESOURCE_CREATE` with the
  `device` argument, `Resource::restrict_device` in libos). Several
  resources of the same device share one domain.
- **Only DMA memory** from that resource is mapped (`VMO_CREATE_CONTIGUOUS`),
  identically: the device address is the physical address, the driver does
  not change. When the memory object is destroyed, its pages are unmapped
  from the domain before they return to the allocator.
- **A blocked access** is reported by the IOMMU with an interrupt (vector
  36) and the kernel logs it as a warning with the device, address and
  reason code (see [messages.md](messages.md#kernel-iommu)).
- **Without an IOMMU** (the machine has none, has it disabled in the
  firmware, or `cargo xtask run --iommu none`) the kernel logs a warning and
  DMA is not restricted.
- A virtio driver must accept the `VIRTIO_F_ACCESS_PLATFORM` feature (the
  device then uses translation); virtio-blk does.

A driver's steps (nothing extra compared to a system without an IOMMU):

```rust
// devmgr: a resource only for device 00:03.0 (BARs, configuration, quotas)
let res = root.restrict_device(&ranges, 8, 4 << 20, (0, 0x00, 0x03, 0))?;
// driver: DMA memory is mapped into the device's domain
let dma = res.dma_memory(64 * 1024)?;
let phys = res.phys_addr(&dma, 0)?; // = device address
```

Implementation in `kernel/src/arch/x86_64/iommu/`:

| File | What it does |
|---|---|
| `mod.rs` | the common interface: `init`, `domain(segment, bus, function)`, `Domain::map/unmap`, fault interrupt handling |
| `vtd.rs` | Intel VT-d: root table -> bus context tables -> domain page tables (3 or 4 levels depending on the unit); after every change a global invalidation of the context cache and IOTLB through registers; faults from the fault recording registers |
| `amdvi.rs` | AMD-Vi: device table (65,536 entries, default "everything blocked") -> domain page tables (4 levels); invalidation through the command queue with a completion wait; faults from the event log (MSI interrupt set up in the IOMMU's own PCI configuration) |

Fault output is limited: one blocked transfer can raise dozens of events,
the first 4 are logged in detail and the rest as a summary.

Missing for now: interrupt remapping, the VT-d invalidation queue (QI), RMRR
and IVMD regions (memory the firmware keeps using after boot, e.g. older
USB) and devices behind PCI bridges listed in the tables.

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
