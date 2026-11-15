# C++ runtime

C++ programs and libraries (HarfBuzz, later libass, Wine, Ladybird, LLVM)
run on **libc++**, **libc++abi** and **libunwind** from the LLVM project
in the same version as the host clang (23.1.5). Below them is the C library
[cstd](cstd.md) and Rust std (one runtime per process). The build is a port
in xtask ([C ports](c-ports.md), `xtask/src/cports/cxx.rs`).

## Status

| Part | What | Status |
|---|---|---|
| static runtime | `libc++.a`, `libc++abi.a`, `libunwind.a` in the port prefix, `clang++.cfg` | **[verified in QEMU]** x86_64 and aarch64 (suite `cpp`, test `cpptest`) |
| shared runtime | one `/system/lib/libc++.so` (libc++ + abi + unwind), a PIE program via the loader | **[verified in QEMU]** x86_64 and aarch64 (test `cpptest-dyn`) |
| exceptions | Itanium ABI, DWARF `.eh_frame` tables, also through C frames (`-funwind-tables`) | **[verified in QEMU]** x86_64 and aarch64 (`cpptest`: through a C frame, `exception_ptr`, in threads) |
| RTTI | `dynamic_cast`, `typeid`, `bad_cast` | **[verified in QEMU]** x86_64 and aarch64 |
| threads | `std::thread`, `mutex`, `condition_variable` over pthread from cstd; `thread_local` with destructors (`__cxa_thread_atexit_impl`) | **[verified in QEMU]** x86_64 and aarch64 |
| process exit | `__cxa_atexit` and `atexit` in one reverse order, `__cxa_finalize`, `__dso_handle` in every module | **[verified in QEMU]** x86_64 and aarch64 (destructor of a global object after `main`) |
| `std::filesystem` | over fsd: `*at` relative to a directory, `realpath`, `statvfs`, `sendfile` | **[verified in QEMU]** x86_64 and aarch64 |
| `chrono`, `random_device` | `clock_gettime`, `getentropy` (kernel generator) | **[verified in QEMU]** x86_64 and aarch64 |
| HarfBuzz 14.2.0 | amalgamation without exceptions and RTTI, called from C | **[verified in QEMU]** x86_64 and aarch64 (test `hbtest`) |
| `<iostream>`, `<locale>`, wide characters | libc++ without locale and without `wchar_t` | **[designed]** stage 7a-2 after the wchar and locale functions in cstd |
| selected `libcxx/test` tests | batches of libc++ tests in QEMU | **[designed]** |

## Layers

<figure class="diagram">
<svg viewBox="0 0 780 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="C++ runtime layers">
<defs><marker id="cp1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="10" width="760" height="40" rx="6"/><text class="t b" x="390" y="35" text-anchor="middle">C++ program (STL, exceptions, threads, filesystem)</text>
<rect class="acc" x="10" y="70" width="370" height="50" rx="6"/><text class="t b" x="195" y="92" text-anchor="middle">libc++</text><text class="m" x="195" y="110" text-anchor="middle">containers, threads, chrono, filesystem</text>
<rect class="acc" x="400" y="70" width="370" height="50" rx="6"/><text class="t b" x="585" y="92" text-anchor="middle">libc++abi</text><text class="m" x="585" y="110" text-anchor="middle">new/delete, RTTI, __cxa_throw, guard</text>
<rect class="acc" x="400" y="140" width="370" height="40" rx="6"/><text class="t b" x="585" y="165" text-anchor="middle">libunwind (.eh_frame via dl_iterate_phdr)</text>
<rect class="ok" x="10" y="200" width="760" height="40" rx="6"/><text class="t b" x="390" y="225" text-anchor="middle">cstd (libc.a / libc.so) + osrt + Rust std</text>
<path class="ln" d="M195 50 L195 66" marker-end="url(#cp1)"/>
<path class="ln" d="M585 50 L585 66" marker-end="url(#cp1)"/>
<path class="ln" d="M585 120 L585 136" marker-end="url(#cp1)"/>
<path class="ln" d="M195 120 L195 196" marker-end="url(#cp1)"/>
<path class="ln" d="M585 180 L585 196" marker-end="url(#cp1)"/>
</svg>
<figcaption>libc++ stands on libc++abi, which stands on libunwind; everything below is the C library cstd over Rust std, so the process has one allocator, one set of threads and one TLS.</figcaption>
</figure>

