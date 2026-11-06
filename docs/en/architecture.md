# Architecture

A living document: what we are building, how and why. Every decision states
its reason and its cost. When a decision changes, it is updated here
together with the code. (The Czech version, `docs/cs/architektura.md`, is
the primary one.)

## Principles

1. **A clean kernel.** The kernel knows nothing about Windows or any other system.
2. **No bending for compatibility.** When the Windows layer needs something
   the native system would not want on its own, it is solved in the layer,
   not in the kernel. Only general mechanisms that make sense natively go
   into the kernel.
3. **Capabilities.** No root, no ACLs. A process can access only what it has
   a handle to.
4. **Asynchronous I/O from the ground up**; the synchronous API is a thin
   layer on top.
5. **Drivers in userspace.** A crashing driver does not take the system down.
6. **Modern platforms only:** 64-bit, UEFI / device tree, ACPI, SMP.
7. **UTF-8 everywhere.**
8. **Portability:** architecture code under `arch/<arch>/`, x86_64 first.

## Layers

```
+--------------------------------------------------------------+
|  Native applications         |  Windows applications (.exe)  |
|                              |  Windows layer (sandbox)      |
|  libos (native API, C ABI)   |  libos                        |
+--------------------------------------------------------------+
|  System services (userspace): init, device manager,          |
|  drivers, storage + FS, namespace, compositor, configuration |
+--------------------------------------------------------------+
|  Microkernel: memory, scheduler, IPC, interrupts, objects,   |
|  handles + rights, asynchronous queues                       |
+--------------------------------------------------------------+
|  arch/x86_64 (later arch/aarch64)    |  UEFI loader          |
+--------------------------------------------------------------+
```

## Kernel

### Type: microkernel

Only memory, scheduling, IPC, interrupts and objects with handles live in
the kernel.

- **Why:** userspace drivers and capabilities lead naturally to a
  microkernel (seL4, Zircon, QNX). A hybrid opens the door to exceptions
  "for performance" (win32k in Windows).
- **Cost:** more context switches. Mitigation: zero-copy through shared
  memory, queues without a syscall per operation, a fast path for
  synchronous calls (direct switch to the receiver like seL4 call/reply).

### Objects, handles, rights

Everything that can be referred to is a kernel object. A process accesses it
through a handle (an index into its handle table). A handle carries rights
(bitmask: `READ`, `WRITE`, `EXECUTE`, `MAP`, `DUPLICATE`, `TRANSFER`, `WAIT`,
`SIGNAL`, ...). Rights can only be removed when duplicating.

| Object | Meaning |
|---|---|
| `Process` | address space + handle table |
| `Thread` | thread |
| `MemoryObject` | memory pages, lazy allocation, copy-on-write clones |
| `AddressRegion` | part of an address space, mappings |
| `Channel` | bidirectional message queue (bytes + handles) |
| `Port` | completion and event queue |
| `Event`, `Timer` | synchronization |
| `Interrupt` | IRQs delivered to drivers |
| `MmioRange`, `IoPorts`, `DmaRegion` | driver access to hardware |
| `Job` | a group of processes with limits |

The kernel has no global namespace. Names are handled by a namespace service
in userspace; each process sees only its own tree (Plan 9).

Every object has **signals** (a bitmask of states). Waiting: `wait_one`,
`wait_many`, or registering on a `Port`, which receives a packet when a
signal changes.

### Asynchronous I/O

A process creates a ring: submission and completion queues in shared memory
(io_uring style). Syscalls only wake up and sleep. Synchronous functions in
libos = submit a request and wait for completion.

### IPC

- `Channel`: a message of up to 64 KiB of data and 64 handles. Handles are
  moved with the message.
- Large data: send a handle to a `MemoryObject` (zero-copy).
- Service interfaces are described in a simple custom IDL that generates code.

### Memory

- Physical memory: buddy allocator over the loader's memory map.
- Virtual: 4-level paging, kernel in the upper half, direct map of physical
  memory at `0xFFFF_8000_0000_0000`, kernel at `0xFFFF_FFFF_8000_0000`.
- `MemoryObject`: lazy allocation, copy-on-write clones.
- Kernel heap: slab for fixed-size objects + general heap.

