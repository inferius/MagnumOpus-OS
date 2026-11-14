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
| DL-2 | the `user/ld` + `libs/dynload` loader: `PT_INTERP` in `spawn`, ASLR, `DT_NEEDED`, `BIND_NOW`, relocations, static TLS; PIC in `cbuild`, `/system/lib` in the image | **[designed]** |
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