## Build

The `llvm-runtimes` port (`xtask/src/cports/cxx.rs`) is built before the
other ports, because C++ ports (HarfBuzz) link libc++:

- **Source:** `llvm-project-23.1.5.src.tar.xz` (HTTPS, SHA-256); only the
  needed subtrees are extracted (`fetch::prepare_subset`, about 50 MB of
  179 MB; no tests and docs). Patch
  `third_party/c-ports/llvm-runtimes/0001-magnumopus.patch`:
  `random_device` via `getentropy`, `chrono` via `clock_gettime`.
- **Two CMake configurations** over `runtimes/`:
  - static (`toolchain.cmake`, no PIC) installs `libc++.a`,
    `libc++abi.a`, `libunwind.a` and the `include/c++/v1` headers into
    `target/c-ports/<arch>/prefix`,
  - dynamic (`toolchain-dyn.cmake`, PIC over `libc.so`) builds one
    `libc++.so` with libc++abi and libunwind inside (like Android) into
    `prefix-dyn`; in the initfs it is `/system/lib/libc++.so`.
- **No locale and wide characters** (`LIBCXX_ENABLE_LOCALIZATION=OFF`,
  `LIBCXX_ENABLE_WIDE_CHARACTERS=OFF`): `<iostream>`, `<fstream>`,
  `<locale>`, `<regex>` are not there yet. Stage 7a-2 turns them on once
  cstd has the wchar and locale functions.
- **Threads** over pthread from cstd (`LIBCXX_HAS_PTHREAD_API`), without
  libgcc and compiler-rt: the compiler helper functions are in `libc.a`
  (static) and `libbuiltins.a` (dynamic).

The first build of the runtime takes a few minutes (both configurations,
about 1900 Ninja steps per architecture); later builds are incremental.

### A C++ program against the sysroot

The port prefix has files for `clang++ --config=...`:

| File | Program |
|---|---|
| `target/c-ports/<arch>/prefix/clang++.cfg` | static ET_EXEC with `libc++.a`, `libc++abi.a`, `libunwind.a`, `libc.a` |
| `target/c-ports/<arch>/prefix/clang++-dyn.cfg` | PIE over `libc++.so` and `libc.so` (loader `ld.so`) |

The libc++ headers come before the cstd headers (`-nostdinc++ -isystem
.../c++/v1`): the wrapper headers (`<cstdio>`, `<cstdlib>`) include the
cstd ones via `#include_next`. The `Sources` method of ports compiles
`.cc` and `.cpp` sources with clang++ and links such a program with libc++
([C ports](c-ports.md)).

## Exceptions

The Itanium standard with DWARF tables (`.eh_frame`, `.eh_frame_hdr`), as
on Linux:

- **Finding the tables:** libunwind calls `dl_iterate_phdr`. Under the
  loader `ld.so` answers (each module with its `PT_GNU_EH_FRAME`). A static
  program has no ELF headers in memory: `dl_iterate_phdr` in `libc.a`
  describes a single module (`PT_LOAD` over the whole image,
  `PT_GNU_EH_FRAME` over `.eh_frame_hdr`) from the linker script symbols of
  `user/user-std.ld` (`__mos_image_start/end`,
  `__eh_frame_hdr_start/end`). lld creates the `.eh_frame_hdr` table with
  `--eh-frame-hdr` (the C sysroot passes it).
- **C frames:** the C sysroot compiles with `-funwind-tables`, so an
  exception also passes through a callback from C code (test
  `vyjimka-pres-ramec-c`).
- **Rust frames:** the target has `panic=abort` and Rust has no unwind
  tables. An exception that would have to pass through a Rust function
  (for example a callback from `qsort` in cstd) ends in `std::terminate`.
  Rust libraries that call back into C++ must catch exceptions on their
  side.
