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
3. **Capabilities.** No root and no user who may do everything. A process
   can access only what it has a handle to. Services may add finer rules on
   top (FS permissions), see [Security](#security).
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
|  arch/x86_64, arch/aarch64          |  UEFI loader          |
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
| `MemoryObject` | memory pages **[implemented]**; lazy allocation and copy-on-write clones **[designed]** |
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
- `MemoryObject`: pages are allocated up front at creation **[implemented]**;
  lazy allocation on first access and copy-on-write clones **[designed]**
  (memory management is queued, see [Current deviations from the design](#current-deviations-from-the-design)).
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

> **Change from the earlier description:** the introduction said "no ACLs"
> and "file access = a handle to the directory". Since the FS permissions
> (F1d) the file system has grants and a system identity. The model below
> describes the layers as they really are.

**Three layers, each protecting something else:**

1. **Capabilities in the kernel** (the base). A process may do only what it
   has a handle with rights for. The kernel knows no users and no ACLs; no
   process has "everything" automatically.
2. **Permissions inside services.** A service managing shared data
   evaluates its own rules over the client's channel. In the FS these are
   the owner, allow-only grants with inheritance and `PROTECTED`, hiding
   what is inaccessible (`NOT_FOUND`) and `NOEXEC` on datasets, see
   [Permissions and session identity](file-system.md#permissions-and-session-identity).
   The client channel carries the identity: **the client cannot make it
   up**, it gets it from whoever created the session.
3. **The system identity (user 0).** FS checks do not apply to it. **Only
   the fsd control session held by init** (later the session manager) and
   host tools have it. Nobody can log in as it and no application gets it.
   It is the identity of the system itself, not an administrator account.

- A user is a userspace concept: a session = an identity + a set of
  capabilities (FS mounts, service channels). Init mints it
  (`CONNECT_AS`); login and accounts belong to the account service.
- Applications always run sandboxed; the manifest says which capabilities
  they need (network and budget so far, see [Application manifest](firewall.md#application-manifest)).
  Files outside their space come only from the user's choice in a dialog
  (powerbox, `GRANT`); trusted applications may use the working folders
  without a prompt
  ([Working folders](fs-v3-design.md#working-folders-and-protected-areas)
  **[designed]**).
- **Query budget** **[designed]**: prompt fatigue is a security problem
  (the user stops reading and clicks "allow"). The system therefore asks
  **only for an irreversible or unusual action**, the prompt is
  **rememberable** (once is enough) and it **never asks when an application
  launches** nor for access by intent (the Open/Save dialog). Everyday work
  runs without prompts and security is held by mechanisms (capabilities,
  snapshots, rollback), not by clicking. Examples:
  [Working folders](fs-v3-design.md#working-folders-and-protected-areas),
  [Firewall](firewall.md#asking-the-user), computer administration
  ([Administering the computer](accounts.md#administering-the-computer):
  only whole-computer actions).
- **Administering the computer** without root: an action for the whole
  computer is confirmed by an authorised user in a system dialog, creating
  a capability for that one action only (account service design). Computers
  managed by an organization (enrollment, company home-key backup, endpoint
  monitoring) are described in
  [Organization management](organization-management.md) **[designed]**.

**Critical components (TCB).** They hold a lot of authority, so a bug in
them is a security bug of the whole system. That is why they stay small,
tested, and divide their authority further:

| Component | Authority | How it is limited |
|---|---|---|
| kernel | everything | microkernel, drivers and services outside it |
| init (supervisor) | root capabilities, the fsd control session, mints identities and channels | gives each service only its part, serves nothing itself |
| `devmgr` | the root resource (PCI, I/O, interrupts) | a driver gets only its device; DMA through the IOMMU |
| `fsd` | the whole disk with the FS | checks rights by the channel identity, `diskguard` protects foreign disks |
| account service | user authentication, session requests | passwords only as a hash (Argon2id); init mints the session, not it |
| drivers with DMA | the memory the device writes to | IOMMU; without an IOMMU a DMA driver is part of the TCB |
- **Resources:** every process runs in a job with a byte budget of kernel
  memory and user memory. A parent divides its budget among children, a
  child cannot raise its limit. There are no fixed counts (handles,
  threads, ...), only limits given by formats (see
  [Jobs](syscalls.md#jobs-resource-budgets)).
- **The firewall** is part of the network service, so it cannot be
  bypassed. It consists of the application manifest, asking the user, a
  packet filter by network profile, audit and a DNS filter (see
  [firewall.md](firewall.md)).
- **Plan:** a launch gate (verifying a program signature, a more lenient
  mode by setting) and an antivirus scanning in isolated processes without
  network access.

### Household profiles and content policy (designed)

**[designed]** A shared device (TV, family tablet) needs a **household
profile**: a lightweight account without a password, optionally with a
PIN, picked with a remote or by touch. It has its own history,
recommendations and settings, but no encrypted home directory of its
own; sensitive data (mail, password vault) stays in full
[accounts](accounts.md).

The system enforces the **content policy**, not the apps:

| What | How |
|---|---|
| rating | an app states its rating in the manifest, media in its metadata; the profile has a maximum allowed rating |
| above the rating | content is **hidden** from a child profile (not shown in rows or search results) |
| exception | a parent allows one title once, with a PIN in a trusted dialog, only from local physical input |
| time limits and purchases | a system policy of the profile; the app asks for permission, it does not enforce them itself |

For a child **account** (not just a household profile) the extended
[Parental controls](parental-controls.md) apply: time limits, web
filtering, requests to the parent **[designed]**.

The app only gets "allowed / not allowed" and a reason for the text, not
the profile settings. Related to the [TV design](appearance.md#tablet-and-tv-design).

## System services

| Service | Task |
|---|---|
| init / supervisor | starts services, hands over devices attached at run time, restarts crashed ones (standalone services so far; see [Init](init.md)) |
| device manager | enumerates PCI, ACPI, device tree; starts drivers with their resources |
| drivers | virtio first, then NVMe, xHCI, HDA, ... |
| ACPI | AML interpreter; the kernel itself parses only static tables (MADT, HPET, MCFG) |
| storage + FS | block devices, partitions, file system (`fsd` done) |
| namespace | composes each process's view of the tree |
| input (`inputd`) | keyboard and mouse, layouts, secure attention shortcut (done, see [Input](input.md)) |
| compositor | display, windows |
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


Added 2026-11-09 (agreed, **[designed]**):

- **Who may install** **[designed]**: without an organization everybody on
  their own; an organization or the administration role may restrict the
  level (Freely / From the catalog / None, request for an application), see
  [The right to install applications](organization-management.md#the-right-to-install-applications).
- **No install scripts.** No app code runs during installation.
  Everything is a declaration in the manifest that the system registers:
  the icon in the bar and launcher, file types, palette and menu actions,
  context menu items, `window.instances`.
- **Several entry points in one package**: windows (GUI), **terminal
  commands** and background services. Terminal commands have the same
  metadata as the shell's built-in commands (description, arguments,
  examples), so `help`, Tab completion and the palette work; they appear
  in the terminal by themselves (no manual `PATH` or `sudo`), with the app
  icon in help. Permissions are shared by the package or narrowed per
  entry point.
- **App data**: the user's writable data in `/home/<user>/apps/<id>/`,
  the cache separately (the system may delete it any time), data shared
  by all users only with a permission. On an app update the FS snapshots
  its data; rolling back the app returns both program and data to the same
  point (no new data in the old program). Converting the data format to a
  new version is the app's job.
- **Runtimes** for big shared parts (Wine, library sets): versioned by
  **major version** (e.g. `wine 10`), an app asks for a compatible range.
  The system keeps only the latest patch release of each major version,
  security fixes reach all apps at once, an unused major version is
  removed. Blocks that apps bundle are deduplicated by the FS.
- **Multimedia as a system service**: FFmpeg is part of the system (an
  isolated process behind the playback and decoding service, see
  [Unified interfaces](device-interfaces.md)), apps do not link it and use
  the service's stable API. Different FFmpeg versions per app therefore do
  not arise; only apps that really need their own codecs (e.g. video
  editing) bundle them.

## Remote access (agreed 2026-11-09)

All **[designed]**. Image and input transport is the same for both modes
(the compositor sends damaged regions, input arrives as synthetic input
with a capability); the rules differ.

- **Assistance (support)** - someone helps in the **running session**:
  every time it is allowed by the person at the computer (one-time code /
  invitation, time limit); view only by default, control separately; a
  permanent indicator "Screen viewed / controlled by: …" with immediate
  termination; the secure attention shortcut always ends assistance;
  system dialogs (elevation, passwords) cannot be controlled remotely
  unless explicitly allowed.
- **Remote work** - the account owner on their own computer from
  elsewhere: instead of on-site consent, **sign-in** (password + second
  factor: FIDO2 or a paired device); the local screen is locked with "This
  computer is being used remotely" **without content**; taking over only by
  signing in (same account = the remote session disconnects); enabled in
  account settings (which accounts, devices, times; off by default); the
  audit (when, from where, how long - never content) is visible to the
  account owner and the administrator. If another user works at the
  computer, the remote session runs separately and they only see that a
  remote user is connected.
- **Stages**: (1) remote terminal - a shell session over TLS (like SSH),
  immediately useful for development on real HW; (2) a VNC (RFB) server in
  the compositor for compatibility with common clients; (3) an own
  efficient protocol (layer tree, GPU video encoding, link adaptation),
  possibly RDP.
- **Resolution**: remote work runs in a separate session on a virtual
  output whose resolution, scale and refresh rate **the client controls**
  (connecting from a 4K monitor = a 4K session; resizing the client window
  = a resolution change at run time). The local monitor is unaffected.
  Assistance shares the local session, so it has the local resolution and
  the client only scales the image.
- AI agents do not need the remote image - they use the Action API.

## Graphics

> Detailed design and status: [Graphics and windows (GUI)](gui.md).

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
| 2 | shell language | **a new typed language, two notations of one language** (command and expression close to Rust, [design](shell-design.md)): no implicit word splitting, errors as values (code + description) that propagate by themselves when unhandled and stop the script; **a script carries a permission manifest**; POSIX `sh` separately for ported scripts |
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

## Input: keyboard and mouse

> Done in 6a (virtio-input, `inputd`, US and CZ layouts, the
> Ctrl+Alt+Delete secure attention shortcut); details in [Input](input.md).


- An **input service** (`inputd`) in userspace: device drivers send events
  (key, motion, button, wheel, touch) through one protocol over a channel;
  the service translates them by keyboard layout and delivers them **only
  to whoever has focus** (terminal, compositor). No application reads input
  globally (protection against keyloggers).
- **Drivers, in order:** virtio-input (keyboard, mouse; done) in QEMU -
  PS/2 (done; internal laptop keyboards and the firmware's USB emulation) -
  USB HID through our own xHCI driver (phases 1-4 done: controller, hubs,
  HID with a report parser, hotplug, new device verification; see [USB](usb.md)) - USB HID through our own xHCI
  driver (the USB stack is a big separate piece of work).
- **Layouts:** Czech and English in the first version, diacritics
  composition (dead keys), Unicode input.
- **A secure key combination** (like Ctrl+Alt+Del on Windows): only the
  system captures it, an application cannot fake it - for unlocking,
  confirming permission dialogs and the task manager.

## Shell and smart terminal (proposal)

> Agreed decisions: [Decisions for milestones 6a-6d](#decisions-for-milestones-6a-6d-agreed).
>
> **Status (6b):** the language, lazy undo, "show what will happen", the
> dangerous operation lock, the `needs` manifest, completion, history, error
> explanations, the native protocol and the session service are done. They
> are described on the [Shell and terminal](shell-design.md#terminal-and-session-service)
> page, together with the [integration status](shell-design.md#integration-status)
> (init, `inputd`, framebuffer).

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

## Task overview (agreed)

Everything running in the background is in **one place**: terminal tasks
(`async`, `bg`), system services, long FS operations (scrub, deduplication,
check), application actions and later AI agents. Nothing runs "in a void".

| Part | What it does | Who does it |
|---|---|---|
| **kernel jobs** | a group of processes with budgets (memory, ports, DMA); the "who started whom" tree | done (network) |
| **task service `taskd`** | one task list: a human-readable name, **who started it** (user, script, application, service, agent), state, progress, estimate, usage from the job budget, actions (attach, pause, cancel, log); finished tasks stay for a while with their result and error | done (shell thread 6b) |
| **`tasks` command** | a structured table (`tasks \| where owner == "agent"`), `attach`, `cancel`; a task panel in the terminal, notification on completion or failure | done (shell thread 6b) |
| **GUI task manager** | a window with the same data, a "N tasks running" icon in the bar | GUI thread (6c) |

- **Registration:** tasks register themselves when created (shell,
  supervisor, services, action API). A process without registration still
  shows up, as "unknown process in job X" - nothing stays hidden.
- **Permissions:** a user sees their own tasks; others' and system tasks only
  with a right. Tasks started by an AI agent are clearly marked (see
  [AI in the system](#ai-in-the-system-proposal)).

### Implementation status (done)

```text
shell (shsess) ──producer──▶ taskd ◀──producer── init (services, apps)
fsd (scrub, maintenance) ──▶ │ ▲                  │ watch: app jobs
                             ▼ │ client           ▼
          tasks, panel (Ctrl-T) · GUI task manager · bar icon
```

- **The `taskd` service** (`user/taskd`, core `libs/taskcore` with host
  tests, protocol `libs/taskproto`): one task list in memory. Finished
  tasks stay with their result or error: at most the last 100 and at most
  30 minutes. Results of important operations stay elsewhere too (the
  terminal block, the service log, the FS journal), so taskd does not write
  finished tasks to disk.
- **Only init mints channels** (the taskd control channel): a client with an
  identity and a scope (own tasks / all; control: nothing, everything
  visible, or only its own - `client <identity> <all> <0|1|own>`), a
  producer with an
  identity (a task belongs to the producer's identity), a watched job. A
  shell session gets its channels through termd and the session registry
  (`RegMsg::TasksReq`, init mints them with the user's identity), fsd
  through a `tasks` message on its control channel.
- **Subscription:** a snapshot and events with a sequence number. A change
  of a task the client does not see arrives as `Removed(id)` - the numbers
  have no gaps and resuming from `since` works (the last 512 changes,
  otherwise a new snapshot).
- **Producers:** the shell (`async` tasks with their block and
  cancellation, `bg`), the supervisor (service state: running, restarting,
  gave up, stopped), init (applications with a window, including their
  job), fsd (scrub with progress and result, defragmentation and
  deduplication when idle). taskd forwards an action (cancel) to the
  producer only when the task supports it and the caller may.
- **Unknown process:** every 2 s taskd walks the processes of the watched
  jobs (syscall `JOB_PROCESSES`, CPU time and job memory). A process in a
  job that no running task reports is a task of origin `unknown`; a task
  with a job without live processes finishes.
- **The GUI task manager** (`gui-tasks`) is started by init in the normal
  run with a client channel that sees all tasks (like the tasks icon in the
  bar) and may control only the user's tasks. **[implemented]**
- **Shell:** `tasks` returns a table (`tasks | where owner == "service"`),
  `tasks cancel|pause|resume <id>`, `tasks log <id> -n 50`. In the window a
  task panel (Ctrl-T: arrows, `c` cancel, `p` pause, `r` resume), "tasks N"
  in the status line and a notice on completion or failure.

⚠️ **Limits:** no producer supports pausing yet (taskd returns
`NOT_SUPPORTED`); processes in
a nested job of an application are unknown (only processes directly in its
job belong to the task).

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

### Upper tier from Wine, lower tier ours

**Status: [designed]** (decided 2026-11-13, replaces the original plan of
our own `kernel32`/`ntdll`).

<figure class="diagram">
<svg viewBox="0 0 780 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Windows layer tiers">
<defs><marker id="wv2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="10" width="760" height="44" rx="6"/>
<text class="t b" x="390" y="37" text-anchor="middle">Windows applications (.exe, .dll) in their own directory with a virtual C:</text>
<rect class="ok" x="10" y="74" width="760" height="56" rx="6"/>
<text class="t b" x="390" y="97" text-anchor="middle">upper tier: Wine DLLs unchanged (PE)</text>
<text class="m" x="390" y="117" text-anchor="middle">kernel32 · kernelbase · user32 · gdi32 · comctl32 · ucrtbase · ... · DXVK (d3d9-11 -> Vulkan)</text>
<rect class="acc" x="10" y="150" width="760" height="70" rx="6"/>
<text class="t b" x="390" y="173" text-anchor="middle">lower tier: ours (Rust)</text>
<text class="m" x="390" y="193" text-anchor="middle">PE loader · lower part of ntdll · layer service instead of wineserver</text>
<text class="m" x="390" y="210" text-anchor="middle">Wine drivers: graphics → compd, audio → audiod</text>
<rect class="box" x="10" y="240" width="760" height="50" rx="6"/>
<text class="t b" x="390" y="262" text-anchor="middle">libos and system services</text>
<text class="m" x="390" y="280" text-anchor="middle">handly · wait_many · Port · MemoryObject · AddressRegion · Job · exception channel · fsd · netd · compd · audiod</text>
<line class="ln" x1="390" y1="54" x2="390" y2="70" marker-end="url(#wv2)"/>
<line class="ln" x1="390" y1="130" x2="390" y2="146" marker-end="url(#wv2)"/>
<line class="ln" x1="390" y1="220" x2="390" y2="236" marker-end="url(#wv2)"/>
</svg>
<figcaption>Win32 is adopted from Wine; our own work is only at the boundary with the system.</figcaption>
</figure>

Win32 is tens of thousands of functions and Wine is thirty years of work
on them. Our own `kernel32` or `user32` would be a dead end. Today Wine
builds almost all its libraries as ordinary Windows DLLs (PE) that talk to
the system below only through a narrow interface: the lower part of
`ntdll` and the `wineserver` service. **We write this interface ourselves
in Rust on top of our objects** (table above) and adopt the rest unchanged:

- **upper tier (adopted, LGPL):** Wine DLLs, `ucrtbase`, DXVK
  (Direct3D 9-11 on Vulkan); runs as an ordinary process, the license is
  fine,
- **lower tier (ours):** PE loader, the lower part of `ntdll` (Wine system
  calls onto our handles, `wait_many`, `Port`, `MemoryObject`), a **layer
  service** instead of `wineserver` (state shared between the processes of
  one application: named objects, windows, registry), a Wine graphics
  driver on top of `compd` and an audio driver on top of `audiod`,
- **what we do not do:** all of Wine on top of a POSIX layer (like the
  macOS port) - it would need an almost complete Unix (signals, `mmap` at
  fixed addresses, ptrace) that we do not want to build for this.

**Prerequisites** (Wine will not start without them):

1. Rust std phase 2 and the C library (`cstd`) - tools and building Wine.
2. Memory management: reserving address space at fixed addresses, demand
   paging, CoW, W^X with a "may JIT" permission in the manifest (.NET,
   browsers).
3. TEB via the GS register on x86_64: the kernel uses GS for per-CPU data
   today (`swapgs`), this needs a clean design.
4. Exception delivery (SEH) on the exception channel and suspending a
   thread with reading and writing its context.

**Steps:**

1. **v1 (milestone 5):** PE loader and the lower `ntdll`, console .exe
   (e.g. Wine `cmd`, small tools), the terminal through a VT adapter.
2. The layer service (`wineserver`) - several processes of one
   application, named objects, registry.
3. **v2 (milestone 8):** GUI through the graphics driver on top of
   `compd`, audio.
4. DXVK and games (Vulkan: Venus in QEMU, GPU drivers on hardware).

**Limitations:** x86_64 only (ARM would need x86 emulation, e.g. FEX or
box64 - much later, stated explicitly as a limitation per the
two-architecture rule); 32-bit applications only through the WoW64 mode
of new Wine.

### Proton

Proton (Valve) is not a different layer than Wine, it is **a Wine
distribution for games**: its own Wine branch with patches, DXVK
(Direct3D 9-11 on Vulkan), VKD3D-Proton (Direct3D 12 on Vulkan) and faster
synchronization (fsync on futexes). For us: the upper tier can come from
plain Wine (applications) or from Proton's Wine (games), the lower tier is
the same. We adopt DXVK and VKD3D-Proton either way and map
synchronization to `wait_many` and the futex. Steam would run as the
Windows client in Wine. **Risk:** games with anti-cheat (EasyAntiCheat,
BattlEye) often refuse unknown systems - vendor policy, not technology.

## Adopting existing software: order

**Status: [designed]** (decided 2026-11-14). What must be done to adopt
existing programs (FFmpeg, Wine/Proton, SDL, a browser). Each step builds
on the previous one:

| # | Step | Why | Status |
|---|---|---|---|
| 1 | Rust std phase 2: processes, pipes, network ([Rust std](rust-std.md)) | Rust applications unchanged, verified on existing crates | in progress |
| 2 | C library `cstd` and a POSIX layer: `malloc`, `pthread`, files, sockets, time, `stdio`, math | nothing in C/C++ without it | [designed] |
| 3 | Memory management: `mmap`, reserving address space at fixed addresses, demand paging, CoW, W^X with a "may JIT" permission | Wine, browsers, .NET | [designed] |
| 4 | Dynamic linking: loading shared libraries (ELF `ET_DYN`) | Wine and most C/C++ projects | [designed] |
| 5 | Exception delivery (SEH, signals) and suspending a thread with reading and writing its context | Wine, debuggers | [designed] |
| 6 | SDL2/3 on top of the compositor and `audiod` | one port opens games and multimedia apps | [designed] |
| 7 | FFmpeg as an isolated process (LGPL), then a player | multimedia | [designed] |
| 8 | Windows layer (Wine/Proton on our lower tier, see above) | Windows applications and games | [designed] |
| 9 | Vulkan on real hardware: Intel natively, AMD/NVIDIA through LinuxKPI | DXVK/VKD3D and games on hardware (Venus in QEMU for now) | [designed] |

Steps 2-5 can partly run in parallel (memory is kernel, `cstd` is
userspace), but `cstd` needs `mmap` from step 3 for most real programs.

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

## Text and line endings (agreed 2026-11-08)

The line ending in the whole system is **LF (`
`)** - service output,
logs, configuration, protocols, sources. It is the standard of Rust, git,
the web, JSON and Unixes; CRLF would mean conversions everywhere else.

1. **Only `
` inside the system.** **[designed]** as a rule; new code
   follows it.
2. **Tolerant reading**: tools accept `
` too (a file from Windows,
   pasted text) as one line ending and do not show a lone `
` at the end
   of a line as garbage.
3. **Files are never rewritten silently**: the editor keeps the style of
   the opened file; conversion only on request (a command, an option when
   saving).
4. **Terminal**: `
` = new line and return to the start (like modern
   terminals); a lone `
` returns the cursor to the line start (progress
   indicators).
5. **Exchange with the world**: `osfs` (sharing the FS with Windows) and
   git do not convert line endings; conversion is optional in a tool.
6. **Structured shell output** (tables, records) carries no line endings;
   they appear only when a value is turned into text for display.

### Text encoding

The text encoding of the whole system is **UTF-8 without a BOM** (Rust,
file names - normalized to NFC, protocols, logs, configuration, terminal).
**[implemented]** for names and internal text, reading rules
**[designed]**:

- **Tolerant reading**: a BOM at the start of UTF-8 is skipped; UTF-16 (by
  BOM, e.g. files from Windows) is converted when read; invalid bytes are
  shown as the replacement character `U+FFFD`, nothing crashes.
- **Files are never converted silently**: the editor keeps the encoding of
  the opened file; conversion only on request (`convert`).

## Code conventions

See [building.md](building.md#code-conventions). Code comments are in Czech,
documentation is in Czech and English (`docs/cs`, `docs/en`).

## Third-party code and license

- Project license `MIT OR Apache-2.0`.
- Anything may be reused (Mesa, DXVK, VKD3D, ...) where writing our own does
  not make sense. Long-term we aim for our own components.
- The system font Tuffy (Thatcher Ulrich) is public domain
  (`libs/font/fonts/Tuffy.ttf`); the terminal font Noto Sans Mono is
  under the OFL (crate `noto-sans-mono-bitmap`).

## Platforms

1. QEMU x86_64 + OVMF (main development)
2. real x86_64 PCs
3. QEMU aarch64 `virt`
4. Raspberry Pi, PinePhone (Pro)

aarch64 runs in QEMU `virt` (milestone 9, see [the aarch64
implementation](#the-aarch64-implementation-qemu-virt)); Raspberry Pi comes
later as another platform of the same architecture.

## Architecture interface

Goal: **porting to another architecture = writing a new implementation of
one interface**, not rewriting half of the kernel. Everything that depends on
the CPU or its surroundings (interrupts, timers, paging, CPU startup, IOMMU)
lives in `kernel/src/arch/<arch>/`, and the rest of the kernel talks to it
only through a fixed contract. Status: the interface is implemented by
**x86_64 and aarch64** (QEMU `virt`); both architectures pass all tests.

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
<text class="m mono" x="290" y="232" text-anchor="middle">DeviceIrq, LegacyIo, Iommu, Smp, PlatformInfo, Diagnostics</text>
<rect class="ok" x="10" y="276" width="270" height="62" rx="6"/>
<text class="t b" x="145" y="298" text-anchor="middle">arch/x86_64 (done)</text>
<text class="m" x="145" y="318" text-anchor="middle">GDT/IDT, APIC, TSC+HPET, VT-d, AMD-Vi, SIPI</text>
<rect class="ok" x="300" y="276" width="270" height="62" rx="6"/>
<text class="t b" x="435" y="298" text-anchor="middle">arch/aarch64 (QEMU virt)</text>
<text class="m" x="435" y="318" text-anchor="middle">GICv3 + ITS, generic timer, SMMUv3, PSCI</text>
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

| trait | what the architecture provides | x86_64 | aarch64 |
|---|---|---|---|
| `Cpu` | CPU init, HW ID, per-CPU pointer, halt, breakpoint | GDT/TSS/IDT, local APIC ID, GS base | VBAR_EL1, MPIDR_EL1, TPIDR_EL1 |
| `Interrupts` | enable/disable interrupts, sleep until an interrupt | `sti`/`cli`/`hlt` | DAIF, `wfi` |
| `Threads` | thread context, switch, kernel stack, entering user space | callee-saved + XSAVE/FXSAVE, `iretq` | x19-x30 + v0-v31, `eret` |
| `Time` | clock in ns, cycle counter, one-shot timer, linking firmware time (`ns_from_counter_to_start`), hardware clock ([real time](time.md)) | TSC (calibrated by HPET), APIC timer, CMOS RTC | CNTVCT_EL0, CNTP; clock only from UEFI so far |
| `Ipi` | reschedule, halt, flush TLB | APIC IPI | GICv3 SGI |
| `Memory` | page tables (`PageTableOps`), TLB, cache for DMA | 4 levels, CR3, IPI shootdown | TTBR0/TTBR1, `TLBI ... IS` without IPI |
| `DeviceIrq` | device interrupt number range, MSI message | vectors 48..239, 0xFEE.. | LPIs from 8192 via ITS |
| `LegacyIo` | I/O ports and legacy interrupt lines (ISA IRQ) for on-board devices without PCI ([PS/2](input.md#the-ps2-driver)) | `in`/`out`, IOAPIC (MADT overrides) | `NOT_SUPPORTED` |
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

### The aarch64 implementation (QEMU virt)

> **Change (2026-11-07):** the `todo!` skeleton became a working
> implementation for QEMU `virt`. **Why:** milestone 9 - a second
> architecture proves the interface really is enough; the generic kernel
> did not change for the port (only `init` tests got aarch64 branches and
> programs are linked with one script from xtask).

The whole system runs on aarch64: the loader under UEFI (edk2), the kernel
on 4 CPUs, device interrupts via the ITS, DMA translation via SMMUv3,
drivers, the file system and all tests (`cargo xtask test --arch aarch64`).
Modules in `kernel/src/arch/aarch64/`:

| module | contents |
|---|---|
| `exceptions` | vector table (VBAR_EL1), register frame, ESR -> ABI fault number, syscall `svc #0` |
| `gic` | GICv3: distributor, per-CPU redistributor, SGIs (IPIs), timer PPI, LPI tables |
| `its` | ITS: command queue (MAPD, MAPC, MAPTI, INV, SYNC), MSI message = GITS_TRANSLATER + event |
| `timer` | generic timer: CNTVCT_EL0 as the clock, EL1 virtual timer (PPI 27) one-shot |
| `mmu` | 4 levels, 4 KiB granule, TTBR1 kernel / TTBR0 process (EPD0 for kernel threads), ASID 0 + nG |
| `context` | x19-x30, sp and the whole FPU/SIMD state (q0-q31, FPCR, FPSR); entering EL0 via `eret` |
| `smp` | CPU startup via PSCI CPU_ON (HVC/SMC per FADT), a stub enables the MMU with the kernel tables |
| `smmu` | SMMUv3: two-level stream table, stage 1 STE with a CD and its own ASID per device |
| `acpi` | MADT (GICC, GICD, GICR, GIC ITS), FADT (PSCI), IORT (SMMUv3, RID -> StreamID) |
| `pl011`, `semihosting` | debug serial port (address from SPCR via the loader), test exit in QEMU |

Boot on aarch64:

1. **The loader** (`loader/src/arch/aarch64`) runs under UEFI in EL1 or EL2
   (real hardware), finds the PL011 in ACPI SPCR (maps it identity as a
   device) and cleans the kernel image to memory. From EL1 it turns the MMU
   off, sets MAIR, TCR, TTBR0 = TTBR1 and jumps to the kernel in the upper
   half; from EL2 it sets the EL1 registers directly, prepares EL2 without
   traps (EL1 in AArch64, timer, FPU, GIC, no stage 2) and drops into the
   kernel in EL1 via `eret`.
2. **The kernel** sets up the vector table, takes over the tables (TTBR1),
   disables the lower half, reads ACPI and enables GICv3, the ITS and the
   timer.
3. **Other CPUs**: PSCI CPU_ON at the physical address of the `__ap_entry`
   stub (the page is temporarily identity mapped); the stub drops from EL2
   to EL1 if needed, loads its parameters, enables the MMU and jumps into
   the kernel.
4. **IOMMU**: SMMUv3 from IORT; from enabling on all streams are blocked, a
   device gets a domain only with a restricted Resource (like VT-d on x86).

Running and testing:

```bash
cargo xtask run --arch aarch64
```

```bash
cargo xtask test --arch aarch64
```

QEMU: the `virt` machine with GICv3 and ITS, `-cpu cortex-a76` (the same
core as Raspberry Pi 5), with EL2 (`virtualization=on`: the firmware runs in
EL2 as on real hardware and the loader and other CPUs drop to EL1), SMMUv3 (`--iommu
smmu`, default; `--iommu none` without DMA translation). Tests end via
semihosting (code 33 = success as on x86; zero is not used because QEMU also returns it on a plain shutdown). Under TCG emulation it runs roughly 2x
slower than x86 under TCG.

Differences from x86 worth knowing:

- **Programs have FPU and NEON** (target `aarch64-unknown-magnumopus`,
  hard-float, like x86 with SSE); the kernel stays without FPU and saves
  the threads' FPU state on switches (`FPU: ...` tests in the `procesy`
  suite).
- **MSI needs the device identity** (DeviceID for the ITS): `INTERRUPT_CREATE`
  without a device returns `INVALID_ARGS`. The ITS page (GITS_TRANSLATER)
  is mapped into the device's SMMU domain, otherwise the SMMU would block
  its MSIs.
- **No hardware clock** (PL031 is not in ACPI, the kernel does not call the
  EFI RTC at run time): time comes only from UEFI at boot and `set` does not
  write a clock.
- **IORT**: only one root complex mapping is read (segment 0); the ITS
  assumes DeviceID = RID (true on QEMU virt).
- **A kernel stack overflow** loops on the guard page (x86 has a separate
  IST stack for double faults); it shows up as a hang, not memory
  corruption.

Outside the kernel the port changed: `loader` (the `arch/aarch64` part),
`xtask` (`--arch`, `arch.rs`, `qemu.rs`), `libs/elf` (accepts the machine
of the build target), `libs/libos` (`svc` and the `_start` entry),
`libs/boot-info` (version 4: PL011 and device tree from the loader).

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
   top of libos (threads, TLS, synchronization, files, processes, time);
   [Rust std](rust-std.md): phase 1 (allocation, threads, TLS, files, stdio)
   **[verified in QEMU]**, network and processes **[designed]**.
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
| framebuffer console in the kernel | same | draws until the screen owner takes over (`FB_CONTROL` `CLAIM`): without virtio-gpu `fbdisplay` under the compositor ([GUI](gui.md#displays-virtio-gpu-and-fbdisplay)), otherwise the rescue console; after `RELEASE` or a crash of the owner it draws again, a panic always does |
| `fbdisplay` (GOP framebuffer) | GUI on machines without virtio-gpu (VMware, Hyper-V, VirtualBox, PCs) | fixed firmware resolution, no acceleration and no vsync; replaced by native GPU drivers |

## Milestones

1. Boot: UEFI loader, kernel prints text (done)
2. Kernel basics: GDT/IDT, exceptions, physical allocator, paging, heap (done)
3. Processes: scheduler, SMP, threads, objects, handles, capabilities (done)
4. Userspace: syscalls, first process, IPC, libos, initfs (done)
5. Windows layer v1: PE loader and the lower `ntdll` under Wine DLLs, console .exe
   (after std phase 2, `cstd` and memory management - see [Windows layer](#windows-layer))
6. System: userspace drivers (virtio - done), FS (done), network (virtio-net,
   TCP/IP, DHCP, DNS, time synchronization `ntpd`, access through a
   capability, the firewall, TLS 1.3 and an HTTP/1.1 and HTTP/2 client
   done - see [network.md](network.md), [firewall.md](firewall.md) and
   [tls-http.md](tls-http.md); IPv6, own stack remain), jobs with resource
   budgets (done)
   - **6a Input (done):** input service, keyboard and mouse (virtio-input;
     PS/2, USB HID), US and CZ layouts, secure attention shortcut -
     see [Input](input.md)
   - **6b Shell and smart terminal:** console, structured output, command
     undo, "show what will happen", permissions at launch - see [Shell](#shell-and-smart-terminal-proposal)
     (language, session service and QEMU test done; input and framebuffer pending)
   - **6c GUI:** virtio-gpu, compositor, native UI library, system dialogs,
     action API - see [Graphics and windows](gui.md) (software compositing
     on virtio-gpu 2D, compositor, UI library, fonts, dialogs, actions and
     QEMU tests done; the virgl 3D backend remains)
   - **6d AI-ready:** primitives for safe agents (mostly from FS v3), model
     service, semantic search, assistant - see [AI in the system](#ai-in-the-system-proposal)
7. Languages and compilers (see [Languages and compilers](#languages-and-compilers-plan)):
   Rust (`std` for our OS), C (own libc on top of libos), C++
   (libc++/libunwind on top of libc), cross-compiled with clang from the
   host, then TinyCC in the system
8. Windows layer v2: GUI (user32/gdi32 from Wine) through a driver on top of
   `compd`, audio, DirectX via DXVK on Vulkan
9. aarch64 port and the first ARM device (architecture interface and QEMU
   `virt` done, Raspberry Pi remains)

## Current deviations from the design

What currently works more simply than the design describes (and why it is
fine for now):

- **Syscalls are synchronous** and the driver and FS protocols
  (`blockproto`, `fsproto` v1) copy data in channel messages; request
  queues in shared memory (io_uring style) come when performance needs them.
- **Memory objects allocate pages up front** (no lazy allocation, no CoW clones).
- **Only whole mappings can be unmapped.**
- **Syscalls return via `iretq`** instead of the faster `sysret`.
- **Programs have FPU and SIMD** **[verified in QEMU]** (FPU-1): all
  services, drivers and applications are built for the targets
  `x86_64-unknown-magnumopus` (baseline x86-64-v2) and
  `aarch64-unknown-magnumopus` (NEON) with the hard-float ABI; core and
  alloc from rust-src (`-Zbuild-std`), C code with the same ABI (`cbuild`).
  The kernel stays soft-float and saves the threads' FPU state (FPU-2:
  XSAVE with AVX and AVX-512, FXSAVE without XSAVE; [kernel.md](kernel.md)).
  The kernel stops an older CPU at start
  ([hardware-requirements.md](hardware-requirements.md)). AMX is not
  enabled yet.
- **IOMMU (Intel VT-d, AMD-Vi) only for DMA translation:** interrupt
  remapping and RMRR/IVMD regions are missing; on a machine without an
  IOMMU DMA is not restricted (the kernel logs it).
- **The TCP/IP stack is smoltcp** (behind the `stack` module of the `netd`
  service), our own stack replaces it later - see [network.md](network.md#plan).
- **The shell's change preview is an overlay in shell memory**, not a
  dataset clone: `fsproto` cannot do clones and diffs yet. Programs
  therefore do not run inside `preview`, and undoing their changes relies on
  a snapshot of the whole volume.
- **Shell background jobs run cooperatively** (when the shell is idle), not
  in their own threads.
- **Strict priority between QoS classes** (lower classes can starve).
- **Memory of finished threads** (kernel stacks) is reused but not returned
  to the free frames: the stack (and the job budget) is released right when
  the thread ends and the next thread gets its slot including the pages.
