# Dynamic linking

ELF shared libraries (`ET_DYN`) and PIE programs: the `ld.so` loader,
relocations, TLS of shared libraries, `dlopen`, sharing library code
between processes and the RELRO seal. It is step 4 in the order of adopting
foreign software ([architecture.md](architecture.md#security)): the Unix
part of Wine and FFmpeg (LGPL, dynamically) need it. Rust programs stay
static (`ET_EXEC`, [rust-std.md](rust-std.md)).

## Status

| Part | What | Status |
|---|---|---|
| DL-1 | the `ET_DYN` parser in `libs/elf`: program headers (`PT_INTERP`, `PT_TLS`, `PT_GNU_RELRO`, `PT_DYNAMIC`), the dynamic section, symbols with GNU and SysV hash, RELA and RELR relocations of both architectures, rejecting TEXTREL | **[implemented]** (host tests `cargo test -p elf` against files from clang and ld.lld) |
| DL-2 | the `user/ld` + `libs/dynload` loader: `PT_INTERP` in `spawn`, ASLR, `DT_NEEDED`, `BIND_NOW`, relocations, static TLS of the main thread, read-only RELRO; PIC and linking via `ld.lld` in `cbuild`, libraries in initfs (`lib/`) | **[verified in QEMU]** x86_64 and aarch64 (`procesy` suite, `dynamicke linkovani: ...` tests) |
| DL-3 | the loader table (`abi::loader`, `dl*` already in the ABI), TLS of new threads (libos, osrt), DTV and generations, static TLS surplus, `fini`, `dl_iterate_phdr`, `dladdr`, `r_debug` | **[verified in QEMU]** x86_64 and aarch64 (test `dynamicke linkovani: vlakna libos ...`) |
| DL-4a | the `libd` library service (sealed templates, sharing library code between processes), a libd connection for every process (anyone can start a program with the loader), region sealing (`PROT_SEAL`, like `mseal`) for code and RELRO | **[verified in QEMU]** x86_64 and aarch64 (`pamet`, `procesy` suites) |
| DL-4b | `dlopen`/`dlsym`/`dlclose` via libd (dynamic TLS: lazy allocation via the DTV, dynamic TLSDESC), static TLS reserve from the manifest (`[app] static_tls_reserve`) and measurement | **[verified in QEMU]** x86_64 and aarch64 (`procesy` suite) |
| DL-4c | libraries from disk and `$ORIGIN` (once applications run from disk `/apps/<id>/`) | **[designed]** |

## Design (approved choices)

<figure>
<svg viewBox="0 0 760 200" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Starting a PIE program">
<style>
.box{fill:#f4f6fb;stroke:#4a5a80;stroke-width:1.2}
.acc{fill:#e8f1e4;stroke:#3f7a3a;stroke-width:1.2}
.t{font:13px sans-serif;fill:#1d2333}
.m{font:11px sans-serif;fill:#3a4256}
.b{font-weight:bold}
.ln{stroke:#4a5a80;stroke-width:1.2;fill:none}
@media (prefers-color-scheme: dark){
.box{fill:#1f2533;stroke:#8fa2d0}.acc{fill:#1f2e22;stroke:#7fbf78}
.t{fill:#e6e9f2}.m{fill:#c3c9d9}.ln{stroke:#8fa2d0}}
</style>
<defs><marker id="dl1e" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto"><path d="M0,0 L8,4 L0,8 z" fill="#4a5a80"/></marker></defs>
<rect class="box" x="5" y="30" width="130" height="60" rx="6"/><text class="t b" x="70" y="55" text-anchor="middle">spawn (parent)</text><text class="m" x="70" y="73" text-anchor="middle">program + ld.so</text>
<rect class="acc" x="165" y="30" width="130" height="60" rx="6"/><text class="t b" x="230" y="55" text-anchor="middle">ld.so (child)</text><text class="m" x="230" y="73" text-anchor="middle">PT_INTERP, ASLR</text>
<rect class="acc" x="325" y="30" width="130" height="60" rx="6"/><text class="t b" x="390" y="55" text-anchor="middle">DT_NEEDED</text><text class="m" x="390" y="73" text-anchor="middle">/system/lib, $ORIGIN</text>
<rect class="acc" x="485" y="30" width="130" height="60" rx="6"/><text class="t b" x="550" y="55" text-anchor="middle">relocations</text><text class="m" x="550" y="73" text-anchor="middle">BIND_NOW, TLS</text>
<rect class="box" x="645" y="30" width="110" height="60" rx="6"/><text class="t b" x="700" y="55" text-anchor="middle">RELRO R--</text><text class="m" x="700" y="73" text-anchor="middle">then program entry</text>
<line class="ln" x1="135" y1="60" x2="161" y2="60" marker-end="url(#dl1e)"/>
<line class="ln" x1="295" y1="60" x2="321" y2="60" marker-end="url(#dl1e)"/>
<line class="ln" x1="455" y1="60" x2="481" y2="60" marker-end="url(#dl1e)"/>
<line class="ln" x1="615" y1="60" x2="641" y2="60" marker-end="url(#dl1e)"/>
<text class="m" x="5" y="130">Library code from sealed templates (libd, shared pages), data and GOT as copy-on-write.</text>
<text class="m" x="5" y="150">DL-1 (this part): the libs/elf parser from which ld.so reads headers, symbols and relocations.</text>
</svg>
<figcaption>Starting a PIE program with shared libraries (DL-2 to DL-4b; libraries from disk and <code>$ORIGIN</code> in DL-4c).</figcaption>
</figure>

- **Loader:** a separate `ld.so`; `libc.so` (cstd) is an ordinary library
  from `DT_NEEDED` (decision 2A in [cstd.md](cstd.md)), the
  program has `PT_INTERP`; the loader is written in Rust (`libs/dynload`,
  `user/ld`), statically relocated itself.
- **Format:** ELF `ET_DYN` per the SysV gABI: `DT_GNU_HASH` and `DT_HASH`,
  RELA and RELR, `DT_RUNPATH`, `SONAME`. Only `BIND_NOW` and full RELRO (no
  lazy PLT). **`DT_TEXTREL` is rejected** (writing into executable memory
  contradicts W^X). TLS: local-exec, initial-exec, general/local-dynamic via
  `__tls_get_addr`, TLSDESC (the default on aarch64). Symbol versions
  (`DT_VERSYM`) are read but not evaluated; a hidden version only removes
  the symbol from lookups.
- **Where libraries live:** `/system/lib` (an immutable part of the image)
  and private application libraries via `$ORIGIN`; `LD_LIBRARY_PATH` only
  for development.
- **Versions:** the file is named exactly like the `SONAME` with the major
  version (`libavcodec.so.61`), no symlink chains.
- **ASLR:** a random base of every `ET_DYN` in every process, 64 KiB
  alignment.
- **Rust stays static**; C/C++ (`cstd` as `libc.so`) and `cdylib` plugins
  are dynamic.
- **RELRO:** after relocation `VM_PROTECT` to R-- and lowering `max_flags`
  (like `mseal`).

## The `ld.so` loader (DL-2)

**[verified in QEMU]** x86_64 and aarch64.

**Starting a PIE program.** `spawn` (libos) recognises `ET_DYN` with
`PT_INTERP` = `/system/lib/ld.so` (`abi::auxv::INTERP`; another loader =
`NOT_SUPPORTED`, a library without a loader does not start):

1. It maps the program and the loader (`lib/ld.so` from the caller's
   initfs) at **random bases** (`spawn::random_base`: a reservation of the
   whole module range in `0x2000_0000_0000..0x5000_0000_0000`, 64 KiB
   alignment, ~29 bits); program code from initfs is still shared through
   templates.
2. It writes a SysV block to the top of the stack (`abi::auxv`):
   `argc = 0`, empty `argv` and `envp`, `AT_PHDR`, `AT_PHENT`, `AT_PHNUM`,
   `AT_PAGESZ`, `AT_BASE`, `AT_ENTRY`, `AT_RANDOM` (16 random bytes). The
   numbers are SysV ones, so adopted `getauxval` works.
3. It puts a **loader message** (`abi::bootstrap::LOADER_MAGIC`, "OSLD")
   into the bootstrap channel **before** the start message: a process
   handle and a read-only initfs (`READ | MAP`). The start message and the
   channel stay with the program (it keeps talking to the parent through
   it).
4. The thread starts at the loader entry (`rdi`/`x0` = bootstrap, SP = the
   block).

**The loader** (`user/ld`, Rust, static-PIE: `relocation-model=pie`,
`-pie --no-dynamic-linker`, only relative relocations):

1. The assembly entry computes the base (`__ehdr_start`) and `_DYNAMIC`
   PC-relatively and **relocates itself** (RELA `R_*_RELATIVE` and RELR)
   before touching any data with pointers.
2. It reads the loader message and auxv; it sees the program as a loaded
   module (`ElfFile::parse_loaded`).
3. Libraries from `DT_NEEDED` **breadth-first**, each once (name or
   `SONAME`), from initfs `lib/<name>` (`dynload::initfs_path`;
   `/system/lib/x` = `lib/x`, other paths are rejected). Each gets its own
   random base; segments without write are **sealed** (W^X: only sealed
   memory can execute).
4. **Static TLS** of all modules (`dynload::tls_layout`: x86_64 variant II
   with the TCB after TP, aarch64 variant I with a 16 B TCB) - the program
   is the first module, so its local-exec matches the linker. It sets the
   thread pointer.
5. **Relocations**, dependencies first, the program last: `RELATIVE`,
   `ABS64`, `GLOB_DAT`, `JUMP_SLOT` (all at once), `IRELATIVE`, TLS
   (`DTPMOD`/`DTPOFF`/`TPOFF`, TLSDESC with a static resolver, a built-in
   `__tls_get_addr`), RELR. Symbols in the order program, libraries (local
   and hidden symbols belong to the module itself); an undefined weak one
   = 0, an undefined strong one = an error.
6. It switches RELRO to read-only (`VM_PROTECT`), calls `DT_INIT` and
   `DT_INIT_ARRAY` (dependencies first, the program last).
7. It unmaps initfs, closes its handles and jumps to the program entry with
   the same bootstrap and SP (the program can read auxv).

An error (missing library, symbol, bad file) = `ld.so: ...` in the log and
the process exits with code 127 (`abi::auxv::EXIT_LOAD_FAILED`).

**C programs and libraries** are built via `cbuild::Build::link`
([c-build.md](c-build.md)): programs `-fPIE` (the program's TLS
local-exec, foreign initial-exec), libraries `-fPIC` (TLS general-dynamic,
TLSDESC on aarch64). A PIE program links with `--allow-shlib-undefined`
(loader functions such as `__tls_get_addr` exist nowhere at link time).
Test modules are in `user/dltest` (a minimal `crt.c`: `_start` ->
`program_main` -> exit syscall). C programs over `libc.so` (cstd) have
`crt1.o` from the C sysroot and the `__libc_start` start in `osrt`, see
[cstd.md](cstd.md#libcso).

⚠️ **Limitations of DL-2 (deliberate, solved by DL-3 and DL-4):**

- Sharing library code and starting by anyone is solved by DL-4a (`libd`,
  above); libraries from disk and `$ORIGIN` by DL-4c.
- TLS of new threads and fini are solved by DL-3a (above).
- RELRO is sealed since DL-4a (`PROT_SEAL`).
- The loader has all of initfs (read-only) while loading; the program does
  not get it (the loader unmaps it and closes the handles before the jump).

## The library service and sealing (DL-4a)

**[verified in QEMU]** x86_64 and aarch64.

- **`libd`** (`user/libd`, protocol `libs/libdproto`) holds templates of
  shared libraries from `/system/lib` (initfs `lib/<name>`): segments
  without write as **sealed VMOs** (all processes map the same code pages),
  data ones as a template for a copy-on-write clone (`VMO_CLONE`, zeros up
  to `mem_size` included). A template is found by name and content
  (BLAKE3). The client gets code handles `READ | MAP | EXECUTE` and data
  handles `READ | MAP`, never write.
- **Sessions and leases:** see [Sessions with libd and recovery](#sessions-with-libd-and-recovery-dl-4b)
  (DL-4b): only the loader of a program with the loader and a launcher (a
  static program with `spawn::Options::libd`) get a libd session and a lease
  to it.
- `spawn` also takes **`ld.so`** itself from libd (shared loader code); the
  process caches the template (the VMOs are immutable).
- **Sealing** (`PROT_SEAL` in `VM_PROTECT`, like Linux `mseal`,
  [syscalls.md](syscalls.md#vm_protect)): a sealed region cannot be
  unmapped, released, have its rights changed or be mapped over, until the
  process ends. `spawn` seals the program and loader code, the loader seals
  library code and the RELRO of the program, the libraries and itself after
  relocation (so the GOT cannot be switched back to RW or replaced with a new
  anonymous mapping).
- **Untrusted input:** names only `A-Z a-z 0-9 . _ + -` without a leading
  dot, at most 255 B (`libdproto::valid_name`, a decoder fuzz test); a file
  at most 256 MiB, at most 16 segments, ELF via `libs/elf` (TEXTREL, REL
  rejected, headers in the first segment). Sessions: at most 256 live ones
  created from one session (per process, init's root without a limit) and
  4096 in total.

## Sessions with libd and recovery (DL-4b)

**[verified in QEMU]** x86_64 and aarch64.

- **libd is a supervisor service** (`user/init/src/libd.rs`, the
  `LibdService` adapter, [Service recovery](service-recovery.md)): after a
  crash the supervisor starts it again (backoff, attempt limit) and clients
  reconnect through leases. The recipe is empty (a libd session has no
  identity and no extra rights).
- **Who has a session** (`bootstrap::kind::LIBD`: arg 0 session, arg 1
  lease):
  - the **loader** of every program with the loader (in the loader message),
  - a **launcher**: a static program started with `spawn::Options::libd`
    (init gives it to `launchd` and `termd`),
  - a **program under the loader** only at its first spawn of a program
    with the loader (`abi::loader::Interface::libd_session`: the loader
    makes another one for it),
  - **init** holds the root connection (libd's bootstrap channel).
  - **A static program without the option has nothing** and its spawn talks
    to neither libd nor init. std applications start programs through
    `launchd` (`osrt::process`), so they do not need libd.
- **A session and a lease for a child without waiting**
  (`libos::libd::child`): the parent sends `Connect` on its session (libd
  creates the child's session, no reply) and `ADOPT` on its lease (the
  supervisor takes over a new lease with the same recipe, no reply;
  [protocol](service-recovery.md#the-lease-protocol)). The child can use
  both right away: a message on the lease waits in the channel until the
  supervisor takes it over. Init gives its children a session from the
  root and writes the lease to the supervisor directly
  (`supervisor::queue_lease`, even inside its round).
- **Recovery:** `PEER_CLOSED` on the session = `RECONNECT` on the lease and
  the request repeated (`Open`, `Stats` are idempotent). This holds for
  `spawn` and for `dlopen` in the loader (the same `libos::libd` client).
  Ordering test: libd crashes after the spawn, **before the supervisor
  processes `ADOPT`**, and the child still recovers its `dlopen`.
- **A rejection without a reply shows at first use**, never as a hang:
  - libd closes the session (more than 256 sessions from one session = per
    process, or 4096 in total): `RECONNECT` returns `PEER_CLOSED` within
    0.2 s (the instance keeps running = a deliberate close), the lease
    ends, `dlopen`/spawn return the reason ("libd relaci odmitla (limit
    relaci procesu)"),
  - the supervisor rejects `ADOPT` (more than 256 live adopted leases from
    one lease or more than 128 per second, burst 128;
    `leaseproto::adopt`): it closes the lease endpoint and logs
    `init: listek libd: ADOPT odmitnut (...)`; the child gets
    `SERVICE_UNAVAILABLE` only when it needs the lease.
- **Measurement** (test `rezie spawnu a propustnost initu`, program
  `user/dlspawn`; **QEMU TCG** on Windows, 4 vCPUs, debug - on real hardware
  the times are an order of magnitude smaller, the ratios similar). Average
  per spawn:

  | | x86_64 | aarch64 | revision |
  |---|---|---|---|
  | spawn of a program with the loader, 1 launcher × 200 | 5975 µs | 6112 µs | `bd7e656` |
  | of which the child's session and lease (`Connect` + `ADOPT`) | 659 µs | 595 µs | `bd7e656` |
  | the same, 4 launchers × 50 concurrently | 5473 µs | 7170 µs | `bd7e656` |
  | static spawn (no libd) | 1952 µs | 1861 µs | `bd7e656` |
  | round trip to libd (what DL-4a added to every spawn) | 1342 µs | 1301 µs | `bd7e656` |

  The variant with `NARROW` (the parent waits for init's reply, an
  uncommitted intermediate stage based on `df51756`) was much more
  expensive - hence `ADOPT` without a reply:

  | `NARROW` | x86_64 | aarch64 | revision |
  |---|---|---|---|
  | spawn of a program with the loader, 1 × 200 | 6911 µs | 7639 µs | `df51756` + A |
  | of which `NARROW` | 1830 µs (26 %) | 1785 µs (23 %) | `df51756` + A |
  | `NARROW` share at 4 × 50 concurrently | 35 % | 31 % | `df51756` + A |

  The rest after `ADOPT` (`Connect` + `ADOPT`, about a tenth of the spawn)
  are two channel creations and two writes in the parent process; nothing
  waits for init or libd. The numbers vary between runs by about a fifth
  (TCG). DL-4a (a connection
  for every child) added one round trip to libd to every static spawn.

## dlopen and dynamic TLS (DL-4b)

**[verified in QEMU]** x86_64 and aarch64.

- **`dlopen(path, flags)`**: the path only `/system/lib/<name>` or
  `<name>` (libraries from `libd`; disk and `$ORIGIN` in DL-4c), null = the
  program (global scope). An already loaded module (name or `SONAME`)
  returns the same handle and increments the counter. A new module and its
  missing dependencies are loaded from libd, **relocated immediately**
  (`LAZY` = `NOW`) in the scope global scope (program, start-up libraries,
  modules with `RTLD_GLOBAL`) + the library's local group (itself and its
  dependencies breadth-first), code and RELRO are sealed and the
  initializers are called (dependencies first). `RTLD_NOLOAD` only returns
  the handle of a loaded module. **An error anywhere** (missing library,
  undefined symbol, not enough TLS reserve) = nothing changes: new modules
  are discarded (not sealed yet), the TLS reserve is returned and the
  reason goes to `dlerror`.
- **`dlsym(handle, name)`**: searches the module's local group,
  `RTLD_DEFAULT` (0) and the program handle in the global scope; a TLS
  variable = the address in the calling thread (a lazy block is created if
  needed), IFUNC = the resolver's result. `RTLD_NEXT` does not exist yet
  (error).
- **`dlclose`** decrements the counter; the module stays loaded (its code
  is sealed, TLS module numbers are never reused) - like `RTLD_NODELETE`
  for all.
- **`dlerror`** is per thread (by TP); the string is valid until the next
  `dl*` in the thread, reading clears the error. `dl*` calls are
  serialized by a **recursive** lock: an initializer may call `dlopen`.
- **TLS of a `dlopen` module:**
  - **initial-exec** (`DF_STATIC_TLS` or TP-offset relocations): a piece of
    the **static TLS reserve** (`dynload::Surplus`) - the same TP offset in
    all threads. The loader copies the template into the blocks of all
    living threads (block list) and fills their DTV; new threads get it in
    `thread_tls_new`. Alignment above the TP alignment is impossible.
  - **others** (general-dynamic, TLSDESC): a **lazy block** via the DTV.
    The entry stays empty until the thread touches the module's TLS: the
    slow path of `__tls_get_addr` (x86_64 GD) or of the **dynamic TLSDESC
    resolver** (aarch64 always, x86_64 `-mtls-dialect=gnu2`) allocates the
    block (template, zeros to the end), writes it to the DTV and frees it
    when the thread ends. The resolver's fast path only reads the DTV; the
    slow path saves every register a called Rust function may change -
    x86_64 `rcx rdx rsi rdi r8-r11` and `xmm0-15` (the loader is built
    without AVX, so it does not change the upper halves of `ymm`/`zmm`),
    aarch64 `x1-x18` and `q0-q31`. The descriptor ABI changes only
    `rax`/`x0` (and flags).
  - The DTV has a fixed size (256 modules with TLS) and numbers are never
    reused, so an empty entry always means "no block yet" (glibc also
    compares generations because it recycles numbers after `dlclose`).
  - TLS access with no way to return an error (no memory for a lazy block,
    an unknown module) ends the process, like glibc.
- **Static TLS reserve** per process: manifest `[app] static_tls_reserve`
  (e.g. `"8K"`, at most 64 KiB, see [the manifest](firewall.md#application-manifest))
  -> init `spawn::Options::static_tls_reserve` -> auxv
  `abi::loader::AT_MO_TLS_RESERVE` -> the loader (`dynload::tls_reserve`;
  without a value the default 2 KiB). The reserve is in the block of
  **every** thread, hence the upper bound.
- **Reserve measurement:** the loader logs the usage at every `dlopen` of a
  module that used the reserve, and at process end (only when it was used):

  ```text
  ld.so: dlopen libdlstatic.so.1 (statické TLS 4 B): rezerva statickeho TLS pouzito 4 z 2048 B
  ld.so: konec procesu: rezerva statickeho TLS pouzito 4 z 2048 B
  ```

- **An exhausted reserve** = `dlopen` fails and `dlerror` says what to do:

  ```text
  dlopen: libdlbig.so.1: staticke TLS (initial-exec) potrebuje 4096 B (zarovnani 1), v rezerve zbyva 2048 z 2048 B; zvyste [app] static_tls_reserve v manifestu aplikace (nejvys 64 KiB)
  ```

- **libos:** `libos::loader::{dlopen, dlsym, dlclose}` (the error as text
  from `dlerror`), `Library` = a handle.
- **The loader's allocator** (`user/ld/src/heap.rs`) frees memory
  (`linked_list_allocator` over an address reservation chosen by the
  kernel).
- `r_debug` is extended at `dlopen` (`RT_ADD`, `_dl_debug_state`,
  `RT_CONSISTENT`, `_dl_debug_state`); `dl_iterate_phdr` sees the new
  modules (`adds`).
- ⚠️ **Limitations:** no `RTLD_NEXT`; modules are never removed (library
  memory stays until the process ends); finalizers of `dlopen` modules run
  only at process end and in load order (not by dependencies on older
  modules); C code calling `__tls_get_addr` with a stack not aligned to
  16 B (old GCC) could crash on the slow path (clang is fine).

## Threads, fini and the debugger (DL-3)

**[verified in QEMU]** x86_64 and aarch64.

- **The loader table** `abi::loader::Interface` (version, size;
  `thread_tls_new`, `thread_tls_free`, `run_fini`, `dl_iterate_phdr`,
  `dl*`, `libd_session`, `run_init`; new fields only at the end, the test
  `loader_table_offsets` checks the offsets). The
  loader writes its address into auxv (`AT_MO_LOADER`, `spawn` prepares the
  slot) and when jumping to the program also passes SP (auxv) and
  `ENTRY_MAGIC` (x86_64 `rsi`/`rdx`, aarch64 `x1`/`x2`). libos (`rt_own` ->
  `loader::init`) and crt find it that way; a program without the loader has
  `loader::interface() = None` and behaves as before.
- **TLS of new threads:** under the loader `libos::thread::spawn` takes a
  block (`thread_tls_new`: copies of all modules' templates, the surplus, the
  TCB on x86_64) and sets the thread pointer in the trampoline before the
  thread's code; `join` returns the block. `osrt` (Rust std) creates no block
  of its own under the loader.
- **Static TLS surplus** (default `dynload::STATIC_TLS_SURPLUS` = 2 KiB,
  configurable since DL-4b, see below) after the
  modules loaded at start: for a later `dlopen` of modules with
  initial-exec TLS (Vulkan/GL drivers from Mesa). glibc has a fixed 1664 B
  surplus, today 512 B + 144 B per namespace (1520 B); 2 KiB covers both.
- **init** **[verified in QEMU]** x86_64 and aarch64 (suites `procesy`, `cstd dyn`,
  `cpp`): the loader does not run the initializers of the start-up modules
  (`DT_INIT`, `DT_INIT_ARRAY`, dependencies first, the program last) before
  jumping to the program. The program runtime calls them through the table
  function `run_init` right after its own initialization and before `main`
  (`rt_own` after `startup`, `__libc_start` in osrt after `init`, the test
  crt before `program_main`): constructors of C++ static objects allocate,
  register `__cxa_atexit` and touch files, which is not possible before the
  heap and stdio are ready ([C++ runtime](cpp.md)). A second call does
  nothing; `dlopen` initializes its modules itself. The field is at the end
  of the table (7a-1); a program looks for it by `size`.
- **fini:** a normal program end (`rt_own` after `main`, crt after
  `program_main`) calls `run_fini`: the program first, dependencies last,
  `DT_FINI_ARRAY` backwards, then `DT_FINI`; a second call does nothing.
  Kill and crashes do not call fini; neither does a direct `PROCESS_EXIT`.
- **`dl_iterate_phdr`** (`struct dl_phdr_info` layout as glibc, for unwinders
  and adopted code): in the table and as a built-in symbol for C; without
  allocation, the callback runs without the lock (it may call `dlopen`).
  Modules: the program, start-up libraries and `dlopen` ones (not the
  loader itself).
- **`r_debug`** (SVR4, `link_map` program and libraries, `r_brk` =
  `_dl_debug_state`): its address in the program's `DT_DEBUG` (written
  before RELRO).
- **DTV and generations** (glibc model, preparation for `dlopen`): each
  thread has a vector `[generation, block of module 1, 2, ...]` in its TLS
  block, a pointer to it in the TCB (x86_64 `TP + 8`, aarch64 `TP + 0`).
  `__tls_get_addr` reads the block from the DTV; modules loaded at start
  have it in the static TLS. Lazy allocation of a `dlopen` module's block:
  see DL-4b.
- **`dlopen`, `dlsym`, `dlclose`, `dlerror`, `dladdr`** are in the table and
  as built-in symbols for C (the ABI did not change in DL-4b). `dladdr`
  finds the module by address (no symbol yet).
- The loader **keeps the handle of its own process** (TLS blocks of
  threads, `dlopen`); the program has a handle to the same process anyway,
  so it gains no rights.

## The `libs/elf` parser (DL-1)

**[implemented]** The file is untrusted: every read is bounds-checked, bad
input returns an `ElfError` and never panics (test
`damaged_input_does_not_panic`: truncation and thousands of randomly changed
bytes).

| API | What |
|---|---|
| `ElfFile::parse(data)` | a static program (`ET_EXEC`) for this architecture - loader, kernel, `spawn` (unchanged) |
| `ElfFile::parse_object(data, Machine)` | `ET_EXEC` and `ET_DYN` for the given architecture (host tests handle both) |
| `program_headers`, `load_segments` (with `align`), `interp`, `tls` + `tls_data`, `relro` | program headers |
| `dynamic()` -> `Dynamic` | `DT_NEEDED` (at most 32), `SONAME`, `RUNPATH` (else `RPATH`), tables, `init`/`fini` and arrays, flags (`bind_now`, `pie`, `static_tls`), versions; **rejects `DT_TEXTREL`/`DF_TEXTREL` (`ElfError::TextRelocations`) and `DT_REL`** |
| `symbols(&Dynamic)` -> `SymbolTable` | `get(i)`, `lookup(name)` via GNU hash (Bloom filter), else SysV; returns only exported symbols (global/weak, default/protected visibility, version not hidden) |
| `relas(&Dynamic)` | RELA and PLT as `Rela { offset, kind, sym, addend }`; `Kind` shared by both architectures (`Relative`, `Absolute`, `GlobDat`, `JumpSlot`, `TlsModule`, `TlsOffset`, `TlsTpOffset`, `TlsDesc`, `IRelative`, `Copy`, `Unknown`) |
| `relrs(&Dynamic)` | addresses of relative relocations from RELR (the `Relr` decoder) |
| `at_vaddr(addr, len)` | file data at a virtual address (must lie in the data of one `PT_LOAD`) |

Test files (`libs/elf/tests/fixtures/`, C source and `gen.sh`): a library
with RELA and both hashes, the same with RELR and only the GNU hash, a PIE
program with `PT_INTERP`, a library with TEXTREL (x86_64 only - lld does
not produce it on aarch64). They are built with clang and ld.lld with
`max-page-size=65536` and committed; the tests do not need clang.

## Tests

| Test (`cargo test -p elf`) | Checks |
|---|---|
| `shared_library_headers` | `PT_TLS` and its data, RELRO, `DT_NEEDED`, `SONAME`, `RUNPATH $ORIGIN`, `BIND_NOW`, 64 KiB alignment (x86_64 and aarch64) |
| `symbol_lookup_gnu_and_sysv` | lookup via GNU and SysV-only hash, symbol kind and value, hidden and undefined symbols are not found |
| `relocations_x86_64`, `relocations_aarch64` | relocation kinds and their symbols (TLS GD with `__tls_get_addr`, TLSDESC on aarch64), `Rela::value` |
| `relr_matches_relative_relocations`, `relr_decoder` | RELR from the file and synthetic bitmaps (an empty bitmap also advances 63 words) |
| `pie_program` | `PT_INTERP`, `DF_1_PIE`, `ElfFile::parse` rejects PIE |
| `rejects_textrel_and_wrong_machine` | TEXTREL, another architecture, a short file |
| `hash_functions_match_reference` | GNU and SysV hash against known values |
| `damaged_input_does_not_panic` | truncation and random changes without panics |
| `loaded_module_view` | a module loaded in memory (`parse_loaded`) gives the same data as the file |
| `cargo test -p dynload` | TLS layout against linker offsets (both variants), values of all relocation kinds, library paths |
| `cargo test -p cbuild` (`link_skutecne_moduly`) | linking a library and a PIE via `ld.lld` on both architectures |
| QEMU `procesy`: `dynamicke linkovani: PIE s knihovnami` | `dltest`: calls through two libraries, a function pointer (ABS64), an initializer after the dependencies are relocated, library TLS (GD/TLSDESC), library TLS from the program (initial-exec), program TLS (local-exec) |
| QEMU `procesy`: `ASLR - dva behy, ruzne baze` | two `dltest` runs have different program bases |
| QEMU `procesy`: `chybejici knihovna -> kod 127` | `dltest-missing`: `ld.so: knihovna ... nenalezena`, code 127 |
| QEMU `procesy`: `vlakna libos s TLS knihoven, dl_iterate_phdr, r_debug, fini` | `dltest-rs` (Rust + libos as PIE, test only): 4 threads have their own fresh copy of the library TLS (through the DTV), the main thread unchanged; the loader table; `dl_iterate_phdr` (3 modules, names, TLS); `r_debug` via `DT_DEBUG` (version 1, 3 nodes); the library finalizer; `dladdr` of a library function; `dl*`: a bad name and a path outside `/system/lib` = an error, a loaded library = a handle and `dlsym`, `RTLD_NOLOAD`; `dlopen` of libraries with TLS (TLSDESC and `__tls_get_addr`, an aligned block, initial-exec in the reserve, an initializer, `dlsym` of a TLS variable) in the main thread, in a thread created before `dlopen` and after it; 4 KiB initial-exec = an error advising `static_tls_reserve` |
| `cargo test -p dynload` (`tls_surplus_after_modules`) | the TLS surplus after the modules does not change their offsets |
| QEMU `procesy`: `obnova po padu libd pres listky (spawn i dlopen)` | a launcher running before the libd crash starts more programs after the restart (`RECONNECT`), init starts from the new root, `dltest-rs` (`dlopen`) passes |
| QEMU `procesy`: `pad libd pred prevzetim listku (ADOPT)` | the launcher's child loads its libraries, libd crashes before the supervisor processes `ADOPT`; the child's `dlopen` waits for the lease and passes after the restart |
| QEMU `procesy`: `rezie spawnu a propustnost initu` | 200 spawns from one launcher and 4 × 50 concurrently without an error, numbers in the log (table above) |
| `cargo test -p svcclient` (`adopt_*`) | `ADOPT` = op 4 without data; the limiter: burst, refill per second, live leases |
| QEMU `procesy`: `rezerva statickeho TLS z voleb spusteni (manifest)` | `dltest-dl`: `dlopen` of a library with 4 KiB initial-exec fails with the default reserve with advice in `dlerror`, succeeds with `static_tls_reserve` 8 KiB and the TLS is right |
| `cargo test -p dynload` (`surplus_*`, `tls_reserve_bounds`) | space in the reserve below the modules (x86_64) and above them (aarch64), alignment, exhaustion; the reserve bound |
| `cargo test -p manifest` (`static_tls_reserve`) | `[app] static_tls_reserve`: a size, at most 64K |
| `cargo test -p libdproto` | library names, messages and limits; decoder fuzzing (20,000 damaged messages without a panic) |
| QEMU `pamet`: `pecet: ...` | a sealed mapping and reservation: `VM_PROTECT`, `VM_UNMAP`, decommit, mapping over it and filling the reservation fail, neighbouring regions work |
| QEMU `procesy`: `libd sdili sablony knihoven mezi procesy` | the second `dltest` run creates no new template, only libd cache hits |
| QEMU `procesy`: `vlakna libos ...` (bit 256) | library code and RELRO are sealed (`ATTR_SEALED`, changing rights and unmapping fail) |
