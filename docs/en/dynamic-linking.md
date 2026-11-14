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
| DL-3 | `dlopen`/`dlsym`/`dlclose`, dynamic TLS (`__tls_get_addr`, TLSDESC), `dl_iterate_phdr`, `r_debug` | **[designed]** |
| DL-4 | the `libd` template service (sharing library code), the RELRO seal | **[designed]** |

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
<figcaption>Starting a PIE program with shared libraries (DL-2 to DL-4 designed).</figcaption>
</figure>

- **Loader:** a separate `ld.so` (also `libc.so` in musl style), the
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
`program_main` -> exit syscall; `cstd` as `libc.so` does not exist yet).

⚠️ **Limitations of DL-2 (deliberate, solved by DL-3 and DL-4):**

- **Library code is not shared between processes**: each process copies the
  library into its own memory (sharing through the `libd` template service
  in DL-4).
- **Libraries only from initfs**, and only a process that has initfs
  (init today) may start a program with the loader; disk, application
  `$ORIGIN` and `libd` in DL-4.
- **TLS of the main thread only**: new threads (libos `thread::spawn`) do
  not get the libraries' TLS block, and a runtime with its own TLS (`osrt`
  for Rust std) would overwrite the thread pointer - an agreement between
  the loader and libos in DL-3.
- **`DT_FINI` and `DT_FINI_ARRAY` are not called** (the process ends with
  a syscall).
- **RELRO can be switched back to RW** (`VM_PROTECT`); the `max_flags` seal
  in DL-4.
- The loader has all of initfs (read-only) while loading; the program does
  not get it (the loader unmaps it and closes the handles before the jump).

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
