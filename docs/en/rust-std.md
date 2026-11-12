# Rust std for applications

Applications for MagnumOpus will be written in ordinary Rust with `std`
(`fn main()`, `std::fs`, `std::thread`, `std::net`, crates from
crates.io). System services and drivers stay `no_std` on top of
[libos](libos.md). This page is the **design for approval (phase 0)**:
the approach, the build, the mapping of `std` onto libos and services,
what is missing in the kernel, and the impact on future plans.

> **Status:** everything is **[designed]**, there is no code yet. Open
> decisions are in [Decisions for approval](#decisions-for-approval).

## Goal and scope

- Targets `x86_64-unknown-magnumopus` and `aarch64-unknown-magnumopus`
  (`target_os = "magnumopus"`) **[designed]**.
- `std` on top of libos: allocation, threads, synchronization, TLS, time,
  files through `fsd`, network through `netd`, processes, arguments,
  environment variables, stdio **[designed]**.
- `cargo xtask` builds std (a sysroot) and the application and puts it
  into initfs **[designed]**.
- Out of scope: dynamic linking, `fork`, POSIX signals, porting rustc
  (only the impact, see [Impact on the future](#impact-on-the-future)).

## Approach

### Options

| | A: own PAL in a std fork (like Motor OS, Hermit) | B: `unix` family on top of our libc (like Redox) | C: no fork, own `no_std` facade |
|---|---|---|---|
| How | small glue in `library/std/src/sys/**` (`cfg_select!` arms for `target_os = "magnumopus"`), logic in our `osrt` crate | `target_family = "unix"`, std uses its unix PAL through the `libc` crate, POSIX comes from `cstd` | a `mostd` crate re-exporting `core`/`alloc` plus its own `fs`, `thread`... |
| crates.io crates | work when they only use `std`; code with `cfg(unix)` needs an arm for our OS | work, including `cfg(unix)` and `libc` users | do not work (they want a real `std`) |
| Work before the first `hello` | small: ~30 glue files + `osrt` | large: the std unix PAL needs hundreds of POSIX functions (pthread, mmap, stat, poll, signals) and support in the `libc` crate | smallest |
| Semantics | our objects directly (channels, handles, FS sessions) | POSIX emulation (fd numbers, `errno`, paths, permissions), does not fit the capability model | ours |
| Maintenance on a new nightly | rebase the glue (scope below) | smaller rebase, but also a fork of `libc` and a large libc | none |
| Performance | direct libos calls | every call through the libc layer | direct |

**Recommendation: A.** It is what upstream itself does (Motor OS and
Hermit live in `std` exactly like this: only switches and thin modules in
`std`, all logic in the external `moto-rt` / `hermit-abi` crate). B gives
better crate compatibility, but almost the whole POSIX libc would have to
be written first, and POSIX semantics (fd numbers, `fork`, signals) go
against the system design. C does not meet the goal (gix, the browser and
other crates want a real `std`). Compatibility with `cfg(unix)` crates is
handled one crate at a time (see [gix](#gix-for-the-fs-git-layer)).

Later (once the project is public and stable) it makes sense to get the
target upstream as tier 3, like Motor OS: the fork maintenance goes away
and crates (libc, getrandom, jobserver...) can add support themselves.

### Layers

<figure class="diagram">
<svg viewBox="0 0 780 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Layers of std for MagnumOpus">
<defs><marker id="rs1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="ok" x="10" y="10" width="370" height="50" rx="6"/><text class="t b" x="195" y="32" text-anchor="middle">applications with std</text><text class="m" x="195" y="50" text-anchor="middle">hello-std, gix, browser, media player</text>
<rect class="box" x="400" y="10" width="370" height="50" rx="6"/><text class="t b" x="585" y="32" text-anchor="middle">no_std services and drivers</text><text class="m" x="585" y="50" text-anchor="middle">fsd, netd, compd, drivers (unchanged)</text>
<path class="ln" d="M195 60 L195 88" marker-end="url(#rs1)"/>
<rect class="acc" x="10" y="90" width="370" height="60" rx="6"/><text class="t b" x="195" y="113" text-anchor="middle">library/std (fork = glue)</text><text class="m" x="195" y="131" text-anchor="middle">sys/*/magnumopus.rs: cfg_select! arms, thin wrappers</text>
<path class="ln" d="M195 150 L195 178" marker-end="url(#rs1)"/>
<rect class="acc" x="10" y="180" width="370" height="60" rx="6"/><text class="t b" x="195" y="203" text-anchor="middle">libs/osrt (rustc-dep-of-std)</text><text class="m" x="195" y="221" text-anchor="middle">start, heap, threads, futex, TLS, fs, net, stdio</text>
<path class="ln" d="M195 240 L195 268" marker-end="url(#rs1)"/>
<path class="ln" d="M585 60 L585 268" marker-end="url(#rs1)"/>
<rect class="box" x="10" y="270" width="760" height="50" rx="6"/><text class="t b" x="390" y="292" text-anchor="middle">libos (rt feature off for std) + fsproto, netproto, svcclient, abi</text><text class="m" x="390" y="310" text-anchor="middle">one source of truth for protocols, shared by std apps and services</text>
</svg>
<figcaption>std contains only glue; the logic lives in our osrt crate on top of libos and the service protocols, which no_std services share.</figcaption>
</figure>

- **`libs/osrt`** **[designed]**: the runtime for `std` (like `moto-rt`).
  `#![no_std]`, feature `rustc-dep-of-std` (depends on
  `rustc-std-workspace-core`/`-alloc` instead of `core`/`alloc`, like all
  std dependencies). It holds the entry point, heap, threads, futex, TLS,
  the FS and network clients and stdio. Logic is tested on the host like
  other `libs/*`.
- **libos** gets an `rt` feature (on by default) **[designed]**:
  `entry!`, `#[global_allocator]` and `#[panic_handler]` exist only with
  it. `std` provides those itself, so `osrt` uses libos without `rt`. The
  heap moves into a module usable without the `#[global_allocator]`
  attribute (the same allocator for services and std apps).
- **fsproto, netproto, svcclient, wire, abi** get a `rustc-dep-of-std`
  feature. The `spin` dependency in fsproto is replaced with our own lock
  (the `spin` crate lacks that feature). The protocols thus stay in **one
  source** for both services and `std`.

### What changes in std (the glue)

In the pinned nightly (`nightly-2026-11-05`), `target_os = "motor"` has an
arm in 32 files under `library/std/src`, Hermit in 34. Our estimate is
**~35 files**, mostly one line in a `cfg_select!` and a module of up to
~200 lines (`sys/fs/magnumopus.rs`, `sys/thread/magnumopus.rs`,
`sys/pal/magnumopus/mod.rs`...). Plus `library/std/Cargo.toml` (the
`osrt` dependency) and `std/src/os/magnumopus/` (extensions such as
`os::magnumopus::io::AsRawHandle`).

**Maintenance on a toolchain update:** the glue is a patch series over
`library/` from the `rust-src` component of the pinned nightly. A new
nightly = re-apply the patches; conflicts usually appear in `cfg_select!`
lists when upstream adds another platform (minutes). Once in a while
upstream restructures `sys/` (in 2025 `pal` was split into
`sys/{fs,thread,...}`); then rewriting the glue takes 1 to 2 days. The
logic in `osrt` does not change. The nightly is updated **only on
purpose and for the whole repository at once** (one toolchain for the
kernel, services and std).

## Toolchain and build

### Pinned nightly

The same as the rest of the repository: `rust-toolchain.toml`
(`nightly-2026-11-05`, the `rust-src` component is already listed)
**[designed]**. A second toolchain just for std would mean two copies of
`core` and two sets of compiler bugs. `rustc` must be exactly the version
the std sources belong to; xtask checks it (version from `rustc -vV`
against the version in the patch header).

### How xtask builds std

<figure class="diagram">
<svg viewBox="0 0 780 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Building the std sysroot">
<defs><marker id="rs2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="230" height="70" rx="6"/><text class="t b" x="125" y="45" text-anchor="middle">rust-src (rustup)</text><text class="m" x="125" y="64" text-anchor="middle">library/ of the pinned nightly</text>
<rect class="box" x="10" y="150" width="230" height="70" rx="6"/><text class="t b" x="125" y="175" text-anchor="middle">third_party/rust-std</text><text class="m" x="125" y="194" text-anchor="middle">patches + new sys/* files</text>
<path class="ln" d="M240 55 L288 100" marker-end="url(#rs2)"/>
<path class="ln" d="M240 185 L288 140" marker-end="url(#rs2)"/>
<rect class="acc" x="290" y="85" width="210" height="70" rx="6"/><text class="t b" x="395" y="110" text-anchor="middle">target/std-src</text><text class="m" x="395" y="129" text-anchor="middle">copy + git apply</text>
<path class="ln" d="M500 120 L548 120" marker-end="url(#rs2)"/>
<rect class="acc" x="550" y="20" width="220" height="70" rx="6"/><text class="t b" x="660" y="45" text-anchor="middle">target/sysroot</text><text class="m" x="660" y="64" text-anchor="middle">rlibs core, alloc, std, osrt</text>
<rect class="ok" x="550" y="150" width="220" height="70" rx="6"/><text class="t b" x="660" y="175" text-anchor="middle">application</text><text class="m" x="660" y="194" text-anchor="middle">cargo build --sysroot, initfs</text>
<path class="ln" d="M660 90 L660 148" marker-end="url(#rs2)"/>
</svg>
<figcaption>xtask prepares the std sources with our glue, builds a sysroot from them, and applications are then compiled against it.</figcaption>
</figure>

1. `cargo xtask std` **[designed]** copies `library/` from the `rust-src`
   of the pinned nightly into `target/std-src/<nightly>/` and applies the
   patches from `third_party/rust-std/` (`git apply`; a failure is a clear
   message naming the files that do not match). It adds the path to
   `libs/osrt` into `library/std/Cargo.toml`.
2. It builds the sysroot: `cargo build -p sysroot` in the `library/` copy
   with `RUSTC_BOOTSTRAP=1`, the JSON target and bootstrap-like flags
   (`-Zforce-unstable-if-unmarked`, `compiler-builtins-mem`,
   `panic_abort`). The resulting `.rlib` files go to
   `target/sysroot/<nightly>/lib/rustlib/<target>/lib`. It is rebuilt only
   when the patches, `osrt`, libos or the nightly change (a fingerprint
   file next to the sysroot).
3. An application is compiled with plain `cargo build --target <json>`
   and `--sysroot target/sysroot/<nightly>`; rust-lld links it with the
   `user/user.ld` script. `cargo xtask build` then adds the application
   to initfs (a list of std apps next to `USER_PROGRAMS`).

Why not `-Zbuild-std` directly: cargo always takes the std sources from
`rust-src` in the toolchain sysroot, and feeding it a patched copy is only
possible through internal variables or a custom toolchain via `rustup
toolchain link`. Our own sysroot (what Xargo used to do) is deterministic
and needs no internal cargo switches. **Risk:** std dependencies
(hashbrown, compiler_builtins...) are downloaded from crates.io on the
first build (then from the cargo cache); an offline first build fails.
This is checked first in phase 1 (a spike); if the custom sysroot hits a
wall, the fallback is `-Zbuild-std` with a toolchain via `rustup
toolchain link`.

### Target description (JSON)

| Field | x86_64 | aarch64 | Why |
|---|---|---|---|
| `llvm-target` | `x86_64-unknown-none-elf` | `aarch64-unknown-none` | OS-less ELF in LLVM |
| `os` / `target-family` | `magnumopus` / none | same | not `unix`, see options |
| floating point | SSE2 (hard-float) | NEON, `fp-armv8` (hard-float) | performance (FFmpeg, LLVM, browser); the kernel already saves thread FPU/SIMD state |
| `panic-strategy` | `abort` | `abort` | unwind later (see rustc) |
| `relocation-model`, linker | `static`, rust-lld, `user/user.ld` | same | the loader only handles `ET_EXEC` |
| `has-thread-local`, `tls-model` | yes, `local-exec` | yes, `local-exec` | native TLS (requires a kernel change) |
| `max-atomic-width` | 64 | 128 or 64 (depending on `lse`) | |

**Note:** services keep being built for `*-none` with soft-float (the
kernel and services use no SSE/NEON). std applications will be
**hard-float**. It is a different target, so `.rlib` files never mix;
only shared crates (libos, fsproto...) are compiled twice. AVX is not
possible yet (the kernel saves state with FXSAVE, AVX instructions end
with `#UD`); FFmpeg will need XSAVE in the kernel.

## Mapping std onto libos and services

Legend: **P1** = phase 1 (now), **P2** = next step, **Unsupported** =
returns `io::ErrorKind::Unsupported`, **kernel** = missing in the kernel,
**libos** = missing in libos/protocols.

| Part of std | On top of | When | What is missing |
|---|---|---|---|
| start (`main`, `exit`, `abort`) | `_start` in osrt reads the startup message, calls `main` through `lang_start`; `process_exit` | P1 | nothing |
| allocator (`System`) | libos heap (`sizeclass` + first-fit, grows with VMOs) | P1 | large allocations (>= 1 MiB) as their own VMO so memory is returned; P2 |
| `env::args` | process parameter block | P1 | **libos**: parameter protocol (below) |
| `env::var`, `set_var` | parameter block, changes kept in-process | P1 | same |
| `env::current_dir` / `set_current_dir` | path in osrt, relative paths joined in-process | P1 | same (initial directory) |
| `io::stdout/stderr` | byte-stream channel from the parameter block; without one the kernel log (`debug_log`) | P1 | **libos**: stream protocol (below) |
| `io::stdin` | stream channel | P2 | same + termd integration |
| `time::Instant` | `clock_monotonic` | P1 | nothing |
| `time::SystemTime` | `clock_realtime` (`abi::time::Reading`) | P1 | nothing |
| `thread::spawn`, `join`, `sleep`, `yield_now` | `libos::thread`, `time::sleep`, `thread_yield` | P1 | **libos**: stack guard (below) |
| `thread::park`, `Mutex`, `Condvar`, `RwLock`, `Once` | futex (upstream `sys/sync/*/futex.rs` unchanged) | P1 | **kernel**: futex (decision 2) |
| `thread_local!` | native ELF TLS (`#[thread_local]`), a TLS block per thread | P1 | **kernel**: thread register (decision 3) |
| `thread::available_parallelism` | CPU count | P1 | **libos**: CPU count query (missing today, otherwise Unsupported) |
| `random` (`HashMap` `RandomState`) | `random_get` | P1 | nothing |
| `fs::File` open/read/write/seek/`set_len`/`sync_all` | fsproto v2 (session handles, a file survives a rename) | P1 | nothing (FS session from the parameter block) |
| `fs::read_dir`, `metadata`, `create_dir`, `remove_*`, `rename` | fsproto (`LIST_PAGE`, `STAT`, `CREATE`, `REMOVE`, `RENAME`) | P1 | nothing |
| `fs::Permissions` | only `readonly` from FS rights | P2 | mapping of FS rights |
| symlinks, `hard_link`, `canonicalize` | `canonicalize` = path normalization; links Unsupported | P2 | FS v3 decides about links |
| `net::TcpStream`, `TcpListener`, `UdpSocket` | netproto (`tcp_connect`, `tcp_listen`, `udp_bind`), network session from the parameter block | P2 | nothing |
| `ToSocketAddrs` (DNS) | `Net::resolve` | P2 | nothing |
| `process::Command`, `Child`, `exit status` | read the ELF from FS, `libos::spawn_in` in its own job, parameter block for the child | P2 | **libos**: launching with a manifest (decision 4) |
| `io::pipe`, `Stdio::piped` | a channel pair with the stream protocol | P2 | same as stdio |
| `os_str`, `path` | UTF-8 bytes (like unix `os_str/bytes`), separator `/` | P1 | nothing |
| `backtrace` | Unsupported (a panic prints the message) | later | unwinder + symbols |
| `fork`, signals, `os::unix` | never | - | on purpose |

### Process parameter block (libos) **[designed]**

Today a parent passes handles with no agreed meaning (`kind::USER`,
per-program tags) and a program has no arguments, environment or stdio.
std needs one common place:

- a new handle kind in the startup message, `kind::PARAMS`: a read-only
  memory object with the **parameter block** (version, `argv`, `envp`,
  initial directory, a "name -> handle index" table),
- the handles the block refers to travel in the same startup message
  (`kind::USER` with a tag): `stdin`, `stdout`, `stderr`, the FS session,
  the network session, the channel to the parent,
- libos gets `spawn::Options::params(...)` for parents and block parsing
  for children; `no_std` programs (shell, init) can use it too.

A memory object instead of message data: arguments and environment are
not limited by the channel message size.

### Byte-stream protocol **[designed]**

`stdout`/`stderr`/`stdin` and `io::pipe` are channels with the simplest
protocol: every message = a chunk of bytes (up to the message limit),
closing the channel = end of stream. Back-pressure comes from the channel
queue limit (a write waits while the queue is full). The terminal (termd)
and the shell convert the stream to their native protocol; that is work
for the terminal thread and is coordinated with it.

### Stack guard **[designed]**

libos today maps a thread stack wherever the kernel puts it; another
mapping may lie right below it and an overflow silently overwrites it.
std threads have a 2 MiB default stack and deep recursion (parsers,
serde). Needed: reserving an address range without memory (`vm_reserve`)
or mapping with an unmapped page below the stack, so an overflow ends in
a page fault. Belongs in libos (and possibly the kernel).

## What is missing in the kernel

1. **Futex** (decision 2) **[designed]**: `futex_wait(address, expected
   value, deadline)` and `futex_wake(address, count)` for the threads of
   one process (key = process + virtual address). std then uses the
   upstream `Mutex`, `Condvar`, `RwLock`, `Once` and `park`
   implementations unchanged.
2. **Thread register for TLS** (decision 3) **[designed]**: the kernel
   must save and restore `FS_BASE` (x86_64) and `TPIDR_EL0` (aarch64) on a
   thread switch and let it be set (at `thread_start` or via a syscall).
   **A risk already today:** the kernel neither saves nor clears
   `TPIDR_EL0` on a switch, yet an aarch64 program may write it itself. A
   value written by one process is thus visible in another (a covert
   channel between processes, and C code using TLS would break). Fix it
   independently of std.
3. **CPU count** for `available_parallelism` (could be part of the system
   capability queries).
4. Later: XSAVE (AVX) for FFmpeg, demand paging (see
   [Memory management](#memory-management)), loading `ET_DYN` (see rustc).

## Phase 1 (after approval)

1. Build spike: `third_party/rust-std` with an empty platform (everything
   Unsupported), `cargo xtask std` produces a sysroot for both targets.
2. libos: `rt` feature, parameter block, stream protocol; `osrt`: start,
   heap, stdout/stderr, args/env, time, random.
3. Kernel: futex and the thread register (per the decisions; possibly
   another thread does it).
4. `osrt`: threads, TLS, futex; std `Mutex`, `thread::spawn`.
5. `osrt`: `fs` (open, read, write, read_dir, metadata, create_dir).
6. `user/hello-std`: prints arguments and variables, runs 4 threads with
   a shared `Mutex`, writes and reads a file, lists a directory. A test in
   `cargo xtask test` on x86_64 and aarch64 (a public API is done only
   with verified behavior on both architectures).
7. Docs: this page with status tags matching the state, [libos](libos.md)
   (parameter block, stream), [Syscalls](syscalls.md) (futex, thread
   register).

## Impact on the future

### libc/POSIX layer (cstd) for FFmpeg

- **One runtime per process.** A Rust program with linked C (FFmpeg in a
  media player) must have one allocator (`malloc` = the libos heap,
  already today), one TLS (`errno` and `_Thread_local` in C need native
  ELF TLS, not keys), one set of threads (pthread on top of `osrt`
  threads and the futex) and one stdio. That is why `osrt` is designed as
  the shared base for `cstd` too: cstd then becomes a thin C ABI layer
  over `osrt`.
- `cstd` needs an fd number table (int -> file, stream, socket); it
  belongs in `osrt` so that `std` and C see the same thing
  (`os::magnumopus::io::AsRawFd`).
- FFmpeg: `pthread_*`, anonymous `mmap` (works through VMOs), files,
  optional sockets, no `fork`. SIMD assembly: SSE only on x86_64 (AVX
  waits for XSAVE in the kernel), NEON on aarch64; the std target must be
  hard-float (it is).

### Porting rustc + LLVM + lld into the system

The user chose the full port (option A, no rush). What follows for the
design already now:

- **std glue as patches over `library/`**, not a copy: the same patches
  will apply to a full `rust-lang/rust` checkout (x.py with our target as
  tier 3) from which rustc for our system is cross-compiled.
- **Procedural macros** are dynamic libraries rustc loads at run time
  (`dlopen`). Without a dynamic loader rustc cannot compile crates using
  `serde_derive` and the like. Needed: an `ET_DYN` loader, `dlopen` in
  `osrt`, `dynamic-linking` in the target. This is the largest piece of
  work for rustc in the system.
- **Unwinding**: rustc catches panics (`catch_unwind`) to report ICEs;
  it works with `panic=abort`, but worse. An unwinder (libunwind) comes
  with C++ (LLVM is C++ and needs libc++, libc++abi and libunwind over
  cstd). `user.ld` already keeps `.eh_frame`.
- **Processes and memory**: rustc runs the linker as a process
  (`Command`), compiles in parallel (threads, a jobserver over a pipe) and
  needs gigabytes of memory; `memmap2` for metadata can be replaced by
  reading the file.
- **Crates with `cfg(unix)`/`cfg(windows)`** (libc, jobserver, memmap2,
  tempfile, filetime, getrandom...) need an arm for our OS. Forking each
  of them is expensive; in the long run upstream tier 3 and support in
  the crates is better.

### gix for the FS git layer

gix rests on `std::fs`, `std::path`, `std::process` (only for external
helpers) and pure Rust (SHA-1, zlib via `miniz_oxide`). Risks: some gix
crates have only `unix`/`windows` arms (file owner in `gix-sec`, the
executable bit and symlinks in `gix-fs`/`gix-index`, `gix-tempfile` with
signals). After phase 1 we run `cargo check -p gix` for our target with
minimal features and list what is missing (`cfg` arms, or permissions and
symlinks in `std::os::magnumopus`). The FS v3 version layer then talks to
gix through ordinary `std::fs`.

### Memory management

- Today: a memory object allocates all pages up front; there is no
  address reservation without memory, no file `mmap` and no
  copy-on-write.
- For std phase 1 this does not matter (the allocator and stacks go
  through VMOs).
- For large applications (browser, rustc) **demand paging** is needed
  (2 MiB stacks x dozens of threads, reservation of large heaps, JIT/LLVM
  arenas). Without it, memory is physically reserved right away.
- File `mmap`: emulated for now by reading into anonymous memory
  (read-only or private copies). Shared file mappings and CoW come with
  kernel and fsd support; there will be no `fork` (processes only via
  `spawn`).

## Decisions for approval

1. **Approach**: A own PAL (recommended) / B unix family over libc /
   C facade without a fork.
2. **Synchronization**: A a futex in the kernel (recommended; pthread in
   cstd and shared memory between processes need it too) / B a futex
   emulated inside the process on top of `Event` objects (no kernel
   change, process-local only; a temporary fallback if nobody can change
   the kernel now).
3. **TLS**: A a thread register in the kernel + native ELF TLS
   (recommended; C needs it too) / B TLS keys like Motor OS (still needs
   a thread register, only slower, and C with `_Thread_local` will not
   work).
4. **Launching processes from std** (`Command`): A directly with
   `spawn_in` in its own job, the child gets only the parent's
   capabilities / B through a launcher service (init) that applies the
   child's manifest (recommended for applications with their own manifest;
   A only for helper programs without a manifest). Needs approval because
   it touches the permission model.
5. **Floating point**: hard-float for std applications (recommended) /
   soft-float like services.
6. **Toolchain**: the same nightly as the repository (recommended) / a
   separate one for std.
7. **stdio protocol**: a plain byte stream over a channel (recommended) /
   the termd native protocol.