### Scheduler

- Per-CPU queues, work stealing, SMP since milestone 3.
- Tickless (APIC one-shot, later TSC deadline) for power: the timer is set
  only to the end of the quantum or the next wake-up; idle CPUs sleep
  without interrupts.
- QoS classes instead of raw priorities: `realtime`, `interactive`,
  `normal`, `background`. On big.LITTLE they map to core types. For now,
  strict priority between classes and round-robin within (10 ms quantum);
  later a fairer algorithm (EEVDF) so lower classes do not starve.
- Load balancing: new and woken threads go to idle CPUs, a preempted thread
  moves to an idle CPU, an idle CPU steals work.
- Wake-ups: each wait has a sequence number, a wake-up with an old number is
  ignored; spurious wake-ups are allowed and the waiting code re-checks.
- Preemption never puts a thread to sleep or ends it, it only requeues it.
  A thread can be interrupted anywhere, even while preparing to wait.
- Clock: TSC calibrated against the HPET (assumption: invariant TSC in sync
  across CPUs; checked and corrected at CPU startup).

### User interface responsiveness (plan)

A single "UI thread" is not enough to keep the UI smooth - the whole chain
from input to display has to be responsive:

- **QoS `interactive`** for the compositor and the threads of the focused
  window: short quanta, preferred wake-up; background work (FS commit,
  defragmentation, indexing, scrub) runs as `background`.
- **Priority inheritance over IPC:** when a UI thread waits for a service
  reply (fsd, a driver), the service handles the message with the caller's
  QoS. Without it, priority stops at the first service and a microkernel
  stutters exactly there.
- **I/O priorities:** disk requests carry a class; reads for the UI
  overtake defragmentation writes. fsd never blocks reads because of a commit.
- **Frame deadlines:** the compositor schedules by vsync (deadline), not by
  queue order.
- **Applications:** the UI thread only draws and reacts; work is done
  asynchronously (libos has asynchronous I/O as its base model).

### Syscall ABI and libos

**libos** is the userspace library every process has (like `ntdll` on
Windows or vDSO + libc on Linux).

- The stable ABI is a libos function, not a syscall number. The kernel can
  change syscalls without breaking applications.
- It contains the synchronous API on top of asynchronous queues and IPC helpers.
- It will export a C ABI; internally it is Rust. The Windows layer is just
  another client.

## Security

- Init gets the root capabilities and hands them out to services, each only
  its own part.
- A user is a userspace concept: a session = a set of capabilities.
- Applications always run sandboxed; the manifest says which capabilities
  they need.
- File access = a handle to the directory it is in. No ACLs.

## System services

| Service | Task |
|---|---|
| init / supervisor | starts services, restarts crashed ones |
| device manager | enumerates PCI, ACPI, device tree; starts drivers with their resources |
| drivers | virtio first, then NVMe, xHCI, HDA, ... |
| ACPI | AML interpreter; the kernel itself parses only static tables (MADT, HPET, MCFG) |
| storage + FS | block devices, partitions, file system (`fsd` done) |
| namespace | composes each process's view of the tree |
| compositor | display, input, windows |
| configuration | reading, validating and versioning settings |

## File system

Our own copy-on-write FS is done (format v2); the detailed description
including the on-disk format, algorithms and examples is in
[file-system.md](file-system.md). In short:

- one CoW B+tree for metadata, a Merkle tree with 128-bit checksums
  (XXH3-128 or BLAKE3) in every pointer, a ring of 8 superblocks and a
  fallback of up to 3 generations when the newest state is damaged,
- extents with delayed allocation (contiguous writes), allocation groups
  with space logs, delayed block reuse, TRIM, background defragmentation,
- an intent log for fast `fsync`, snapshots with deletion via deadlists,
  instant directory sizes, fsck and scrub,
- the `fsd` userspace service on top of the disk driver, protocol `fsproto`.

Further according to the design:

- Atomic A/B updates: the system is a read-only subvolume, an update creates
  a new version, a failed boot falls back to the old one (needs writable
  clones - planned).