- **CPU exceptions** (SIGSEGV, division by zero) are not turned into C++
  exceptions (no `-fnon-call-exceptions`). An exception thrown out of a
  signal handler = `terminate`.

## Process exit and `thread_local`

| Function | Where | Behaviour |
|---|---|---|
| `__cxa_atexit(f, arg, dso)` | cstd (`cxxsup.rs`) | destructors of static objects; one list with `atexit`, run at exit in reverse registration order |
| `__cxa_finalize(dso)` | cstd | runs and removes the functions of a module (`NULL` = all) |
| `__dso_handle` | `libc.a` (static program), `crt1.o` (PIE), `crtdso.o` (every `.so`) | hidden module symbol; `__cxa_finalize` uses it to tell what belongs to a library |
| `__cxa_thread_atexit_impl` | cstd | `thread_local` destructors; at the end of a `pthread_create` thread before the key destructors, at `exit` for the calling thread before the `atexit` list |

The `atexit`/`__cxa_atexit` list runs from one function in
`osrt::at_exit`, registered after the stdio flush, so the streams are
flushed only after all destructors.

**`dlclose`:** the loader does not remove modules (`dlclose` only
decrements the counter, [dynamic linking](dynamic-linking.md), like musl).
The library code therefore stays mapped and its destructors of static
objects and `thread_local` run safely at process exit, not at `dlclose`.
The loader does not call `__cxa_finalize`: destroying the objects of a
library that stays loaded and is not initialized again on the next
`dlopen` would be worse than waiting until exit.

## What cstd added

| Function | For |
|---|---|
| `getentropy`, `sys/random.h` | `std::random_device`; waits until the kernel generator is seeded |
| `aligned_alloc`, `posix_memalign` (cstd-core) | aligned `operator new`; freed by plain `free` (the alignment is in the block header, [cstd](cstd.md)) |
| `timespec_get`, `lldiv` | `<ctime>`, `<cstdlib>` |
| `dl_iterate_phdr` (static), `link.h`, `elf.h` | libunwind |
| `openat`, `unlinkat`, `mkdirat`, `fstatat`, `fchmodat` relative to a directory, `realpath`, `pathconf`, `statvfs`, `sendfile`, `PATH_MAX` | `std::filesystem` |

⚠️ The FS works with paths: `openat(dir, "x")` joins the directory path
remembered when it was opened with the name. Unlike Linux, the `*at`
functions therefore do not protect against the directory being renamed
between opening and the call (`remove_all` in libc++ builds its protection
against symlink swapping on it; the FS does not create symlinks yet).

## HarfBuzz

The `harfbuzz` port (`Sources` method): the `src/harfbuzz.cc` amalgamation
compiled with clang++ and `-fno-exceptions -fno-rtti`, without FreeType,
ICU and glib (own OpenType parser, Unicode tables inside). The `hbtest`
test (in C) loads the Inter font from the initfs via fsd, shapes Czech text
and checks the glyphs (no `.notdef`), advances and the kerning of the pair
"AV". HarfBuzz is the first consumer of the C++ runtime (libass subtitles
and text rendering in FFmpeg, step 7).

## Tests

| Test | Where | What it checks |
|---|---|---|
| `cpptest: libc++ staticky` | QEMU, suite `cpp` | `user/cpptest/cpptest.cpp`: static constructors, containers, exceptions (also through a C frame), RTTI, aligned `new`, `bad_alloc`, threads and `thread_local` with destructors, `chrono`, `random_device`, `std::filesystem` over fsd; destructor of a global object after `main` |
| `cpptest-dyn: libc++.so` | QEMU, suite `cpp` | the same as a PIE over `/system/lib/libc++.so` |
| `hbtest: HarfBuzz` | QEMU, suite `cpp` | shaping text with diacritics, kerning |
| `cxxsup_tests`, `fsat_tests` | host, `cargo test -p cstd` | order of `__cxa_atexit` and `__cxa_finalize` by module, `thread_local` destructors, `getentropy`, `timespec_get`, `statvfs`, `*at` paths, `sendfile`, constants matching the headers |

Run: `cargo xtask test --only cpp` (`--arch aarch64` for the second
architecture).
