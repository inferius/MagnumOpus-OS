# Windows layer: Wine over our lower layer

Design of step 8 of the order in [architecture.md](architecture.md) (the
plan "upper layer from Wine, lower layer ours"): Windows applications and
games over adopted Wine. **Status: [designed]**, questions decided on 15 Nov 2026
(chapter [Decisions](#decisions)); stage W1 is starting.

## Status

| Part | What | Status |
|---|---|---|
| W0 | building Wine 11 (PE DLLs, tools), port of the Unix side over `libc.so` | DLLs as PE (602 libraries) via `cargo xtask wine` in WSL **[implemented]**; the Unix side **[designed]** (error overview with `--keep-going`) |
| W1 | console `.exe` (`cmd`, `hostname`, our own test): lower `ntdll`, TEB via GS, SEH, memory, files, `wineserver` as a process | **[designed]** |
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

## What the system adds

| Where | What | Stage |
|---|---|---|
| kernel | a per-thread program GS, GS in the context (`THREAD_STATE`) | W1 |
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