- Paths: `/` separator, UTF-8, no drive letters.
- Programs at boot are still loaded by the loader from **initfs** (a
  read-only archive in memory); the data disk has our own FS.

### File names: case-insensitive everywhere

Two names a person reads as the same are the same name.

- **Case-preserving, case-insensitive:** a file keeps the name the user gave
  it, but `Foo` and `foo` cannot exist side by side.
- **Unicode NFC normalization:** `é` as one character and `e` + combining
  acute are the same name.
- **Language-independent:** Unicode simple case folding, no locale rules
  (Turkish `i`). The table version is stored in the FS so a new Unicode
  version does not change the meaning of existing names.
- **Cost:** foreign software with names differing only in case (some git
  repositories) cannot be stored. A case-sensitive option for a single
  subvolume will be added if it is ever really needed.

### Content-addressed FS and native versioning (proposal)

At its core git is simple: files and directories identified by a hash of
their content (a Merkle tree), a commit = an immutable tree + metadata +
parents, a branch = a named pointer to a commit. A CoW FS with checksums
already has most of this - every change rewrites the path to the root
anyway. The proposal: take it far enough that versioning is a property of
the FS, not of a program on top of it.

- **A content hash for every file and directory**, maintained by the FS on
  write (Merkle tree, BLAKE3 - fast and hashes in chunks). Can be enabled
  per subvolume, since it has a cost.
- **Snapshot = commit:** an immutable root + author, message, parents. History
  is a DAG, branches are named references, a new branch or clone is CoW
  (instant, no data copied).
- **Diffing two snapshots in time proportional to the change:** an equal
  subtree hash means there is no need to descend into it.
- **Change journal:** an ordered stream of changes in a subvolume (like the
  NTFS USN journal or FSEvents). For build systems, sync, indexing,
  `git status` without scanning the disk.
- **Transactions:** atomic writes of several files at once (all or nothing).
- **Deduplication** for free thanks to content hashes.
- **Programmatic API:** besides paths, a typed IPC interface: transactions,
  metadata queries, hashes, reflink copies, snapshot diffs, change
  subscriptions.

**Git interoperability:** git computes object IDs in its own format (SHA-1
or SHA-256 over `blob <size>\0content`), which cannot be derived from
BLAKE3. A `git` service therefore computes the IDs lazily and caches them
in file metadata. The working tree is the FS itself (no separate index - the
FS knows the state from the journal), repository objects are snapshots,
packfiles only for network exchange. Existing tools use an adapter for the
object store (e.g. a libgit2 backend).

**Order:** the CoW FS itself first (milestone 6), then hashes and the
snapshot DAG, then the git service. The rest will be refined during
implementation.

### Metadata and search (proposal)

Goal: size, type and times of anything instantly, listing a big directory
without waiting, and searching without scanning the disk.

- **Object table (like the NTFS MFT, but a B-tree):** every file and
  directory is a record with a fixed ID: size, times, type, content hash,
  extent list. Small files (up to ~3 KiB) live directly in the record, so
  reading them is a single operation. A B-tree instead of the MFT's fixed
  array: it grows and shrinks, does not fragment, and snapshots share
  unchanged nodes (CoW).
- **Directories** are B-trees keyed by the normalized name (NFC + case
  folding) - O(log n) lookups. A directory entry also carries a copy of the
  most used metadata (size, type, modification time), so listing a
  directory does not jump into the object table (like NTFS index entries).
- **Subtree summaries in every directory:** total size, file count, last
  change. A folder's size is available instantly (no `du`). They are
  updated during the CoW write, which rewrites the path to the root anyway.
- **Search indexes**, maintained in the same transaction as the change:
  name (n-grams, substring search), type/extension, size, modification
  time, tags and typed user attributes (key = value). A query through the
  API, e.g. "name contains `invoice`, size > 1 MiB, modified after Jan 1"
  without scanning the disk. Indexes are optional per subvolume (they cost
  write performance).
- **Full-text** search (in content) is a userspace service over the change
  journal, so it does not slow down writes.

Further planned improvements:

- transparent per-extent compression (zstd, lz4) depending on the data,
- subvolume encryption (the key is a capability, not a password in the FS),
- self-healing storage: checksums + a second copy of metadata (and
  optionally data), repaired from the copy on a read error,
