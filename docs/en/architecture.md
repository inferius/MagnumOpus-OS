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
8. **Portability:** architecture code under `arch/<arch>/` behind a fixed
   [interface](#architecture-interface), x86_64 first.

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

Our own copy-on-write FS is done (format v3 is being introduced in phases - see [format versions](fs-versions.md)); the detailed description
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
- **A Wayland-like path, not X11:** the compositor only composes client
  buffers; applications see neither other windows nor input; screen capture
  and recording only through a capability (the system shows who is
  capturing).
- **One native UI library** (declarative, Rust) instead of two competing
  ones like GTK and Qt: a consistent look, accessibility and localization
  for everyone; the Windows layer (`user32`/`gdi32`) maps onto it.
- **Proposals for approval:**
  - **The window shows permissions:** an icon in the title bar tells which
    folders, network or camera the application can access right now;
    revoke with one click.
  - **Unforgeable system dialogs:** a permission or password dialog is
    drawn by the compositor in a separate layer (an application cannot
    paint over it or imitate it).
  - **Application work-in-progress survives a restart:** an application
    stores its UI state in its own dataset; after an update or a crash the
    windows come back.
  - **Smoothness as a requirement:** see [User interface responsiveness](#user-interface-responsiveness-plan) -
    frame deadlines, QoS and priority inheritance over IPC.

## Decisions for milestones 6a-6d (agreed)

| # | Question | Decision |
|---|---|---|
| 1 | terminal protocol | **our own native protocol** (typed records, tables, links, images, progress, prompts); VT100/pty only as an adapter for ported programs |
| 2 | shell language | **a new typed language, two notations of one language** (command and expression close to Rust, [design](shell-design.md)): no implicit word splitting, errors as values (code + description), `?` as in Rust; **a script carries a permission manifest**; POSIX `sh` separately for ported scripts |
| 3 | what an application sends to the compositor | **a layer tree + an accessibility tree** (the application draws content into buffers, the compositor does scrolling, transparency, animations; the window structure - roles, texts, state - is part of the protocol) |
| 4 | window decorations | **drawn by the system**; a custom look is possible, with the rules below |
| 5 | application action API | **in the GUI foundation** (6c): an application declares actions with typed parameters and required permissions in its manifest; the shell, file manager, assistant and agents call them |
| 6 | order | **6a input -> 6b terminal -> 6c GUI -> 6d AI**; AI primitives arrive along the way with FS v3 and the terminal |

### Input: two streams and synthetic input

- An application receives **physical keys** (a layout-independent code -
  games, shortcuts) and separately **inserted text** (from the layout and
  input method: dead keys, Chinese, emoji, dictation). Layouts and input
  methods are one system service, not code in every application.
- `inputd` delivers events to one seat owner (console, later the
  compositor), which distributes them to windows; `inputd` catches the
  secure key combination before it.
- **Synthetic input** (automation, remote desktop, an AI agent) only through
  an "inject input" capability; the system visibly shows that someone else
  controls the keyboard.

### Terminal: lazy undo and sessions

- **Lazy snapshot:** the shell asks the FS for a snapshot only at the first
  change the command's process makes - read-only commands cost nothing,
  every change can be undone.
- **Sessions belong to a service:** a running command survives closing or a
  crash of the terminal window and can be reattached (also remotely with
  the same structured protocol, not SSH with escape sequences).
- A pipe between processes carries **a binary record stream with a
  schema**; a program declares input and output in its manifest, the shell
  knows the types in advance (completion, checking, conversion to text only
  at the end).

### Custom window look (custom title bar)

An application sometimes wants its own window - tabs in the title bar (a
browser), its own bar (an editor, a player), a frameless window (a game, a
widget). It is possible, but the trusted elements stay with the system:

| Mode | Who draws | What the system keeps |
|---|---|---|
| **standard** (default) | the frame and title bar: the system | everything |
| **custom title bar** | the application also draws the title area (tabs, buttons, search) | a **trust badge** (application identity + what it may access right now: folders, network, camera) at a fixed place the application cannot paint over; the **window buttons** (close, minimize) are drawn by the system, the application only chooses side and colour; moving and resizing through **declared regions** (the application says "drag here", "a button here") - so the compositor works even when the application hangs |
| **frameless / full screen** | everything: the application | the badge appears when hovering at the edge and always on a permission change; the secure key combination and system dialogs always work |

- **System dialogs** (permission, password, payment) are drawn by the
  compositor in a separate layer above all windows, with the rest of the
  screen dimmed and a badge the application cannot render (only the system
  may draw above the dimming). An imitation inside an application window
  therefore always looks different.
- The title area carries the accessibility tree like the rest of the
  window, so a custom title bar is as accessible as the standard one.
- macOS (content up into the title bar, buttons drawn by the system) and
  Windows (extending content into the frame) do something similar; in
  addition we have the permission badge and declared regions instead of
  hit testing in the application.

## Input: keyboard and mouse (plan)

- An **input service** (`inputd`) in userspace: device drivers send events
  (key, motion, button, wheel, touch) through one protocol over a channel;
  the service translates them by keyboard layout and delivers them **only
  to whoever has focus** (terminal, compositor). No application reads input
  globally (protection against keyloggers).
- **Drivers, in order:** virtio-input (keyboard, mouse, tablet) in QEMU -
  PS/2 as a fallback on older hardware - USB HID through our own xHCI
  driver (the USB stack is a big separate piece of work).
- **Layouts:** Czech and English in the first version, diacritics
  composition (dead keys), Unicode input.
- **A secure key combination** (like Ctrl+Alt+Del on Windows): only the
  system captures it, an application cannot fake it - for unlocking,
  confirming permission dialogs and the task manager.

## Shell and smart terminal (proposal)

> Agreed decisions: [Decisions for milestones 6a-6d](#decisions-for-milestones-6a-6d-agreed).

Goal: a terminal more comfortable than today's ones that uses what our FS
and capabilities can do.

| Feature | What it does | Built on |
|---|---|---|
| **structured output** | commands return tables and objects (file, process, network connection), not text; filtering, sorting and columns without `grep`/`awk`; conversion to text only for display | a protocol between the shell and programs (a typed stream) |
| **undo of every command** | before a command that changes files, a cheap snapshot of the affected dataset; "undo the last command" | snapshots, change journal |
| **"show what will happen"** | a dangerous command (mass deletion, a script, an installation) runs in a clone of the folder, the terminal shows the diff, it is taken over after confirmation | clones, diff, taking over changes ("try on the side") |
| **permissions at launch** | the shell knows from the manifest what a command needs (a folder, network) and passes only that; a new demand = a prompt | capabilities |
| **type-aware completion** | knows an argument is a path to a `.toml` file, a PID or a snapshot name and completes only valid values | argument description in the program manifest |
| **explained errors** | on an error it shows the code, its meaning and what to do (from the error and message docs) | `docs/errors.md`, `messages.md` |
| **history with context** | command history with working folder, result and duration; searchable also by content | journal, index |
| **output blocks** | every command is a separate block (collapse, copy, rerun, compare two runs) | the terminal, not the shell |
| **assistant** (optional) | "find large files older than a year" -> a suggested command, run only after confirmation and in preview | the AI service (below) |

Additional proposals for approval:

- **A lock on dangerous operations:** a command that would delete or
  overwrite more than N files outside temporary folders always runs through
  "show what will happen" - even in real mode one confirmation is enough,
  but never blindly.
- **Sessions survive a terminal restart:** a running command belongs to the
  session service, not the window; after a crash or closing the window you
  can come back (like tmux, but built in).
- **Compatibility:** a classic text mode for scripts and POSIX tools (C/C++
  from the languages milestone) - structured output is an extension, not a
  replacement.

## AI in the system (proposal)

> Agreed order and action API: [Decisions for milestones 6a-6d](#decisions-for-milestones-6a-6d-agreed).

The kernel knows nothing about AI. The system does provide **primitives
that make an AI agent safe**, and optional services on top of them.

| Primitive | Why for an agent |
|---|---|
| a capability to one specific folder only | the agent sees nothing the user did not give it |
| working in a clone, the result as a diff | the agent's changes are taken over only after review, or discarded in one step |
| conditional save (expected revision) | the agent does not silently overwrite a change by the user or another tool |
| audit and journal with the application identity | it is visible what the agent read and changed; its changes can be rolled back selectively |
| quotas and priorities (QoS, I/O) | the agent does not flood the disk or CPU, the UI stays smooth |
| network only through a capability | the agent cannot send data out without permission |

Services on top (ordered by usefulness):

1. **Model service** (`aid`): local inference (CPU, later GPU/NPU),
   scheduling compute between applications, one API; cloud models only as
   the user's choice and only through a network capability.
2. **Semantic search:** an index of file content (vectors) as a service on
   top of the change journal, respecting permissions (an agent finds only
   what it may read).
3. **Assistant** in the terminal and file manager: suggests commands and
   actions, performs them through "show what will happen".
4. **Agent sessions:** running an agent on a project = automatically a
   clone, a capability to it only, audit on, a diff to take over at the
   end.

⚠️ **Risks (stated up front):**

- **Injected instructions** in processed data (a document or web page that
  "orders" the agent to do something): so an agent never gets more
  permissions than the user explicitly chose, and irreversible actions are
  confirmed by a human.
- **Privacy:** local models are the default; sending data to the cloud is a
  visible choice.
- **Performance:** models are large; they run in `background` QoS with a
  memory quota.

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

aarch64 is ported per the milestone plan (milestone 9): QEMU `virt` first,
then Raspberry Pi. The kernel already has an explicit [architecture
interface](#architecture-interface), so the port means writing its
implementation in `kernel/src/arch/aarch64/` (the skeleton exists).

## Architecture interface

Goal: **porting to another architecture = writing a new implementation of
one interface**, not rewriting half of the kernel. Everything that depends on
the CPU or its surroundings (interrupts, timers, paging, CPU startup, IOMMU)
lives in `kernel/src/arch/<arch>/`, and the rest of the kernel talks to it
only through a fixed contract. Status: x86_64 is fully behind the
interface, **aarch64 is only a skeleton so far** (it compiles; at run time
it stops at the first `todo!`).

> **Change (2026-11-06):** previously the kernel called `arch::*`
> functions that existed only for x86_64, and several places outside
> `arch/` knew x86 directly (ACPI tables MADT/HPET/DMAR/IVRS in the generic
> `Platform`, CPU startup through pages below 1 MiB, interrupt vectors
> 48..239, TSC synchronization, interrupt handlers calling the scheduler
> directly). **Why:** without an explicit contract, an aarch64 port would
> mean hunting hidden dependencies across the whole kernel. Now the contract
> is a set of traits, the compiler checks completeness, and
> `cargo xtask test` verifies that the kernel also builds for aarch64.

<figure class="diagram">
<svg viewBox="0 0 760 360" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Architecture interface">
<defs>
<marker id="ae1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker>
<marker id="ae2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker>
</defs>
<rect class="box" x="10" y="10" width="560" height="56" rx="6"/>
<text class="t b" x="290" y="33" text-anchor="middle">generic kernel</text>
<text class="m" x="290" y="53" text-anchor="middle">sched, mm, object, syscall, smp, acpi (RSDP, XSDT, MCFG), log</text>
<path class="ln" d="M290 66 L290 96" marker-end="url(#ae1)"/>
<rect class="acc" x="10" y="98" width="560" height="50" rx="6"/>
<text class="t b" x="290" y="119" text-anchor="middle">facade crate::arch</text>
<text class="m mono" x="290" y="138" text-anchor="middle">arch::now_ns(), arch::context::switch, arch::paging::PageTable, ...</text>
<path class="ln" d="M290 148 L290 176" marker-end="url(#ae1)"/>
<text class="m" x="300" y="167">static dispatch to arch::Current</text>
<rect class="box" x="10" y="178" width="560" height="62" rx="6"/>
<text class="t b" x="290" y="198" text-anchor="middle">contract arch::api (traits, impl Architecture)</text>
<text class="m mono" x="290" y="216" text-anchor="middle">Cpu, Interrupts, Threads, Time, Ipi, Memory,</text>
<text class="m mono" x="290" y="232" text-anchor="middle">DeviceIrq, Iommu, Smp, PlatformInfo, Diagnostics</text>
<rect class="ok" x="10" y="276" width="270" height="62" rx="6"/>
<text class="t b" x="145" y="298" text-anchor="middle">arch/x86_64 (done)</text>
<text class="m" x="145" y="318" text-anchor="middle">GDT/IDT, APIC, TSC+HPET, VT-d, AMD-Vi, SIPI</text>
<rect class="warn" x="300" y="276" width="270" height="62" rx="6"/>
<text class="t b" x="435" y="298" text-anchor="middle">arch/aarch64 (skeleton)</text>
<text class="m" x="435" y="318" text-anchor="middle">GICv3, generic timer, SMMUv3, PSCI: todo!</text>
<path class="ln dash" d="M145 276 L145 242" marker-end="url(#ae1)"/>
<path class="ln dash" d="M435 276 L435 242" marker-end="url(#ae1)"/>
<text class="m" x="152" y="262">implements</text>
<text class="m" x="442" y="262">implements</text>
<rect class="acc" x="600" y="98" width="150" height="142" rx="6"/>
<text class="t b" x="675" y="122" text-anchor="middle">crate::trap</text>
<text class="m" x="675" y="145" text-anchor="middle">interrupts, IPIs,</text>
<text class="m" x="675" y="162" text-anchor="middle">syscalls, user</text>
<text class="m" x="675" y="179" text-anchor="middle">exceptions</text>
<text class="m" x="675" y="205" text-anchor="middle">the only path</text>
<text class="m" x="675" y="222" text-anchor="middle">arch -&gt; kernel</text>
<path class="ln-acc" d="M145 338 L145 350 L675 350 L675 242" marker-end="url(#ae2)"/>
<path class="ln-acc" d="M435 338 L435 350"/>
<path class="ln-acc" d="M675 98 L675 38 L572 38" marker-end="url(#ae2)"/>
</svg>
<figcaption>The kernel calls the architecture only through the facade (down); the architecture calls the kernel only through <code>crate::trap</code> (up).</figcaption>
</figure>

### Three parts

1. **Contract** `kernel/src/arch/api/` - traits every architecture
   implements on an empty type (`X86_64`, `Aarch64`). The umbrella trait
   `Architecture` requires all groups, so a missing function or a different
   signature is a compile error, not a run-time error.
2. **Facade** `kernel/src/arch/mod.rs` - free functions and type aliases
   (`arch::now_ns()`, `arch::context::Context`, `arch::paging::PageTable`,
   `arch::iommu::Domain`) that statically forward to `arch::Current`
   (selected by `cfg(target_arch)`). No dynamic dispatch, no overhead.
3. **Reverse direction** `kernel/src/trap.rs` - the only place where
   architecture code calls into the kernel: timer, IPIs, device interrupts,
   syscalls, user-space exceptions, return to user space.

| trait | what the architecture provides | x86_64 | aarch64 (plan) |
|---|---|---|---|
| `Cpu` | CPU init, HW ID, per-CPU pointer, halt, breakpoint | GDT/TSS/IDT, local APIC ID, GS base | VBAR_EL1, MPIDR_EL1, TPIDR_EL1 |
| `Interrupts` | enable/disable interrupts, sleep until an interrupt | `sti`/`cli`/`hlt` | DAIF, `wfi` |
| `Threads` | thread context, switch, kernel stack, entering user space | callee-saved + FXSAVE, `iretq` | x19-x30 + v0-v31, `eret` |
| `Time` | clock in ns, cycle counter, one-shot timer, linking firmware time (`ns_from_counter_to_start`), hardware clock ([real time](time.md)) | TSC (calibrated by HPET), APIC timer, CMOS RTC | CNTVCT_EL0, CNTP; clock only from UEFI so far |
| `Ipi` | reschedule, halt, flush TLB | APIC IPI | GICv3 SGI |
| `Memory` | page tables (`PageTableOps`), TLB, cache for DMA | 4 levels, CR3, IPI shootdown | TTBR0/TTBR1, `TLBI ... IS` without IPI |
| `DeviceIrq` | device interrupt number range, MSI message | vectors 48..239, 0xFEE.. | LPIs from 8192 via ITS |
| `Iommu` | DMA translation domains (`IommuDomain`) | VT-d, AMD-Vi | SMMUv3 |
| `Smp` | starting other CPUs (`CpuStarter`) | INIT-SIPI, trampoline below 1 MiB | PSCI CPU_ON |
| `PlatformInfo` | ACPI tables the generic part does not know | MADT, HPET, DMAR, IVRS | MADT (GICC/GICD/ITS), GTDT, IORT, SPCR |
| `Diagnostics` | debug serial port, stack trace, QEMU exit, own tests | COM1, `rbp`, isa-debug-exit | PL011, `x29`, semihosting |

### Rules

- **The rest of the kernel** uses only the facade `crate::arch` (and types
  from `arch::api`). Never `arch::x86_64::...` - that module is private, so
  it is not even possible.
- **Architecture code** calls into the kernel only through `crate::trap`
  and generic services: `log`, `sync`, `error`, `fatal`, `mm` (physical
  allocator, direct map, MMIO mapping) and ACPI reading
  (`crate::acpi::read`). Never the scheduler, objects or syscalls directly.
- Everything in `trap` is called with interrupts disabled and with the
  interrupt already acknowledged in the controller; the exception is
  `trap::syscall`, which runs with interrupts enabled on the kernel stack.
- Shared types (`MapFlags`, `PageSize`, `MapError`, `PAGE_SIZE`) live once
  for all architectures in `arch::api::paging`.
- Fixed for all architectures (part of the ABI or the kernel layout):
  4 KiB kernel frames, the same address space layout (direct map from
  `0xFFFF_8000_0000_0000`, kernel in the last 2 GiB, shared
  `kernel/linker.ld`), a per-CPU structure with a self pointer at offset 0
  and architecture data at `CPU_LOCAL_OFFSET`, CPU fault numbers for a
  killed process = x86 vector numbers (aarch64 maps its exceptions onto
  them: abort 14, undefined instruction 6, ...), entry points use
  `extern "C"`.
- Beware of type aliases: on x86_64 `arch::paging::PageTable` is the
  concrete type from `arch/x86_64`, so its own methods outside the contract
  could be called. That compiles on x86, but `cargo xtask check --arch
  aarch64` (part of `cargo xtask test`) catches it.

### Examples

Generic code (scheduler) - facade only, no knowledge of x86:

```rust
use crate::arch::{self, interrupts};
use crate::arch::context::{self, Context, ThreadContext};

// new thread: the architecture prepares the context
let ctx = unsafe { Context::new(stack.top(), thread_start) };

// switch: address space, stack for entry from user space, registers
unsafe {
    arch::switch_user_space(root); // Some(process tables) / None
    arch::set_kernel_stack(&cpu.arch, top);
    context::switch(from, to);
}
arch::set_timer(Some(1_000_000)); // interrupt in 1 ms
```

Implementing one group in an architecture (x86_64):

```rust
impl api::Ipi for X86_64 {
    fn send_reschedule(cpu_hw_id: u32) {
        apic::send_ipi(cpu_hw_id, apic::VECTOR_RESCHEDULE);
    }
    fn send_halt(cpu_hw_id: u32) {
        apic::send_ipi(cpu_hw_id, apic::VECTOR_HALT);
    }
    fn send_tlb_flush(cpu_hw_id: u32) {
        apic::send_ipi(cpu_hw_id, apic::VECTOR_TLB_FLUSH);
    }
}
```

Path of a timer interrupt (reverse direction):

```text
APIC timer -> vector 32 -> idt::trap_dispatch (x86)
           -> apic::eoi()                    (acknowledge in the controller)
           -> trap::timer_interrupt()        (contract)
           -> sched::on_timer_interrupt()    (generic kernel)
           -> trap::after_interrupt()        (reschedule if needed)
```

Checking that the kernel builds for another architecture (also part of
`cargo xtask test`):

```bash
cargo xtask check --arch aarch64
```

### The aarch64 skeleton and what goes into it

`kernel/src/arch/aarch64/mod.rs` implements the whole contract; every
function that has to do something ends in a `todo!` describing what it
will contain. The first target is **QEMU `virt`** (GICv3, PL011, PSCI,
SMMUv3, UEFI from edk2 with ACPI), then Raspberry Pi. Planned modules:

| module | contents |
|---|---|
| `exceptions` | vector table (VBAR_EL1), saving registers, ESR -> ABI fault number |
| `gic` | GICv3: distributor, per-CPU redistributor, ITS for MSI (LPIs) |
| `timer` | generic timer: CNTVCT_EL0 as the clock, CNTP_TVAL_EL0 one-shot |
| `mmu` | 4 levels with 4 KiB granule, TTBR0 (process) and TTBR1 (kernel), MAIR, TCR |
| `context` | x19-x30, sp, FPU/SIMD v0-v31, FPCR/FPSR; switching |
| `user` | EL0 <-> EL1: SVC, ERET, SP_EL0 |
| `psci` | CPU startup via CPU_ON (HVC/SMC per ACPI FADT) |
| `smmu` | SMMUv3: stream table, domains |
| `acpi` | MADT (GICC, GICD, GIC ITS), GTDT, IORT (RID -> StreamID/DeviceID), SPCR |
| `pl011` | debug serial port |

Outside the kernel the port will also need: a loader for
`aarch64-unknown-uefi` (today x86_64: paging and the jump into the
kernel), xtask (QEMU `virt`, edk2 firmware for aarch64, ending tests via
semihosting), `libs/elf` (accepts only `EM_X86_64` today), `libs/libos` and
`libs/abi` (syscall instruction, `svc` instead of `syscall`) and
`user/user.ld`.

### Serious limitations and how they are solved

An ARM port brings things that do not exist on x86. The interface and the
kernel handle them up front, so the generic part does not change during
the port:

- **The kernel must not use FPU/SIMD.** A program's FPU state is saved only
  on a thread switch, not on every kernel entry. The kernel is therefore
  built without FPU: x86_64 without SSE, aarch64 for the
  `aarch64-unknown-none-softfloat` target (with plain `aarch64-unknown-none`
  the compiler would use NEON registers, e.g. for copying, and clobber the
  program's).
- **Non-coherent DMA (Raspberry Pi 4).** DMA memory
  (`VMO_CREATE_CONTIGUOUS`) is mapped into the process with
  `MapFlags::DMA`: the architecture picks normal caching (coherent
  platform) or an uncached mapping (Normal Non-cacheable). Where the kernel
  itself touches this memory through the direct map (zeroing,
  `VMO_READ`/`VMO_WRITE`, returning pages), it calls
  `arch::dma_sync_for_device` / `dma_sync_for_cpu`. Drivers need not do
  anything; on x86 the functions are empty.
- **Write ordering towards devices.** Drivers use the `libos::barrier`
  barriers (`dma_wmb`, `dma_rmb`, `mmio_wmb`) instead of `fence`; on ARM
  they are `dmb osh*` and `dsb st`, on x86 the same instruction as before.
  See [libos](libos.md#barriers-for-drivers).
- **MSI on GICv3 needs the device identity.** `INTERRUPT_CREATE` passes the
  device from the restricted Resource to the architecture, and
  `msi_message` can fail: without a device the program gets
  `INVALID_ARGS` (instead of a kernel crash). Drivers started by `devmgr`
  always have a device-restricted resource.
- **Kernel and process tables live in different registers on ARM** (TTBR1
  and TTBR0). Hence `PageTableOps::activate_kernel` (kernel tables only)
  and `arch::switch_user_space(Option<root>)` for processes (`None` =
  kernel thread, the lower half maps nothing).
- **The instruction address in the log** is named per architecture
  (`arch::PC_NAME`: `rip` / `pc`); symbolization in `cargo xtask` accepts
  both.

What cannot be solved, or is a deliberate decision:

- **Raspberry Pi 4 has no IOMMU (SMMU).** Driver DMA will not be restricted
  there - the same risk as x86 without an IOMMU (a buggy driver can
  overwrite the kernel via DMA); the kernel logs a warning at boot. It is a
  hardware property. RPi 5 does have an IOMMU (BCM2712), but it only
  translates display and multimedia in a single 4 GB window and everything
  outside the window passes unchecked - it gives no DMA protection for
  drivers, so an `Iommu` backend for it makes no sense.
- **The process page size is not an ABI constant.** The kernel itself works
  with 4 KiB frames, but a process's page size is a property of its address
  space (`VM_PAGE_SIZE`, `libos::vm::page_size()`, bounds 4 KiB to 64 KiB in
  `abi::vm`). It is 4 KiB everywhere today. On aarch64 the granule can be set
  per address space (TTBR0), so native processes can get 16 KiB (a few
  percent fewer TLB misses) and Windows layer processes 4 KiB (PE sections
  with different rights at 4 KiB). Programs are linked with segments aligned
  to 64 KiB (`user/user.ld`), so they work with any size. Apple M1/M2: the
  CPU supports 4 KiB, but their IOMMU (DART) only 16 KiB - the backend would
  align DMA memory to 16 KiB.


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
6. System: userspace drivers (virtio - done), FS (done), network (virtio-net,
   TCP/IP, DNS, TLS 1.3, HTTP client - services, access through a capability)
   - **6a Input:** input service, keyboard and mouse (virtio-input, PS/2,
     later USB HID), layouts, secure key combination - see [Input](#input-keyboard-and-mouse-plan)
   - **6b Shell and smart terminal:** console, structured output, command
     undo, "show what will happen", permissions at launch - see [Shell](#shell-and-smart-terminal-proposal)
   - **6c GUI:** virtio-gpu, compositor, native UI library, system dialogs -
     see [Graphics](#graphics)
   - **6d AI-ready:** primitives for safe agents (mostly from FS v3), model
     service, semantic search, assistant - see [AI in the system](#ai-in-the-system-proposal)
7. Languages and compilers (see [Languages and compilers](#languages-and-compilers-plan)):
   Rust (`std` for our OS), C (own libc on top of libos), C++
   (libc++/libunwind on top of libc), cross-compiled with clang from the
   host, then TinyCC in the system
8. Windows layer v2: user32/gdi32, GUI, DirectX via Vulkan
9. aarch64 port and the first ARM device (architecture interface and aarch64
   skeleton done, implementation remains)

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
