# Rust std for applications

Applications for MagnumOpus will be written in ordinary Rust with `std`
(`fn main()`, `std::fs`, `std::thread`, `std::net`, crates from
crates.io). System services and drivers stay `no_std` on top of
[libos](libos.md). This page describes the approach, the build, the
mapping of `std` onto libos and services, what the kernel added for it,
and the impact on future plans.

> **Status:** design approved on 2026-11-12 (all recommendations, see
> [Approved decisions](#approved-decisions)). Phases 1 and 2 are done:
> allocation, threads, TLS, files including ACLs, network, processes
> through `launchd` and crates taken over (ripgrep, ureq)
> **[verified in QEMU]** on x86_64 and aarch64 (suites `std`, `std sit`,
> `std procesy`, `std porty`).

## Goal and scope

- Targets `x86_64-unknown-magnumopus` and `aarch64-unknown-magnumopus`
  (`target_os = "magnumopus"`) **[verified in QEMU]**.
- `std` on top of libos: allocation, threads, synchronization, TLS, time,
  files through `fsd`, arguments, environment variables, stdio, network
  through `netd`, processes through `launchd` **[verified in QEMU]**;
  stdin from the terminal, async (tokio) and dynamic libraries
  **[designed]** (per-part status in the
  [mapping](#mapping-std-onto-libos-and-services)).
- `cargo xtask build` builds std (a sysroot) and the applications and puts
  them into initfs **[verified in QEMU]**.
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

**Approved: A.** It is what upstream itself does (Motor OS and
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

- **`libs/osrt`** **[verified in QEMU]**: the runtime for `std` (like `moto-rt`).
  `#![no_std]`, feature `rustc-dep-of-std` (depends on
  `rustc-std-workspace-core`/`-alloc` instead of `core`/`alloc`, like all
  std dependencies). It holds the entry point, heap, threads, futex, TLS,
  the FS and network clients and stdio. Logic is tested on the host like
  other `libs/*`.
- **libos** has an `rt` feature (on by default) **[implemented]**:
  `entry!`, the heap, `#[global_allocator]` and `#[panic_handler]` exist
  only with it (`rt_own.rs`). `std` provides those itself, so libraries
  (`fsproto`, `svcclient`, `osrt`) depend on libos with
  `default-features = false`. std apps use a different allocator than
  services: **dlmalloc on top of memory objects** (vendored in rust-src,
  `rustc-dep-of-std`); a freed segment is unmapped, so memory goes back to
  the system.
- **abi, elf, initfs, tz, wire, libos, svcclient, fsproto** have a
  `rustc-dep-of-std` feature **[implemented]** (netproto comes with the
  network). `spin::Mutex` in fsproto and svcclient was replaced with the
  futex-based `libos::sync::Mutex` (the `spin` crate lacks that feature and
  would be missing in the offline std build). The protocols thus stay in
  **one source** for both services and `std`.

### What changes in std (the glue)

In the pinned nightly (`nightly-2026-11-05`), `target_os = "motor"` has an
arm in 32 files under `library/std/src`, Hermit in 34. Our glue in
phase 1: **35 files, ~1000 lines** (`third_party/rust-std/std.patch`),
mostly one line in a `cfg_select!` and a module of up to ~200 lines (`sys/fs/magnumopus.rs`, `sys/thread/magnumopus.rs`,
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
**[implemented]**. A second toolchain just for std would mean two copies
of `core` and two sets of compiler bugs. `rustc` must be exactly the
version the std sources belong to: xtask takes the sources from the
`rust-src` of the same toolchain it compiles with, and the paths
(`target/std-src/<nightly>`, `target/sysroot/<nightly>`) carry the nightly
name from `rust-toolchain.toml`. A patch that does not fit a new nightly
fails in `git apply`.

### How xtask builds std

<figure class="diagram">
<svg viewBox="0 0 780 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Building the std sysroot">
<defs><marker id="rs2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="230" height="70" rx="6"/><text class="t b" x="125" y="45" text-anchor="middle">rust-src (rustup)</text><text class="m" x="125" y="64" text-anchor="middle">library/ + vendor/</text>
<rect class="box" x="10" y="150" width="230" height="70" rx="6"/><text class="t b" x="125" y="175" text-anchor="middle">third_party/rust-std</text><text class="m" x="125" y="194" text-anchor="middle">std.patch + targets/*.json</text>
<path class="ln" d="M240 55 L288 100" marker-end="url(#rs2)"/>
<path class="ln" d="M240 185 L288 140" marker-end="url(#rs2)"/>
<rect class="acc" x="290" y="85" width="210" height="70" rx="6"/><text class="t b" x="395" y="110" text-anchor="middle">target/std-src</text><text class="m" x="395" y="129" text-anchor="middle">copy + git apply</text>
<path class="ln" d="M500 120 L548 120" marker-end="url(#rs2)"/>
<rect class="acc" x="550" y="20" width="220" height="70" rx="6"/><text class="t b" x="660" y="45" text-anchor="middle">target/sysroot</text><text class="m" x="660" y="64" text-anchor="middle">rlib + rmeta, fingerprint</text>
<rect class="ok" x="550" y="150" width="220" height="70" rx="6"/><text class="t b" x="660" y="175" text-anchor="middle">application</text><text class="m" x="660" y="194" text-anchor="middle">--sysroot, user-std.ld, initfs</text>
<path class="ln" d="M660 90 L660 148" marker-end="url(#rs2)"/>
</svg>
<figcaption>xtask prepares the std sources with our glue, builds a sysroot from them, and applications are then compiled against it.</figcaption>
</figure>

Code: `xtask/src/stdsys.rs` **[verified in QEMU]** (`cargo xtask build`
builds the `std` suite).

1. **Sources:** `cargo xtask std` (and `build`) copies `library/` from the
   `rust-src` of the pinned nightly into `target/std-src/<nightly>/`,
   creates its own git repository in the copy and applies
   `third_party/rust-std/std.patch` (`git apply`; when it does not fit, the
   error says the patch is for another nightly). It rewrites the path to
   `libs/osrt` in `std/Cargo.toml` to an absolute one (works with another
   `CARGO_TARGET_DIR` too). The copy is redone only when the patch changes
   (fingerprint in `patch.stamp`).
2. **Sysroot:** `cargo build --offline --release -p sysroot
   --features compiler-builtins-mem` in the copy with `RUSTC_BOOTSTRAP=1`,
   `-Zjson-target-spec` and `-Zforce-unstable-if-unmarked`. std
   dependencies (hashbrown, dlmalloc, compiler_builtins...) are in the
   `vendor/` of the rust-src component, so the build is **offline** (the
   risk from the design is gone). xtask takes the library files (`.rlib`
   and `.rmeta`: this nightly puts only stub metadata into rlibs) from the
   cargo messages (`compiler-artifact`) and copies them to
   `target/sysroot/<nightly>/lib/rustlib/<target>/lib`. Cargo builds
   incrementally: a change in osrt or libos = only those crates and std.
3. **Applications:** `cargo build --target <json> -Zjson-target-spec` with
   `--sysroot` and the `user/user-std.ld` linker script. A fingerprint of
   the sysroot contents goes into the flags (`--cfg=mo_sysroot="..."`):
   cargo does not track sysroot changes itself and the application would
   stay linked with the old std. `cargo xtask build` adds the binaries to
   initfs (list `stdsys::STD_PROGRAMS`).

Why not `-Zbuild-std` directly: cargo always takes the std sources from
`rust-src` in the toolchain sysroot, and feeding it a patched copy is only
possible through internal variables. Our own sysroot is deterministic and
needs no internal switches.

### Linker script and TLS

`user/user-std.ld` has the layout of `user/user.ld` (static `ET_EXEC` from
4 MiB, R-X, R--, RW- groups on 64 KiB pages) plus a `PT_TLS` segment.
`.tdata` and `.tbss` are in one output section (forced `PROGBITS`), so the
whole TLS template is in the file; the symbols `__tls_template_start`,
`__tls_template_size` and `__tls_align` hand it to the runtime. A thread's
TLS block (`osrt::tls`, local-exec model):

```text
x86_64 (variant II):  [ TLS data | TCB: pointer to itself ]
                                   ^ TP = FS_BASE
aarch64 (variant I):  [ TCB 16 B | TLS data ]
                      ^ TP = TPIDR_EL0
```

Offsets use the segment's `p_align` (exactly as lld does); the block as a
whole is aligned to at least 16 B. A new thread sets TP as its very first
action, before any `std` code.

### Target description (JSON)

| Field | x86_64 | aarch64 | Why |
|---|---|---|---|
| `llvm-target` | `x86_64-unknown-none-elf` | `aarch64-unknown-none` | OS-less ELF in LLVM |
| `os` / `target-family` | `magnumopus` / none | same | not `unix`, see options |
| floating point | x86-64-v2: SSE up to SSE4.2 (hard-float, [hardware-requirements.md](hardware-requirements.md)) | NEON, `fp-armv8` (hard-float) | performance (FFmpeg, LLVM, browser); the kernel already saves thread FPU/SIMD state |
| `panic-strategy` | `abort` | `abort` | unwind later (see rustc) |
| `relocation-model`, linker | `static`, rust-lld, `user/user.ld` | same | the loader only handles `ET_EXEC` |
| `has-thread-local`, `tls-model` | yes, `local-exec` | yes, `local-exec` | native TLS (requires a kernel change) |
| `max-atomic-width` | 64 | 128 | |

Since FPU-1 services and programs without std are built for the same
targets (`*-unknown-magnumopus`, hard-float; core and alloc via
`-Zbuild-std`); the kernel stays `*-none` with soft-float. std
applications have their own sysroot, so shared crates (libos,
fsproto...) are compiled twice. AVX and AVX-512 work since FPU-2 (the
kernel saves state with XSAVE) and are detected at run time.

## Mapping std onto libos and services

Status column legend: status tags from [Writing documentation](documentation.md);
**Unsupported** = returns `io::ErrorKind::Unsupported`. Tests of the `std`
suite are in `user/init/src/tests/std_app.rs`; "hello-std" = test `std:
hello-std: argumenty, vlakna, TLS, soubory` (x86_64 and aarch64).

| Part of std | On top of | Status |
|---|---|---|
| start (`main`, `exit`, `abort`) | `_start` in `osrt::start` reads the startup message (`libos::rt::startup`), sets up TLS and calls `main` from rustc; `process_exit` | **[verified in QEMU]** hello-std |
| allocator (`System`) | dlmalloc on top of memory objects (`osrt::heap`), futex-based lock | **[verified in QEMU]** hello-std |
| `env::args` | process parameter block | **[verified in QEMU]** hello-std |
| `env::var`, `set_var`, `vars` | parameter block, changes kept in-process | **[verified in QEMU]** hello-std |
| `env::current_dir` / `set_current_dir` | path in `osrt::env`, relative paths joined in-process (`.` and `..`) | **[verified in QEMU]** hello-std |
| `io::stdout/stderr` | byte-stream channel; without one the kernel log, line by line | **[verified in QEMU]** hello-std |
| `io::stdin` | stream channel (`osrt::stdio::read`) | **[verified in QEMU]** std-proc (pipe), termd integration **[designed]** |
| `time::Instant` | `clock_monotonic` | **[verified in QEMU]** hello-std |
| `time::SystemTime` | `clock_realtime` (before 1970 = the epoch) | **[verified in QEMU]** hello-std |
| `thread::spawn`, `join`, `sleep`, `yield_now` | `libos::thread` + a TLS block (`osrt::thread`) | **[verified in QEMU]** hello-std |
| `Mutex`, `Condvar`, `RwLock`, `Once`, `park` | upstream implementations on the futex (`osrt::futex`) | **[verified in QEMU]** hello-std (`Mutex`, `Condvar`), the rest **[implemented]** |
| `thread_local!` | native ELF TLS (local-exec), std runs destructors at thread end | **[verified in QEMU]** hello-std |
| `thread::available_parallelism` | `SCHED_CONTROL` `CPU_INFO` (CPU count) | **[verified in QEMU]** hello-std |
| `random` (`HashMap` `RandomState`) | `random_get` | **[verified in QEMU]** hello-std |
| `fs::File` open/read/write/seek/`set_len`/`sync_all`, append | fsproto v2 (session handles; position in `osrt::fs::File`) | **[verified in QEMU]** hello-std |
| `fs::read_dir`, `metadata`, `create_dir`, `remove_*`, `rename`, `canonicalize` | fsproto (`STAT`, `LIST`, `CREATE`, `REMOVE`, `RENAME`); `canonicalize` = normalization + existence check | **[verified in QEMU]** hello-std (except `canonicalize`) |
| `fs::Permissions`, ACL | `readonly` from the effective rights, setting is Unsupported; ACLs through `std::os::magnumopus::fs` | **[verified in QEMU]** hello-std |
| `read_link`, `symlink_metadata`, `soft_link`, `canonicalize` | fsd links ([Symbolic links](file-system.md#symbolic-links)); `canonicalize` is resolved by fsd | **[verified in QEMU]** (through cstd, step `link`) |
| `hard_link`, file locks, file times | Unsupported | FS v3 decides **[designed]** |
| `net::TcpStream`, `TcpListener`, `UdpSocket`, DNS | netproto (`tcp_connect`, `tcp_listen`, `udp_bind`, `resolve`), network session with tag `NET` (see [Network](#network-stdnet)) | **[verified in QEMU]** std-net |
| `process::Command`, `Child`, `io::pipe` | the `launchd` launcher service with the child's manifest, pipes = channel pairs (see [Processes](#processes-stdprocess)) | **[verified in QEMU]** std-proc |
| `os_str`, `path` | bytes (like unix), separator `/`; `std::os::magnumopus::ffi::{OsStrExt, OsStringExt}` | **[verified in QEMU]** hello-std |
| `backtrace` | Unsupported (a panic prints the message to stderr) | later |
| `fork`, signals, `os::unix` | never | on purpose |

### Process parameter block (libos)

Format `abi::params`, reading and creating with `libos::params::Params`,
passed in the startup message as `kind::PARAMS` **[verified in QEMU]**
(hello-std; format tests on the host `cargo test -p abi params`). Standard
handles have tags from `abi::params::tag` (`STDIN`, `STDOUT`, `STDERR`,
`FS`, `NET`, `LAUNCH`, `FD`); format v2 has the `STDOUT_NULL`/`STDERR_NULL`
flags (`Stdio::null`), v3 inherited fds (`posix_spawn`). Details and an example: [libos](libos.md#process-parameter-block).

### Byte-stream protocol

`stdout`/`stderr`/`stdin` are channels: every message = a chunk of bytes
(at most 64 KiB), closing the channel = end of stream, an empty message is
not the end **[verified in QEMU]** (hello-std, the test reads stdout until
it closes). Back-pressure: a channel has no "writable" signal, so with a
full queue (`SHOULD_WAIT`) a write waits a millisecond and retries. Integration
with termd and the shell is work for the terminal thread **[designed]**.

### Network (`std::net`)

`TcpStream`, `TcpListener`, `UdpSocket` and name resolution
(`ToSocketAddrs`) go through the network session the process received with
the `NET` tag **[verified in QEMU]** (test `std sit: std-net: TCP, vlakna,
UDP, DNS, opravneni`, x86_64 and aarch64). What the session may do
(protocols, targets, ports, firewall questions) is decided by whoever
created it; outside the permission it returns `PermissionDenied`, and so
does listening without the right. Without a network session the
operations return `Unsupported`.

- `UdpSocket::connect` is emulated in the process (`send`/`recv` only with
  the target).
- `TcpListener::bind`: netd listens on all interfaces; the interface
  address is not used yet.
- `set_nonblocking(true)`, `peek`, `ttl`, `linger`, `keepalive` and
  multicast return `Unsupported`; `set_nodelay` only reports success.
- **Full duplex:** osrt does not lock its sockets (`TcpStream`,
  `TcpListener`, `UdpSocket`; the `netproto` v2 client types are `Sync`):
  a thread blocked in `read` does not delay a write on the same connection
  (a `try_clone` clone or `&TcpStream`) **[verified in QEMU]** (test
  `std-net`, check `shared_stream_duplex`; `nc` relies on it,
  [Network](network.md#network-commands-of-the-shell)).
- **Limitation today:** osrt still locks the network session, so a long
  `connect` (firewall question up to 300 s) delays the other calls of the
  session (new connections, DNS) **[implemented]**.

### Processes (`std::process`)

<figure class="diagram">
<svg viewBox="0 0 780 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Launching processes through launchd">
<defs><marker id="rs3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="ok" x="10" y="20" width="230" height="110" rx="6"/><text class="t b" x="125" y="45" text-anchor="middle">parent (std)</text><text class="m" x="125" y="66" text-anchor="middle">Command::spawn</text><text class="m" x="125" y="84" text-anchor="middle">job, new FS session,</text><text class="m" x="125" y="102" text-anchor="middle">network only with network(true),</text><text class="m" x="125" y="120" text-anchor="middle">stdio, parameter block</text>
<path class="ln" d="M240 75 L298 75" marker-end="url(#rs3)"/><text class="m" x="269" y="66" text-anchor="middle">LAUNCH</text>
<rect class="acc" x="300" y="20" width="230" height="110" rx="6"/><text class="t b" x="415" y="45" text-anchor="middle">launchd</text><text class="m" x="415" y="66" text-anchor="middle">finds the program</text><text class="m" x="415" y="84" text-anchor="middle">manifest: job limits,</text><text class="m" x="415" y="102" text-anchor="middle">network = intersection</text><text class="m" x="415" y="120" text-anchor="middle">no root rights</text>
<path class="ln" d="M530 75 L588 75" marker-end="url(#rs3)"/>
<rect class="ok" x="590" y="20" width="180" height="110" rx="6"/><text class="t b" x="680" y="45" text-anchor="middle">child</text><text class="m" x="680" y="66" text-anchor="middle">sub-job of the parent</text><text class="m" x="680" y="84" text-anchor="middle">own channel</text><text class="m" x="680" y="102" text-anchor="middle">to launchd</text>
<rect class="box" x="300" y="180" width="470" height="100" rx="6"/><text class="t b" x="535" y="205" text-anchor="middle">lookup by name</text><text class="m" x="535" y="228" text-anchor="middle">1. command registry ([[command]] in manifests)</text><text class="m" x="535" y="246" text-anchor="middle">2. /system/bin/&lt;name&gt; in the parent's FS session  3. initfs</text><text class="m" x="535" y="264" text-anchor="middle">absolute path = file with the EXECUTE right (manifest next to it)</text>
<path class="ln" d="M415 130 L415 178" marker-end="url(#rs3)"/>
</svg>
<figcaption>A child never gets more than its parent: the parent supplies the capabilities, launchd only narrows them by the manifest.</figcaption>
</figure>

`Command`, `Child`, `ExitStatus`, `kill`, `wait`, `try_wait`, `output`,
`Stdio::piped/null/inherit` and `io::pipe` **[verified in QEMU]** (test
`std procesy: std-proc: Command, roury, stdio, kody, sit, registr`, x86_64
and aarch64). They are launched by the `launchd` service
(`user/launchd`, protocol `libs/launchproto`, decisions 1 and 4):

- **The child's capabilities** come from the parent in the request: its
  job (the child gets a sub-job, never more than the parent), a new FS
  session with the same identity (files are inherited), the network
  session **only explicitly**
  (`std::os::magnumopus::process::CommandExt::network(true)`), stdio.
  launchd has no root rights; it adds only what the parent cannot do
  itself: it finds the program, applies the manifest and gives the child
  its own channel to launchd.
- **The child's manifest** (next to the program,
  `<program>.manifest.toml`): job limits, and network = the parent's
  session narrowed by the manifest (netd narrows, never widens); a
  manifest without `[network]` = no network even with `network(true)`. A
  program without a manifest (a helper) gets the network session as the
  parent passed it.
- **Difference from `posix_spawn` in C** ([cstd](cstd.md#processes-posix_spawn-and-waitpid)):
  `Command` passes the network only with `network(true)`, `posix_spawn`
  always passes it when the parent has one (POSIX: the child inherits the
  parent's environment). The manifest rule is the same for both: for an
  application with a manifest launchd decides about the network from the
  manifest, not the parent.
- **Finding the program:** a bare name = the command registry
  (`[[command]]` in manifests), then `/system/bin/<name>` in the parent's
  FS session, then initfs; an absolute path = a file in the FS opened with
  the `EXECUTE` right (datasets with execution disabled refuse it). No
  `PATH`.
- **Stdio:** `inherit` = a duplicate of the parent's channel, `piped` = a
  new channel pair, `null` = a flag in the parameter block (format v2),
  `io::pipe` = a channel pair. A file as the child's stdio returns
  `Unsupported` for now.
- **Codes:** `ExitStatus::code` = the `PROCESS_EXIT` code (kernel codes,
  e.g. -1,000,000 after `kill`, fit into `i32`). `Child::id` = the process
  handle number; `process::id` = a random number chosen at start (the
  kernel does not report a process number yet).
- A launch error carries a description from launchd (e.g. "volume X is
  not mounted", code `NOT_MOUNTED`).

### Command registry and application installation

The registry interface (`launchproto::Registry`: command name -> program
source and application) **[implemented]**; today launchd fills it from the
manifests in initfs **[verified in QEMU]** (test `std-proc`, command
`std-proc-kid`). Connection to installed applications **[designed]** (see
[Architecture](architecture.md), Applications): packages on any volume
(`/svazky/<name>`) add their commands to the same registry; an application
on an unmounted volume = error `NOT_MOUNTED` naming the volume, not "not
found". Launching directly from foreign file systems (FAT, NTFS without
our rights) only with the explicit `ALLOW_FOREIGN` flag **[designed]**
(foreign file systems are not recognised yet). The `NET_NEEDS_MANIFEST`
flag: only a program with a manifest gets the caller's network session
(narrowed by its `[network]`), a program without one gets none. init's
exec service uses it for programs from the shell
([Shell design](shell-design.md)) **[verified in QEMU]**.

### File permissions

`fs::Permissions::readonly` = the write right is missing from the
caller's effective rights; `set_permissions` returns `Unsupported` (the
decision: rights go through ACLs). Full permissions:
`std::os::magnumopus::fs::{access, set_acl, Acl, Grant, MetadataExt}`
(owner, effective rights, permission description with grants)
**[verified in QEMU]** (hello-std sets and restores an ACL).

## Ports: crates taken over unmodified

Practical check (x86_64 and aarch64, suite `std porty`):

| Program | Source | Changes | Status |
|---|---|---|---|
| ripgrep 15.2.0 (`rg`) | crates.io, `cargo install --locked` (`xtask/src/ports.rs`) | patch to `ignore` 0.4.29 (7 lines, `third_party/ports`) | **[verified in QEMU]** test `ripgrep: hledani ve strome souboru` |
| `std-http` on ureq 3.5.2 | library from crates.io without TLS | none | **[verified in QEMU]** test `ureq: HTTP klient, 4 vlakna soubezne` |
| `getrandom-probe` (`user/port-tests`) | getrandom 0.4.3 from crates.io | patch to `getrandom` (`RANDOM_GET` backend) | **[verified in QEMU]** test `getrandom: backend RANDOM_GET` |

### Patches for upstream

Dependency fixes are in `third_party/ports/` and xtask applies them to all
ports (`xtask/src/ports.rs`, `PATCHES`: a copy of the source from the cargo
cache + `[patch.crates-io]`). We will send them to the authors one day
(once the `*-unknown-magnumopus` target is public, ideally as tier 3):

| Crate | Patch | What it changes | Upstream status |
|---|---|---|---|
| `ignore` 0.4.29 | `ignore-0.4.29.patch` | the generic arm of parallel directory traversal creates an entry without an inode number instead of the "unsupported platform" error (helps other platforms too) | not sent |
| `getrandom` 0.4.3 | `getrandom-0.4.3.patch` | a backend for `target_os = "magnumopus"`: the `RANDOM_GET` syscall with `REQUIRE_SEEDED`; until the generator is seeded the thread yields and retries | not sent (requires the target in upstream rustc) |

What was missing (found by compiling for our target, outside the tests):

- **`ignore` (ripgrep):** parallel directory traversal had only `unix` and
  `windows` arms and returned "unsupported platform" elsewhere. Fix: the
  generic arm creates an entry without an inode number. A candidate for
  upstream.
- **tokio with `net` (mio, socket2):** `mio` has no backend for our system
  (Selector, Waker, Event) and `socket2` does not know our target. Needed:
  a readiness API in the kernel and netd (a signal on an object or a
  port), a `mio` backend for MagnumOpus and support in `socket2`
  **[designed]**. tokio without `net` (runtime only) should work.
- **gix 0.90 (`sha1`, no default features):** `getrandom` 0.4 did not know
  our target (solved by the patch above); next
  `gix-sec` (file owner check via `libc::geteuid` and `os::unix`, the
  "not Windows" arm assumes unix). Further errors will show only after
  these.
- **Target family:** in general, crates with only `unix`/`windows` arms
  need a third arm; upstream tier 3 would help broaden support (see
  [Approach](#approach)).

## What the kernel added

1. **Futex** **[verified in QEMU]** (test `std: futex: wait, wake,
   vyprseni, neznama operace`): the `FUTEX` syscall (55) with operations `WAIT(address, expected value,
   deadline)` and `WAKE(address, count)`, a per-process queue (key = virtual
   address), see [Syscalls](syscalls.md#futex).
2. **Thread register for TLS** **[verified in QEMU]** (test `std: registr
   vlakna: vlastni hodnota kazdeho vlakna`): `THREAD_SET_POINTER` (56); the
   kernel stores `FS_BASE` / `TPIDR_EL0` in the thread and loads it on
   every switch. This also fixes the `TPIDR_EL0` leak between processes on
   aarch64 (a program could write it and the kernel did not clear it on a
   switch).
3. **CPU count** was already provided by the kernel (`SCHED_CONTROL`
   `CPU_INFO`).
4. Later: demand paging (see
   [Memory management](#memory-management)), loading `ET_DYN` (see rustc).

## Known limitations

- **Thread stack without a guard page** **[designed]**: an overflow may
  overwrite a neighbouring mapping. Needs address reservation without
  memory (`vm_reserve`) in the kernel.
- **Detached thread** (a dropped `JoinHandle`): its stack and TLS block
  stay until the process ends (the thread may still use them).
- **TLS destructors of the main thread** do not run (the process ends
  after `main`, like on Hermit).
- **Full stdout queue** = waiting in millisecond steps (see the stream protocol).
- **`current_exe`** is Unsupported (a process does not know its path; a
  program may even come from initfs).
- **Returning memory**: dlmalloc returns only whole segments; smaller
  free pieces stay with the process.
- **Concurrency on a network connection** is serialized (see
  [Network](#network-stdnet)) until `netproto` v2.
- **launchd** is not restarted by the supervisor after a crash yet (only
  watched); applications then get `SERVICE_UNAVAILABLE`.
- **Narrowing the FS session by the child's manifest** (mounted paths by
  profile) is not done yet: the child gets a new session with the same
  paths as the parent **[designed]**.

## Phase 1 (done)

1. Build spike: JSON targets, a sysroot from unmodified std (a precursor
   of step 4).
2. Kernel: futex and the thread register **[verified in QEMU]**.
3. libos: `rt` feature, parameter block, `sync::Mutex` **[verified in QEMU]**.
4. `osrt` and the std glue (35 files) **[verified in QEMU]**.
5. `user/hello-std` in initfs, suite `std` on x86_64 and aarch64
   **[verified in QEMU]**.
6. `cargo xtask std` and `cargo xtask build` build std and the
   applications **[verified in QEMU]**.

## Phase 2 (done)

1. `std::net` on top of netd with a restricted network session
   **[verified in QEMU]**.
2. `std::process` through `launchd` (command registry, manifest, pipes,
   stdio, codes, `kill`, network only explicitly) **[verified in QEMU]**.
3. File permissions and ACLs **[verified in QEMU]**.
4. Ports from crates.io: ripgrep and ureq **[verified in QEMU]**, a tokio
   and gix survey (see [Ports](#ports-crates-taken-over-unmodified)).
5. `cargo xtask std-patch`: glue changes in `target/std-src` are saved
   back to `third_party/rust-std/std.patch` **[implemented]**.

Next step **[designed]**: `netproto` v2 (network thread), stdin and stdio
in the terminal, a readiness API for async (mio), a `getrandom` backend
and arms for gix, the stack guard page.

## Impact on the future

### libc/POSIX layer (cstd) for FFmpeg

Part 1 is done, description and plan: [The C library (cstd)](cstd.md).


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
  optional sockets, no `fork`. SIMD assembly: SSE up to AVX-512 on x86_64
  (run-time detection, XSAVE in the kernel since FPU-2), NEON on aarch64; the std target must be
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

Survey on 2026-11-14 (gix 0.90): the first obstacles are `getrandom` and
`gix-sec` (see [Ports](#ports-crates-taken-over-unmodified)).

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

## Approved decisions

Approved on 2026-11-12, all recommendations:

1. **Approach**: A own PAL in a std fork (glue) with the logic in `osrt`.
2. **Synchronization**: a futex in the kernel.
3. **TLS**: a thread register in the kernel + native ELF TLS.
4. **Launching processes from std** (`Command`): applications with their
   own manifest through a launcher service (init) that applies the child's
   manifest; direct `spawn_in` only for helper programs without a manifest.
5. **Floating point**: hard-float for std applications.
6. **Toolchain**: the same nightly as the repository.
7. **stdio protocol**: a plain byte stream over a channel.