- several disks in one pool (mirroring, growing online),
- quotas and reservations per subvolume,
- file history for free from the snapshot DAG (version timeline, "undo"),
- booting without a disk check (CoW + transactions = always consistent).

## Applications: install, update, uninstall (proposal)

Goal: as simple as a portable application on Windows, as safe as a mobile
sandbox, without the dependency hell of Linux package managers.

- **One application = one directory** `/apps/<id>/` (its own subvolume): the
  program, its libraries and the data that belongs to it. No libraries
  shared between applications, no writes anywhere else. The application's
  user data lives in `/home/<user>/apps/<id>/`.
- **Uninstall = delete those two directories.** Nothing else remains in the
  system, because the application cannot write anywhere else (it only gets
  capabilities to its own directories).
- **A package** is a single file (archive + manifest): name, version,
  publisher, signature, required capabilities (network, camera, user
  files, ...). Installing = verify the signature, show the requested
  permissions, unpack. Works by double-clicking a downloaded file just like
  from the catalog.
- **The official catalog** is just one source: a service that can search,
  download and watch for updates. Applications outside the catalog can be
  installed too; the system just clearly says who signed them.
- **Updates are atomic:** the new version is unpacked into a new snapshot of
  the subvolume and switched over at once; if the new version fails to
  start, the previous one is restored. The user chooses: automatic, notify,
  or never. No updates in the middle of work, no restarts because of
  applications.
- **The system updates the same way** (A/B subvolume), just from its own source.
- Windows applications get the same: installed into their own directory
  with a virtual `C:`, uninstall = delete.

## Graphics

- Compositor in userspace (Wayland-like concept); clients render into
  buffers and pass them over a channel.
- Vulkan-first, HiDPI with fractional scaling, touch as a first-class input.
- GPU drivers:
  - the userspace part (Mesa: Vulkan driver, shader compiler) is ported
    through a winsys layer,
  - the kernel part (on Linux DRM: GPU memory management, command
    submission, firmware) cannot simply be translated, it is deeply tied to
    Linux; here it becomes our own userspace driver written using the Linux
    code as reference.
- Plan: virtio-gpu (Venus) in QEMU, then one family of real GPUs (probably Intel).

## Configuration

- Human-written configuration: **TOML** (comments, readability), validated
  against a schema in the configuration service.
- Versioned as part of the FS snapshot, rollback for free. No registry.
- State written by programs is up to each service.

## Windows layer

A separate userspace personality on top of libos. Good NT concepts have a
native counterpart because they are good on their own, not because of
Windows:

| Windows | Native |
|---|---|
| HANDLE | handle (the layer keeps its own table for NT semantics) |
| WaitForMultipleObjects | `wait_many` |
| IOCP | `Port` |
| Section | `MemoryObject` |
| VirtualAlloc | `AddressRegion` |
| Job | `Job` |
| SEH | exception delivered on an exception channel (a general mechanism, also for debuggers) |
| TEB via GS | a general syscall for a thread-local base register (native TLS) |
| APC | only in the layer, via alertable waits |
| UTF-16 | converted at the layer boundary |
| `C:\...` | translated to a path in the sandbox |
| registry | a file in the sandbox, served by the layer |

Each application has a directory with a virtual `C:` and its own registry.
Uninstall = delete the directory.

First goal (milestone 5): an .exe with a static CRT (`/MT`) that needs only
`kernel32` and `ntdll`. Our own UCRT right after.

## Errors and logging

Goal: every error can be traced - what happened, where, on which CPU and when.

- **Errors:** one type for the kernel and userspace (`libs/abi`), stable
  numeric codes that are also syscall return codes. Context is logged where
  the error happens. All codes: [errors.md](errors.md).
