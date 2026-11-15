# Windows layer: Wine over our lower layer

Design of step 8 of the order in [architecture.md](architecture.md) (the
plan "upper layer from Wine, lower layer ours"): Windows applications and
games over adopted Wine. **Status:** questions decided on 15 Nov 2026
(chapter [Decisions](#decisions)); stage W1 (console programs) **[verified
in QEMU]** x86_64, later stages **[designed]** (table below).

## Status

| Part | What | Status |
|---|---|---|
| W0 | building Wine 11 (PE DLLs, tools), port of the Unix side over `libc.so` | all of Wine via `cargo xtask wine` in WSL **[implemented]**: 602 DLLs as PE, 25 Unix `.so` over `libc.so` (`ntdll.so` with `libunwind.so`), the `wine` loader and `wineserver`; `wine --version` in the system **[verified in QEMU]** ([Wine on disk](#wine-on-disk-w1)) |
| W1 | console `.exe` (`cmd`, `hostname`, our own test): lower `ntdll`, TEB via GS, SEH, memory, files, `wineserver` as a process | **[verified in QEMU]** x86_64, release (`wine` suite): GS in the kernel and `ucontext_t` in `cstd` **[verified in QEMU]**; the exception patch `signal_x86_64.c` compiles **[implemented]**; Wine on the data disk as an application (`/apps/org.winehq.wine`), the `wine` loader from disk loads `ntdll.so` ([DL-4c](dynamic-linking.md#libraries-from-disk-and-origin-dl-4c)) **[verified in QEMU]**; a new prefix (`wineboot`), `cmd /c echo`, our own console program `user/wintest` and `hostname` ([Wine on disk](#wine-on-disk-w1)); the `wx_compat` permission (RWX, Q6) is not needed yet **[designed]** |
| W2 | several processes of one application, registry, named objects, `winecfg` without windows | **[designed]** |
| W3 | windows: a Wine driver over the compositor (`guiproto`), GDI, `notepad`, `winecfg`, clipboard, input | **[designed]** |
| W4 | sound over `audiod`, a simple game (GDI: `winemine`, then a game via SDL or Direct3D 9) | **[designed]** |
| W5 | DXVK and VKD3D-Proton over Vulkan (Venus in QEMU, GPU drivers on HW) | **[designed]** |
| W6 | 32-bit applications (the new WoW64) | **[designed]** |
| W7 | fast synchronization (an ntsync analogue), game performance | **[designed]** |

## Starting point

Checked in the repository (`main` + branch 7a-cxx):

- **Finished prerequisites** from the adoption order: Rust std, `cstd`
  (`libc.a` and `libc.so`, signals, `mmap`, threads, sockets, locale and
  wide characters), memory management (`VM_RESERVE`, `MAP_FIXED` inside a
  reservation, `VM_PROTECT`, demand paging, CoW), dynamic linking
  (`ld.so`, `dlopen`), exceptions and thread context (`THREAD_SUSPEND`,
  `THREAD_STATE`, exception channels, in-thread delivery), the C++
  runtime, Vulkan via Venus in QEMU.
- **TEB via GS is missing:** on x86_64 the kernel switches GS to the CPU
  data on entry (`swapgs`) and the program GS is always 0
  (`MSR_KERNEL_GS_BASE` = 0); a program only has the FS base (TLS,
  `THREAD_SET_POINTER`). Windows on x86_64 reads the TEB via `gs:[0x30]`
  and Wine requires it.
- **W^X is strict:** never write and execute at once, not even with the
  JIT permission ([memory management](memory-management.md)). Windows
  applications commonly ask for `PAGE_EXECUTE_READWRITE` (.NET JIT,
  LuaJIT, game protections, older compilers).
- **Segments:** the GDT only has 64-bit user code (`USER_CODE`); 32-bit
  code (WoW64) needs a compat mode code segment.
- **Tools:** `configure` ports (FFmpeg) build in MSYS2; the repository also
  has `tools/wsl` (the Venus build).

## Wine today: what is what

Wine 11.0 (13 Jan 2026) and its split that the design starts from:

<figure class="diagram">
<svg viewBox="0 0 780 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Parts of Wine and what we adopt">
<defs><marker id="wn1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="10" width="760" height="40" rx="6"/><text class="t b" x="390" y="35" text-anchor="middle">the application .exe and its DLLs (PE)</text>
<rect class="ok" x="10" y="70" width="760" height="56" rx="6"/><text class="t b" x="390" y="93" text-anchor="middle">Wine DLLs as PE: kernel32, kernelbase, user32, gdi32, ole32, comctl32, ucrtbase, ...</text><text class="m" x="390" y="112" text-anchor="middle">adopted unchanged (LGPL), built by clang for the Windows target</text>
<rect class="ok" x="10" y="146" width="370" height="64" rx="6"/><text class="t b" x="195" y="170" text-anchor="middle">ntdll.dll (PE)</text><text class="m" x="195" y="189" text-anchor="middle">PE loader (ldr), SEH, heap, Rtl*</text><text class="m" x="195" y="203" text-anchor="middle">adopted</text>
<rect class="acc" x="400" y="146" width="370" height="64" rx="6"/><text class="t b" x="585" y="170" text-anchor="middle">Unix libraries (.so)</text><text class="m" x="585" y="189" text-anchor="middle">ntdll.so, win32u.so, winevulkan.so, drivers</text><text class="m" x="585" y="203" text-anchor="middle">the system boundary: our work is here</text>
<rect class="acc" x="400" y="230" width="370" height="40" rx="6"/><text class="t b" x="585" y="255" text-anchor="middle">wineserver: objects, windows, queues, registry</text>
<rect class="box" x="10" y="290" width="760" height="34" rx="6"/><text class="t b" x="390" y="312" text-anchor="middle">libos, libc.so and the system services (fsd, compd, audiod, netd)</text>
<path class="ln" d="M390 50 L390 66" marker-end="url(#wn1)"/>
<path class="ln" d="M195 126 L195 142" marker-end="url(#wn1)"/>
<path class="ln" d="M380 178 L396 178" marker-end="url(#wn1)"/>
<path class="ln" d="M585 210 L585 226" marker-end="url(#wn1)"/>
<path class="ln" d="M585 270 L585 286" marker-end="url(#wn1)"/>
<path class="ln" d="M500 210 L500 286" marker-end="url(#wn1)"/>
</svg>
<figcaption>The upper part (DLLs as PE) is adopted unchanged. NT system calls end in the Unix libraries and in wineserver: that is the boundary we connect to our system.</figcaption>
</figure>

- **DLLs as PE:** since Wine 9 almost all DLLs are built as real Windows
  DLLs. They call the system via `ntdll.dll`, which goes to the **Unix
  side** through the call dispatcher (`__wine_syscall_dispatcher`).
- **The Unix side** (`ntdll.so` and "unixlibs" such as `win32u.so`,
  `winevulkan.so`, `winepulse.so`, `winewayland.so`): C over POSIX, about
  40,000 lines in `ntdll/unix` alone (memory `virtual.c`, signals and
  exceptions `signal_x86_64.c`, threads, files, the server connection).
  `win32u.so` holds windows, messages and GDI (it talks to wineserver and
  to the window driver).
- **wineserver:** a separate process per prefix (the shared state of
  processes: handles, synchronization objects, windows and message queues,
  hooks, registry). The protocol goes over a Unix socket with fd passing
  (`SCM_RIGHTS`) and shared memory for the queues.
- **WoW64 (new, fully supported in Wine 11):** 32-bit code runs in a
  64-bit process; it switches to 64-bit code with a far jump to a 64-bit
  code segment (`wow64cpu.dll`). The Unix side is 64-bit only.
- **Proton 11** (Valve, stable since 7 Jul 2026) is based on Wine 11.0:
  a Wine branch with game patches, DXVK, VKD3D-Proton, dxvk-nvapi, Wine
  Mono.

## Questions

### Q1 - Wine version

- **A (recommended):** upstream **Wine 11.0.x** (stable branch, a pinned
  archive from dl.winehq.org, SHA-256). DXVK and VKD3D-Proton as separate
  ports (the same versions as in Proton 11). Take game patches from Proton
  one by one when a specific game needs them.
- **B:** the Wine branch of **Proton 11** as the base (more game fixes,
  but its own branch with rebases, changes harder to trace, and the
  upstream 11.x development series moves away meanwhile).
- **C:** both (applications upstream, games Proton) - two builds and two
  sets of patches of our lower layer.

### Q2 - the Unix side of `ntdll`

- **A (recommended):** **adopt `ntdll/unix` from Wine (C) over
  `libc.so`** and replace only the OS-dependent modules with our own:
  memory (`virtual.c` over `VM_RESERVE`/`VM_MAP`/`VM_PROTECT` via a libos
  extension), exceptions (`signal_x86_64.c` over our in-thread delivery
  and `Frame`), threads and the TEB (over osrt and the new GS), the server
  connection (channels instead of a socket). The rest (files, sync via
  the server, registry, processes) stays Wine code over POSIX from
  `cstd`. Estimate: days to weeks to the first console `.exe`.
- **B:** our own Unix side in Rust (a table of about 500 `Nt*`
  functions) - cleaner, but months of work and permanent catching up
  with Wine.
- **C:** the whole Wine Unix side unchanged over a POSIX layer (like the
  macOS port) - the least work at the start, but it needs `SCM_RIGHTS`,
  `ptrace`, signals between processes and fixed addresses that, per the
  architecture, we do not want to build just for Wine.

### Q3 - wineserver

- **A (recommended):** **wineserver from Wine (C) as a separate process
  per prefix** (= per application), over `libc.so`; the protocol
  transport goes over channels with handle passing instead of a socket
  and `SCM_RIGHTS`. Windows, message queues, hooks and the registry stay
  exactly as in Wine (that is the hardest and most important semantics).
  Move synchronization out of the server later (Q4).
- **B:** our own Rust service with the wineserver protocol - tens of
  thousands of lines (`window.c`, `queue.c`, `hook.c`, `registry.c`...)
  again.
- **C:** our own service only for objects and synchronization (Rust over
  `wait_many`, `Event`), windows and registry from wineserver - two
  servers with split state, more complex than A + Q4.

### Q4 - synchronization (game performance, stage W7)

- **A (recommended):** first via wineserver (like Wine without ntsync),
  then **an ntsync analogue in the layer service**: NT objects (event,
  mutant, semaphore) in Rust over futexes and `wait_many`, waiting
  without a server round trip.
- **B:** esync/fsync like Proton (shared memory + futexes in the
  application processes) - fast, but only within one application and
  with weaker isolation.
- **C:** NT objects in the kernel (like the ntsync module in Linux) - the
  fastest, but it breaks "clean kernel" (the kernel would know Windows
  semantics).

### Q5 - TEB via GS (kernel)

- **A (recommended):** **a per-thread program GS base in the kernel**:
  `THREAD_SET_POINTER` gets a second register (FS = TLS as today, GS = "a
  second thread pointer", named generically, not "TEB"); the kernel
  writes it into `MSR_KERNEL_GS_BASE` on a thread switch (the kernel
  entry `swapgs` stays). The context (`THREAD_STATE`) will read and write
  GS. aarch64: Windows ARM64 keeps the TEB in `x18`, an ordinary register
  the context already carries; the GS call returns `NOT_SUPPORTED` on
  aarch64 and we state the two-architecture rule explicitly as a
  limitation.
- **B:** turn on `CR4.FSGSBASE` (the program changes FS/GS itself with
  `wrgsbase`): faster switching, but the kernel entry must handle a GS set
  by the program (a paranoid entry like Linux) and FS is no longer under
  kernel control.
- **C:** the TEB elsewhere than in GS (patch Wine and binaries) -
  impossible, applications read `gs:` directly.

### Q6 - memory and `PAGE_EXECUTE_READWRITE`

- **A:** keep strict W^X for Windows too: emulate `RWX` by toggling (the
  page is RX, a write = a fault, the layer switches it to RW, execution =
  a fault back to RX). Safe, but the .NET JIT and game protections will be
  slow (a fault on every switch) and code that writes into the page that
  is currently running will not work.
- **B (recommended):** **RWX only for applications with a new manifest
  permission** (`wx_compat`, shown to the user at install time as "may
  modify its own code"); other and native programs keep strict W^X; audit
  as for JIT. A change of a memory management invariant, therefore
  explicit.
- **C:** RWX for all Windows layer processes (simplest, weakest).

### Q7 - files, `C:\` and letter case

- **A (recommended):** the application prefix is **an fsd dataset with
  the "case-insensitive" property** (casefold in directory lookup, like
  NTFS or ext4 casefold); `C:\` = the prefix root, other drives only
  explicitly allowed folders (Working folders). A new FS feature, not
  just a layer one.
- **B:** like Wine on Linux: case-insensitive lookup in `ntdll` by
  scanning the directory (slow for large directories, races on create).
- **C:** A, but B first (a quick W1 start, casefold dataset in W2).

### Q8 - windows and graphics

- **A (recommended):** **`win32u.so` from Wine unchanged + our own window
  driver** (a Unix library like `winewayland`, in C over the compositor C
  interface similar to `libs/mosdl`): every top-level window = a
  compositor window, child windows are drawn by `win32u` (GDI in
  software, dibdrv) into the window buffer; input, cursor, clipboard and
  DPI via `guiproto`.
- **B:** first only a "virtual desktop" (one compositor window with the
  whole Windows desktop inside) - a simpler start, worse integration
  (Alt+Tab, windows outside the desktop).
- **C:** A, with the virtual desktop as an option for full-screen games.

Vulkan (`winevulkan.so`) will run over our Vulkan loader (Venus in QEMU);
we will not do OpenGL (`wined3d` via GL) - Direct3D only via DXVK and
VKD3D-Proton.

### Q9 - sound

- **A (recommended):** our own Wine sound driver (a Unix library like
  `winepulse`, the `mmdevapi` interface) over `audioproto`, a session with
  the application rights and the recording permission as with SDL.
- **B:** sound via the SDL port (`winesdl`) - one more layer.

### Q10 - 32-bit applications (WoW64)

- **A (recommended, stage W6):** the new WoW64 from Wine 11. Kernel: **a
  ring 3 32-bit code segment** in the GDT (compat mode, a generic x86_64
  capability, not "for Windows"), exceptions and interrupts from compat
  mode (an `iretq` return with a different CS, the context with CS),
  `syscall` only from 64-bit code (WoW64 calls it after switching).
  Memory: reservations in the lower 4 GiB.
- **B:** no 32-bit applications (many older games and installers will not
  work).
- **C:** emulate 32-bit code (box64/FEX) even on x86_64 - needlessly
  slow.

### Q11 - aarch64

- **A (recommended):** the Windows layer only on x86_64; on aarch64 an
  explicit **limitation** (two-architecture rule: new public API, e.g. GS,
  is defined for aarch64 too, with `NOT_SUPPORTED`). ARM64 and ARM64EC
  applications with x86 emulation (FEX, like Proton 11) as a later step.
- **B:** aarch64 right away with native Windows ARM64 applications
  (`x18` = TEB) and without x86 emulation - few applications.

### Q12 - building Wine

- **A (recommended):** Wine builds in **WSL** (Debian): `configure` and
  the Wine tools (`winegcc`, `widl`, `wrc`, `winebuild`) want a Unix
  host; clang builds the PE DLLs (`--target=x86_64-windows-gnu`, lld) and
  the Unix side against the C sysroot (`clang-dyn.cfg`). xtask runs the
  build via `wsl` (the repository already has `tools/wsl`).
- **B:** MSYS2 like FFmpeg - Wine does not officially support MSYS2 as the
  host for a foreign Unix, I expect a lot of `configure` changes.
- **C:** the PE DLLs from WineHQ builds (an archive with binaries), only
  the Unix side ourselves - fast, but the PE and Unix side versions must
  match exactly and without building PE we cannot fix a bug in a DLL.

## Decisions

Decisions of the user and the coordinator (15 Nov 2026):

| Question | Decision |
|---|---|
| Q1 | A: upstream Wine 11.0.x, DXVK and VKD3D-Proton as ports, Proton patches one by one; possibly a second build from Proton later |
| Q2 | A: `ntdll/unix` from Wine over `libc.so`, our own modules for memory, exceptions, threads, transport |
| Q3 | A: wineserver from Wine as a process per prefix, the protocol over channels |
| Q4 | A: first via wineserver, then an ntsync analogue in the layer service |
| Q5 | A: GS as a generic "second thread pointer" per thread in the kernel |
| Q6 | B: RWX only with the `wx_compat` manifest permission (visible at install time, audited); the W^X invariant change stated explicitly in [memory management](memory-management.md) and `kernel/CLAUDE.md` |
| Q7 | C: first case-insensitive lookup in `ntdll`, a casefold dataset in fsd in W2 (a generic FS feature, a separate design) |
| Q8 | A: `win32u.so` from Wine + our own window driver over the compositor |
| Q9 | A: our own sound driver over `audioproto` |
| Q10 | A: WoW64, the 32-bit code segment in stage W6 |
| Q11 | A: x86_64 only; aarch64 = a documented limitation, `NOT_SUPPORTED` in the API |
| Q12 | A: building in WSL via `tools/wsl` |

## Wine on disk (W1)

**[verified in QEMU]** x86_64 (`wine` suite).

- `cargo xtask wine` puts the result into `target/wine/x86_64/tree` in the
  layout of an application directory: `bin/wineserver`,
  `lib/wine/x86_64-unix` (Unix `.so` files and the `wine` loader),
  `lib/wine/x86_64-windows` (PE DLLs), `share/wine` (`nls`, `wine.inf`).
  It is installed by `make install`, without debug information
  (`llvm-strip`) and without `.a` import libraries: 145 MB, 871 files
  (previously 454 MB for the DLLs alone).
- `cargo xtask run|test --wine` puts the tree on the data disk into
  `/apps/org.winehq.wine` (like a future package installation into
  `/apps/<id>`). The disk then has 512 MiB and is created once as a
  template (`target/wine/x86_64/datadisk-<key>.img`, key = the contents of
  the tree); a run gets a copy.
- Wine finds its directories relative to `ntdll.so`, no prefix is needed
  at build time. The `wine` loader finds `ntdll.so` through
  `realpath(argv[0])` (we have no `/proc/self/exe`): it is therefore
  started by an absolute path (`launchd`), `ntdll.so` is loaded by
  `dlopen` of an absolute path
  ([DL-4c](dynamic-linking.md#libraries-from-disk-and-origin-dl-4c)).
- Test (`cargo xtask test --only wine --wine`): `wine --version` via
  `launchd` prints `wine-11.0`. The `wine` suite runs only on request (the
  full run skips it without `--wine`).
- **The first console program** **[verified in QEMU]** x86_64 (`wine`
  suite, `--release`): `wine cmd /c echo ahoj` creates a new prefix
  (`wineboot`: `services.exe`, `rundll32 setupapi` over `wine.inf`, the
  registry, `C:\windows\system32`, the drives `dosdevices/c:` and `z:` as
  symbolic links) and `cmd.exe` prints `ahoj`, about 70 s in QEMU TCG in
  total. On the way it needed:
  - Wine patches: `0003` wineserver without `fork` (it starts itself in the
    foreground via `posix_spawn`), `0004` section memory as `memfd` (fsd
    files cannot be mapped `MAP_SHARED` with write yet), `0005` a new
    Windows process via `posix_spawn` of the loader (instead of a double
    `fork`),
  - `[app] jit = true` in the loader's manifest (`wine.manifest.toml`; Wine
    switches PE images RW -> RX, [the JIT gate](memory-management.md#wx-sealing-and-the-jit-permission)),
  - in `cstd`/`osrt`: `posix_spawn` normalises the path, the null fd,
    passing a connected `AF_UNIX` with messages in its queue, symbolic
    links, `mmap(PROT_WRITE)` = also read (wineserver maps
    `KUSER_SHARED_DATA` this way; without it time stood still for clients).
- **Our own console program** (`user/wintest`, a PE built with the Wine
  tools, on disk at `/apps/org.magnumopus.wintest`): `printf`, arguments
  (also with a space), environment (`getenv`, `GetEnvironmentVariableA`),
  a file in text mode (write, read, attributes, delete in `%TEMP%`), a
  thread (`CreateThread`, the thread's exit code) and exit code 42; about
  0.7 s with an existing prefix. `wine hostname` prints the computer name.
- ⚠️ **Limitation:** Wine waits for `wineboot` for a fixed 5 minutes. In a
  debug build (QEMU TCG) creating the prefix does not make it (reading from
  fsd about 1.7 MB/s versus 36 MB/s in release, `wine --version` 609 ms
  versus 31 ms) and `cmd` then ends with `c0000135`; the test is therefore
  slow and release-only. Starting a Wine process in release takes about
  0.7 s, mostly loading PE DLLs by reading the file through fsd in 32 KiB
  pieces (the file cannot be mapped).
- **Notes for W2:** extended attributes (`user.DOSATTRIB`) return `ENOSYS`,
  Wine works without them (DOS attributes from the name/permissions); no
  window driver (`nodrv`, "explorer failed to start") and no `RpcSs` -
  console programs do not mind.

## W1: server connection and processes (questions T1-T5)

**Status: [designed]**, decided on 15 Nov 2026 (coordinator); T2
**[verified in QEMU]**.

| Question | Decision | Who |
|---|---|---|
| T1 fds between processes | **A**: generic `AF_UNIX` in `cstd`/`osrt`. The socket path stays in the capability model: `bind` creates a socket-type node in fsd, `connect` needs the write right on the node; `socketpair` and `SCM_RIGHTS` = passing a handle in a channel. | scheduler thread (with `AF_UNIX` in `cstd`) |
| T2 a signal to a thread of another process | **A**: [`THREAD_INTERRUPT`](syscalls.md#thread_interrupt) (syscall 76), the `MANAGE` right on the thread handle; generic for `kill`/`pthread_kill`, Ctrl+C and `SIGCHLD`. | Wine thread (kernel, `cstd` delivery) |
| T3 memory of another process | **A in W2**: `PROCESS_MEMORY` (syscall **77 reserved**), rights `READ_MEMORY`/`WRITE_MEMORY` on the process handle. | Wine thread, stage W2 |
| T4 fd inheritance | **A**: `posix_spawn` inherits any fds without `FD_CLOEXEC` as handle duplicates in the start message. | scheduler thread |
| T5 address reservations | **A**: `AT_MO_RESERVE`, `ld.so` reserves ranges before loading libraries. Done even earlier: `spawn` reserves before mapping the program and `ld.so` ([dynamic-linking.md](dynamic-linking.md#address-reservations-before-loading-t5)), **[verified in QEMU]**. | Wine thread |

**T3 risk (stated explicitly):** `WRITE_MEMORY` gives the holder full
control over the target process (it can overwrite code, the stack and keys
in memory) and `READ_MEMORY` reads all its secrets. Both rights therefore go
only to the **process creator** (the handle from `PROCESS_CREATE`/spawn) or
a **debugger** the creator explicitly hands them to; a service never gives
them to another application and a handle from a process listing does not
have them. The wineserver gets them only for processes it started itself
(or that connect to it with a handle from the layer launcher).

Analysis of Wine 11 (`dlls/ntdll/unix/server.c`, `server/request.c`,
`server/ptrace.c`):

- **Connection:** the client connects to the server over an `AF_UNIX`
  socket in the prefix directory (or inherits a socket in the
  `WINESERVERSOCKET` variable when another Wine process starts it). Every
  thread sends the server its request and reply pipes; files, pipes and
  mappings travel both ways as fds over `SCM_RIGHTS`. The server waits on
  all fds in one `poll` loop.
- **Signals to threads of another process:** the server suspends a thread
  with `SIGUSR1` (the thread sends the server its context in the handler
  and waits) and terminates it with `SIGQUIT` (`tkill` on pid and tid).
- **Memory of another process:** `ReadProcessMemory` and
  `WriteProcessMemory` via `ptrace` and `/proc/<pid>/mem` (debugging, some
  server queries).
- **Process start:** the `wine` loader and `wine-preloader` reserve address
  ranges for Windows (PE images from 0x10000, `KUSER_SHARED_DATA` at
  0x7ffe0000) before the dynamic loader loads libraries.

### T1 - passing fds between processes

- **A (recommended):** generic **`AF_UNIX` in `cstd`/`osrt`**:
  `socketpair`, `bind`/`connect` to a path (a socket node in fsd or a name
  service), `sendmsg`/`recvmsg` with `SCM_RIGHTS`, where an fd is passed as
  a handle duplicate in a channel (file = fsd session, pipe = channel end,
  memory = VMO). Wine code stays unchanged and other ports need the same
  mechanism (X11, D-Bus, Wayland).
- **B:** a Wine patch: connection and passing via our own functions over
  channels in `server.c`, `request.c`, `fd.c`. Passing open files between
  processes is still needed - just not in the POSIX form.
- **C:** only `socketpair` and an inherited socket (no `bind`/`connect` to
  a path): enough if every process of the application is started by the
  launcher or the server.

### T2 - a signal to a thread of another process

- **A (recommended):** a generic **thread interruption in the kernel**:
  `THREAD_INTERRUPT(thread, code)` with a thread handle right; the target
  thread gets a synchronous delivery on its in-thread handler (kind
  `INTERRUPT` with the code) on return to user space, even from a blocking
  syscall (which returns `INTERRUPTED`, like `EINTR`). `cstd` builds `kill`,
  `pthread_kill` and `tgkill` between processes on it (also missing for
  terminal Ctrl+C and `SIGCHLD`). The server gets the client thread
  handles at `init_thread`.
- **B:** without the kernel: the server suspends the thread
  (`THREAD_SUSPEND`) and rewrites its context to jump into the handler
  (like injection via `ptrace`). Fragile in the middle of a syscall and
  without `EINTR`.
- **C:** suspension only via `THREAD_SUSPEND` and the server reading the
  context (`THREAD_STATE`) - but Wine wants the thread to send the context
  itself (a patch of the suspension handshake in the server and `ntdll`).

### T3 - memory of another process

- **A (recommended):** a syscall `PROCESS_MEMORY(process, addr, buf, len,
  flags)` with process handle rights `READ_MEMORY` / `WRITE_MEMORY`
  (debuggers, `ReadProcessMemory`), a copy through the kernel without
  mapping; stage W2.
- **B:** only for processes of the same application via shared VMOs -
  does not cover arbitrary addresses.

### T4 - process start and fd inheritance

`posix_spawn` in `cstd` (the scheduler thread) plans only fds 0-2 so far.
Wine gives the child the server socket as another fd and the
`WINESERVERSOCKET` variable.

- **A (recommended):** `posix_spawn` with `dup2`/`close` actions for any fd
  (fds without `FD_CLOEXEC` are inherited as handle duplicates in the
  start message), as in POSIX.
- **B:** only the inherited server socket under a special start message
  tag (a Wine patch in `loader.c`).

### T5 - address reservations for Windows

The ranges for PE images (the lower 2 GiB, the usual base 0x140000000),
the system shared data at 0x7ffe0000 and stacks must be free before
`ld.so` loads libraries at random bases.

- **A (recommended):** `ld.so` gets a list of ranges to reserve before
  loading libraries (auxv `AT_MO_RESERVE` from the manifest or from the
  layer launcher) and hands them to the program as reservations
  (`VM_RESERVE`) that Wine takes over (instead of `wine-preloader`).
- **B:** library and stack bases always above 4 GiB for programs with a
  manifest flag (simpler, but a fixed rule in the loader).

## What the system adds

| Where | What | Stage |
|---|---|---|
| kernel | a per-thread program GS, GS in the context (`THREAD_STATE`): `THREAD_SET_POINTER` with `SECOND` ([syscalls](syscalls.md#thread_set_pointer)) **[verified in QEMU]** x86_64 and aarch64 (`NOT_SUPPORTED`) | W1 |
| kernel | a ring 3 32-bit code segment, exceptions from compat mode | W6 |
| memory management | the `wx_compat` permission (per Q6) | W1 |
| fsd | a casefold dataset (per Q7) | W2 |
| libos | connecting the application processes to the server (channels with handles), starting a Windows process (`CreateProcess` -> starting the launcher with the same prefix) | W1-W2 |
| compositor | several windows of one application with owner links, borderless popups, window position from the application (`SetWindowPos`) | W3 |

## Security and isolation

- Every application has its own prefix (= virtual `C:` and registry) and
  its own wineserver; processes of different applications share no state.
  Uninstall = delete the prefix.
- An application gets only the rights from its manifest (network, Working
  folders, sound, camera) like a native application; the layer itself has
  no extra rights.
- Wine code is LGPL-2.1 and runs in the application process and in
  wineserver (not in the kernel or system services); we meet the LGPL
  obligations as for FFmpeg (sources and patches, replaceable shared
  libraries).
- ⚠️ **Anti-cheat** (EasyAntiCheat, BattlEye, Vanguard) usually does not
  run on an unknown system - the game vendor decides, not the technology;
  kernel anti-cheats will never work.

## Stages

| Stage | Goal | Done when |
|---|---|---|
| W0 | building Wine 11 (PE and the Unix side) in xtask, licences in `THIRD-PARTY.md` | `cargo xtask build` produces the DLLs and `.so` for x86_64 |
| W1 | console `.exe`: GS, SEH, memory, files, wineserver | test: our own console program and `cmd /c echo` from Wine in QEMU |
| W2 | processes, registry, named objects | `reg query`, two processes with a named event |
| W3 | windows, GDI, input, clipboard | `notepad` (typing, saving a file), `winecfg` |
| W4 | sound, a simple game | `winemine` (GDI), a tone via `mmdevapi` |
| W5 | DXVK, VKD3D-Proton | a Direct3D 11 game over Venus |
| W6 | WoW64 | a 32-bit `notepad` and an installer |
| W7 | synchronization without the server, performance | measured frames of a game against W5 |

## Risks

- **Scope:** Wine is huge and every application finds a different hole;
  the goal is to connect the interface so that fixes come from upstream,
  not to write Win32.
- **The Wine Unix side assumes Linux/macOS** in many places (`/proc`,
  signals between processes, fixed addresses for ntdll and stacks): each
  place is either in our module (Q2 A) or needs an addition to `cstd`.
- **Build speed:** Wine takes tens of minutes to build; xtask rebuilds
  it only when the version or patches change (like C ports).
- **Graphics for games** depends on Vulkan on real HW (step 9); in QEMU
  only Venus.
