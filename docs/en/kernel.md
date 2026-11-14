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
  written and mapped. Pages are created on demand (commit at creation, a
  frame on the first write, reads through the shared zero page)
  **[verified in QEMU]**; details in
  [memory-management.md](memory-management.md).
- **Process address space** (`mm/address_space.rs`): a list of mappings
  (object + offset + rights). Unmapping flushes the TLB on all CPUs
  (`mm/tlb.rs`) before the pages are freed.

## Scheduler

Details, design comparison and measurements: [Scheduler](scheduler.md).

- Each CPU has one run queue (`libs/schedcore`): first `realtime` threads
  with budget left (fixed budget per period, EDF), then the fair classes
  `interactive`, `normal` and `background` weighted 15 : 4 : 1 (weighted
  class virtual time), EEVDF inside a class with a per-class quantum
  (3 / 6 / 12 ms). Nothing starves: `background` keeps at least 1/20 of a
  busy CPU outside `realtime`.
- **Class assignment**: syscall `SCHED_CONTROL`, ceiling from the job (an
  application cannot make itself `realtime`); a message inherits the
  sender's class to the receiving thread (QoS propagation over IPC).
- **Tickless**: the timer is set only to the end of the quantum, a
  `realtime` budget refill or the next wake-up; an idle CPU sleeps in `hlt`.
- **Load balancing** by class and core capacity: `interactive` and
  `realtime` to performance cores, `background` to efficiency cores
  (Intel hybrid CPUID 0x1A, ARM MIDR, ACPI PPTT); an idle CPU steals work.
- **Preemption** never puts a thread to sleep or ends it, it only requeues
  it - a thread can be interrupted anywhere, even while preparing to wait.
- **Wake-ups**: each wait has a sequence number; a wake-up with an old
  number is ignored. Spurious wake-ups are allowed; the waiting code
  re-checks its condition.
- **Switching to a process thread** activates its address space (CR3) and
  the kernel stack for entries from userspace (TSS.RSP0); kernel threads and
  exiting process threads run in the kernel address space.
- **The kernel stack of a dead thread** (64 KiB) and its part of the job
  budget are returned as soon as the CPU finishes the last switch away from
  the thread (`finish_switch` after `exit`). The thread object with its
  signals and exit code lives on while a handle or a futex queue holds it,
  and returns only its small part of the budget. A thread preempted after
  it was marked dead keeps its stack because it still has to run.
  **[verified in QEMU]**

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
  finishes the process.
- **A finished process's memory is returned at once** **[verified in
  QEMU]** (test `job: pamet skonceneho procesu se vrati hned`, x86_64 and
  aarch64): on finishing, the address space is torn down (mappings, memory
  objects held only by a mapping, page tables including the root), handles
  are closed (channels and other objects) and futex queues emptied; job
  budgets are returned. Only then does the process get `TERMINATED`, so a
  waiter already sees the memory returned. Whoever holds a handle to a
  finished process (parent, supervisor) holds only a small **tombstone**:
  number, name, exit code, CPU time and signals for `PROCESS_EXIT_CODE`,
  `JOB_PROCESSES` and waiting. It goes away with the last handle.
- **Ordering for races:** an exiting thread first switches its CPU to the
  kernel address space and the scheduler no longer loads the process space
  for it; only then does it decrement the live thread count. When the last
  thread ends, no CPU has the process space active and the kernel may tear
  it down (after a TLB flush on all CPUs). Another thread's syscall cannot
  overtake the end: the process finishes only after all its threads left
  their syscalls. Operations of other processes through a handle (`VM_MAP`
  -> `BAD_STATE`, `VM_UNMAP` -> `NOT_FOUND`, `THREAD_CREATE` ->
  `BAD_STATE`) are serialized by the address space lock.
- **DMA memory of a driven device** is not returned at once when the
  process ends: it goes to the DMA quarantine, the kernel stops the device
  and releases the memory only afterwards (see [IOMMU](#iommu-dma-protection),
  DMA quarantine).
- **Suspending a thread and its context** (`THREAD_SUSPEND`,
  `THREAD_STATE`, `kernel/src/sched/suspend.rs`) **[verified in QEMU]**
  (suite `procesy`, x86_64 and aarch64): a suspend token stops the thread at
  the boundary of returning to userspace (also before its first entry); a
  thread sleeping in a syscall is suspended at once. General registers are
  in the frame at the top of the thread's kernel stack (every entry from
  userspace saves it there in full), FPU/SIMD in `Context`. A read works
  only for a standing thread; the kernel validates a write and the thread
  applies it itself right before returning (registers into the frame,
  FPU/SIMD straight into the registers), so it never races with saving the
  FPU on a switch. Kill always wins. The basis for exception delivery (SEH
  in the Windows layer, signals in `cstd`) and a debugger.
- **An exception in a program** (page fault, invalid opcode, ...) ends only
  that process, not the kernel: code -1,000,100 - vector, address and fault
  type in the log. Before that the kernel offers it to **exception
  channels** (`EXCEPTION_CHANNEL`, `kernel/src/object/exception.rs`)
  **[verified in QEMU]** (suite `procesy`, x86_64 and aarch64) in order
  debugger (first chance), thread, process, debugger (second chance). The
  thread stands in the exception and a handler may read and change its
  context and let it continue, pass the exception on, or end the process.
  A handler thread (`HANDLER_THREAD`) skips the thread and process
  channels so it never waits for itself. Kill always wins. The basis for
  SEH in the Windows layer, signals in `cstd` and a debugger. The fast path
  without a switch: a handler right in the thread (`EXCEPTION_HANDLER`);
  details in [exceptions.md](exceptions.md).