- **Kernel log:** levels, time, CPU, module, source location; global and
  per-module filters; a ring buffer; no allocation. Details:
  [kernel.md](kernel.md#logging), messages: [messages.md](messages.md).
- **Fatal errors:** a panic or exception prints the message, registers and
  stack; xtask translates the addresses into function names.
- Later: a userspace log service, structured records (key = value),
  persistent on-disk log, levels from the kernel command line.

## Code conventions

See [building.md](building.md#code-conventions). Code comments are in Czech,
documentation is in Czech and English (`docs/cs`, `docs/en`).

## Third-party code and license

- Project license `MIT OR Apache-2.0`.
- Anything may be reused (Mesa, DXVK, VKD3D, ...) where writing our own does
  not make sense. Long-term we aim for our own components.

## Platforms

1. QEMU x86_64 + OVMF (main development)
2. real x86_64 PCs
3. QEMU aarch64 `virt`
4. Raspberry Pi, PinePhone (Pro)

aarch64 is ported per the milestone plan (milestone 8). Until then code is
strictly separated into `arch/` so the port does not hurt.

## Languages and compilers (plan)

Agreed approach (2026-11-06):

1. **Rust:** our own target `x86_64-unknown-magnumopus` and a `std` port on
   top of libos (threads, TLS, synchronization, files, processes, time).
2. **C:** our own libc on top of libos (under our control, without POSIX
   legacy where not needed); compiled with clang on the host.
3. **C++:** libc++, libc++abi and libunwind on top of our libc, compiled
   with clang.
4. **A compiler running in the system:** first a TinyCC port (small, fast C
   compilation), later clang/LLVM.

### Our own compiler (long-term goal)

In the future we want to produce **optimal code for our platform**, so our
own compiler is planned as well. It is not needed to start (the above
covers C, C++ and Rust), but it makes sense as a separate project:

- knowledge of our ABI, syscalls and binary format - code without extra
  layers (direct libos calls, IPC calls without wrappers),
- optimizations for our patterns: channels and messages, capability
  handles, async I/O,
- independence from foreign toolchains (self-hosting),
- a sensible order: first a backend (own IR + x86_64 and aarch64 code
  generation) that can also serve an existing frontend (e.g. as a C backend
  or a cranelift-like backend for Rust), only then our own language frontend.

## Temporary solutions

| What | Why | Gone when |
|---|---|---|
| serial driver in the kernel | debug output before userspace exists | stays only as an early console |
| framebuffer console in the kernel | same | taken over by the compositor |

## Milestones

1. Boot: UEFI loader, kernel prints text (done)
2. Kernel basics: GDT/IDT, exceptions, physical allocator, paging, heap (done)
3. Processes: scheduler, SMP, threads, objects, handles, capabilities (done)
4. Userspace: syscalls, first process, IPC, libos, initfs (done)
5. Windows layer v1: PE loader, ntdll/kernel32, console .exe
6. System: userspace drivers (virtio - done), FS (done), graphics, compositor
7. Languages and compilers (see [Languages and compilers](#languages-and-compilers-plan)):
   Rust (`std` for our OS), C (own libc on top of libos), C++
   (libc++/libunwind on top of libc), cross-compiled with clang from the
   host, then TinyCC in the system
8. Windows layer v2: user32/gdi32, GUI, DirectX via Vulkan
9. aarch64 port and the first ARM device

## Current deviations from the design

What currently works more simply than the design describes (and why it is
fine for now):

- **Syscalls are synchronous** and the driver and FS protocols
  (`blockproto`, `fsproto` v1) copy data in channel messages; request
  queues in shared memory (io_uring style) come when performance needs them.
- **Memory objects allocate pages up front** (no lazy allocation, no CoW clones).
- **Only whole mappings can be unmapped.**
- **Syscalls return via `iretq`** instead of the faster `sysret`.
- **Programs are still compiled without SSE/FPU** (soft-float). The kernel
  already saves each thread's FPU/SSE state (FXSAVE, x87 + XMM0-15); AVX
  (XSAVE) comes with the languages milestone.
- **IOMMU (Intel VT-d, AMD-Vi) only for DMA translation:** interrupt
  remapping and RMRR/IVMD regions are missing; on a machine without an
  IOMMU DMA is not restricted (the kernel logs it).
- **Quotas only per process**, not for groups of processes (job).
- **Strict priority between QoS classes** (lower classes can starve).
- **Memory of finished threads** (kernel stacks) is reused but not returned.