- **aarch64 does not report division by zero** (`udiv`/`sdiv` return 0);
  the "divide by zero" exception exists only on x86_64. `cstd` (SIGFPE)
  and Wine (`EXCEPTION_INT_DIVIDE_BY_ZERO`) must provide it on aarch64 by
  checking in code, as everywhere on ARM.
- **`int3` from userspace** (a breakpoint) has an IDT gate with DPL 3,
  otherwise it would end as a general protection fault.
- **Entering userspace:** a thread's entry point and stack must lie in the
  process memory (`USER_MIN`..`USER_MAX`); the last page of the lower half
  of the address space is always unmapped, so a return from a syscall never
  targets a non-canonical address (which would fault while still in the
  kernel).
- **Thread pointer register** (aarch64 `TPIDR_EL0`, which a program may
  write itself): the scheduler saves it into the outgoing thread on a
  switch and loads the value of the incoming one; a new thread starts with
  0. A value of one process is thus never visible in another
  **[verified in QEMU]** (test `procesy: TPIDR_EL0 neprosakuje mezi
  procesy`, aarch64). On x86_64 a program cannot set `FS_BASE` itself.
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
IVRS table), on aarch64 **SMMUv3** (ACPI IORT table). All work the same way
and the rest of the kernel does not distinguish them (`arch::iommu::Domain`):

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
- **DMA quarantine: a device after its driver dies** **[verified in
  QEMU]** (test `edu: obnova po padu`, with and without an IOMMU, x86_64
  and aarch64; decisions in the self-test `karantena DMA`). After its
  driver ends, a device may keep writing from the driver's descriptors -
  without an IOMMU into pages someone else already got. Therefore
  (`kernel/src/object/quarantine.rs`):
  1. when a process that **drove** a device (created DMA memory for it
     itself) ends, its DMA memory of that device goes to quarantine;
  2. the kernel clears the device's **bus mastering** in the PCI
     configuration (it writes only the COMMAND register) and waits for
     in-flight transfers to finish (PCIe Device Status, Transactions
     Pending, at most a hundred milliseconds; without a PCIe capability a
     fixed ten milliseconds);
  3. only then it releases the memory - and only then the process gets
     `TERMINATED`, so a new driver starts only after the stop. The new
     driver enables bus mastering itself (`libs/pci`).
- **A client** with a shared buffer of the driver does not stop the device:
  only a device for which the dying process created DMA memory itself is
  stopped.
- **`dk:crash`** in driverkit behaves like a real crash: the driver neither
  stops nor releases anything; the kernel cleans up and stops the device.
- **A device that cannot be stopped** (not on PCI Express, or it does not
  finish its transfers) keeps its memory in quarantine forever; the kernel
  logs it with the text `karantena DMA` and `report` (the developer bundle)
  shows it in the preview and in a section of its own.
- **DMA memory only with a device identity:** `VMO_CREATE_CONTIGUOUS` from a
  resource without a device identity returns `ACCESS_DENIED` (the kernel
  could neither stop the device nor put the memory into an IOMMU domain).
  All drivers get a resource with an identity from devmgr.
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

On aarch64 it is `kernel/src/arch/aarch64/smmu.rs` (SMMUv3): a two-level
stream table (StreamID = RID per IORT, an invalid entry = DMA blocked) ->
an STE with stage 1 configuration -> a context descriptor with its own ASID
and page tables in the CPU format; invalidation through the command queue
(CFGI_STE, TLBI_NH_ASID, SYNC); faults from the event queue are logged
during domain operations. Device MSIs pass through the SMMU, so the ITS
page (GITS_TRANSLATER) is also mapped into the device's domain.

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
- **Framebuffer for user space:** when the firmware provided one (and its
  physical address is page aligned), init gets the memory object
  `bootstrap::kind::FRAMEBUFFER` in its start message (geometry from
  `BootInfo`, `FB_CONTROL` `INFO`). Init passes it to a single process -
  the text and rescue console (`user/console`). The takeover is explicit
  (`FB_CONTROL` `CLAIM`): the kernel then no longer draws the log into the
  framebuffer, it keeps going to the serial line. `RELEASE` gives it back
  to the kernel (the screen is cleared); the end (or crash) of the process
  that took it over does the same. Without a framebuffer the entry is
  missing.
- **ACPI data for user space:** init gets the memory object
  `bootstrap::kind::ACPI` (format `abi::acpi::Info`): the RSDP address and
  a copy of it (copied at boot - the RSDP may lie in memory the kernel later
  frees) and the firmware memory regions holding the tables
  (`AcpiReclaimable`, `AcpiNvs`). Init passes them to the `acpid` service,
  which may map inside them (`VMO_CREATE_PHYSICAL`, cached) - see
  [ACPI](acpi.md). The kernel itself reads only MADT, MCFG, HPET and the
  IOMMU tables.
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
