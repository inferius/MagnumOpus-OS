# The C library (cstd)

MagnumOpus's own C library, written in Rust with a C ABI. It has two
parts: **`cstd-core`** (the base without system services, for `no_std`
services such as ACPICA in `acpid`) and **`cstd`** (a POSIX layer over
[Rust std](rust-std.md) for applications). A C program therefore runs on
the same runtime as a Rust program: one allocator, one TLS, one stdio.
Compiling C (clang, flags, protections) is described in
[Building C code](c-build.md).

> **Status:** part 1 **[verified in QEMU]** on x86_64 and aarch64 (test
> `ctest: stdio, soubory, errno, getenv, atexit`, suite `cstd`): `errno`,
> `FILE` (stdin/stdout/stderr, files), `exit`/`atexit`, `getenv`. Further
> parts (threads, fd numbers, time, `qsort`, floating point in `printf`)
> **[designed]**.

## Layers

<figure class="diagram">
<svg viewBox="0 0 780 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Layers of the C library">
<defs><marker id="cs1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="ok" x="10" y="10" width="370" height="50" rx="6"/><text class="t b" x="195" y="32" text-anchor="middle">C application (+ Rust)</text><text class="m" x="195" y="50" text-anchor="middle">user/ctest, later zlib, FFmpeg (process)</text>
<rect class="box" x="400" y="10" width="370" height="50" rx="6"/><text class="t b" x="585" y="32" text-anchor="middle">no_std service with C code</text><text class="m" x="585" y="50" text-anchor="middle">acpid (ACPICA)</text>
<path class="ln" d="M195 60 L195 88" marker-end="url(#cs1)"/>
<rect class="acc" x="10" y="90" width="370" height="60" rx="6"/><text class="t b" x="195" y="113" text-anchor="middle">libs/cstd (POSIX over std)</text><text class="m" x="195" y="131" text-anchor="middle">errno, FILE, exit/atexit, getenv</text>
<path class="ln" d="M195 150 L195 178" marker-end="url(#cs1)"/>
<path class="ln" d="M380 120 L560 178" marker-end="url(#cs1)"/>
<path class="ln" d="M585 60 L585 178" marker-end="url(#cs1)"/>
<rect class="box" x="10" y="180" width="370" height="50" rx="6"/><text class="t b" x="195" y="202" text-anchor="middle">std + osrt</text><text class="m" x="195" y="220" text-anchor="middle">files, stdio, at_exit, TLS</text>
<rect class="acc" x="400" y="180" width="370" height="50" rx="6"/><text class="t b" x="585" y="202" text-anchor="middle">libs/cstd-core (no_std)</text><text class="m" x="585" y="220" text-anchor="middle">string, ctype, malloc, snprintf, SSP</text>
<path class="ln" d="M195 230 L195 258" marker-end="url(#cs1)"/>
<rect class="box" x="10" y="260" width="760" height="34" rx="6"/><text class="t b" x="390" y="282" text-anchor="middle">libos + services (fsd, ...)</text>
</svg>
<figcaption>cstd uses std (one runtime with Rust) and shares the cstd-core base with no_std services.</figcaption>
</figure>

- **`libs/cstd-core`** **[implemented]**: strings, `ctype`, `malloc` (the
  program heap), `strtol` and friends, `snprintf`, stack protection. The
  `service-io` feature (default) adds `printf`/`puts`/`abort` writing to
  the service log; applications with std turn it off (`cstd` provides
  stdio).
- **`libs/cstd`** **[verified in QEMU]**: POSIX headers and functions over
  std. It exports the C symbols only on the `*-unknown-magnumopus` target;
  on the host the logic is tested (`cargo test -p cstd`), the symbols would
  clash with the system libc.

## A C program

A program is a `#![no_main]` crate that links `cstd` and compiles the C
sources in `build.rs`. `osrt` calls the C `main(argc, argv)` at start-up
and passes its return value to `exit`.

```rust
// src/main.rs
#![no_main]
extern crate cstd;
```

```rust
// build.rs: cstd passes the headers (links = "cstd") in DEP_CSTD_INCLUDE
fn main() {
    let dirs = std::env::var("DEP_CSTD_INCLUDE").unwrap();
    let mut b = cbuild::Build::new("app").trust(cbuild::Trust::Own).file("c/app.c");
    for d in dirs.split(';') {
        b = b.include(d);
    }
    b.compile();
}
```

The program is built like other std applications (`STD_PROGRAMS` in
`xtask/src/stdsys.rs`). For the `*-unknown-magnumopus` targets `cbuild`
compiles hard-float (SSE2 / NEON like the Rust target) with the
`local-exec` TLS model (`_Thread_local` and `errno` share TLS blocks with
Rust).

The `cstd` headers (`stdio.h`, `stdlib.h`, `string.h`) extend the base
`cstd-core` headers via `#include_next`; `errno.h` exists only in `cstd`.

## Functions (part 1)

| Header | Functions | Note |
|---|---|---|
| `errno.h` | `errno` (`__errno_location`), `E*` constants | per thread (TLS); numbers as in Linux/POSIX (`ENOENT` = 2, ...) |
| `stdio.h` | `stdin`, `stdout`, `stderr`, `fopen`, `fclose`, `fflush` (also `NULL`), `fread`, `fwrite`, `fgetc`, `getc`, `getchar`, `ungetc`, `fgets`, `fputc`, `putc`, `putchar`, `fputs`, `puts`, `printf`, `vprintf`, `fprintf`, `vfprintf`, `fseek`, `ftell`, `rewind`, `feof`, `ferror`, `clearerr`, `setvbuf`, `perror`, `remove`, `rename` | `snprintf` and the formatting engine come from `cstd-core` |
| `stdlib.h` | `exit`, `_Exit`, `atexit`, `getenv`, `abort` | `EXIT_SUCCESS`, `EXIT_FAILURE` |
| `string.h` | `strerror` | the rest is in `cstd-core` |
| `assert.h` | `assert` | a failure prints file and line to stderr and ends the process |

### Streams and buffers

- `stdout` line-buffered, `stderr` unbuffered, files 4 KiB (`BUFSIZ`).
  `setvbuf` changes the mode; a caller-supplied buffer is not used.
- Reading `stdin` flushes `stdout` first (a prompt without a newline).
- Every `FILE` has its own lock; threads may share a stream.
- At process exit (`exit`, return from `main`) all streams are flushed:
  `cstd` registers the flush via `std::os::magnumopus::rt::at_exit` (an
  `osrt` function); `atexit` uses the same list. Functions run in reverse
  registration order; the flush is registered first, so it runs after all
  `atexit` functions (they may still write).
- `_Exit` ends without `atexit` and without flushing (`rt::exit_now`).

### Errors

A `std::io::Error` is mapped to `errno` by its kind (`NotFound` ->
`ENOENT`, `PermissionDenied` -> `EACCES`, ...; an unknown kind is `EIO`).
An invalid `fopen` mode or a name that is not UTF-8 = `EINVAL`.

## Limitations and plan

- **Floating point in `printf`** is missing (`%f`, `%g`, `%e` print `?`);
  it will be added to the formatting engine in `cstd-core` (only for
  hard-float targets).
- **Part 2 [designed]:** `pthread` over `osrt` threads and the futex
  (needs a futex extension in `std::os::magnumopus`), an fd number table in
  `osrt` (`open`, `read`, `write`, `close`, shared with `std`), time
  (`time`, `clock_gettime`), `qsort`, `bsearch`, `strtod`, anonymous `mmap`
  (after on-demand page allocation), sockets.
- **Ports:** zlib first (small, tests in the package), sqlite later.

### Kernel requirement: AVX and XSAVE

The kernel today saves FPU/SIMD state with FXSAVE (x87 + XMM0-15) and does
not enable AVX (the instructions fault with `#UD`). High-performance C code
(FFmpeg, codecs) needs AVX and AVX2, therefore **[designed]**: the kernel
enables `XSAVE`/`XRSTOR` (CR4.OSXSAVE, XCR0 with x87, SSE, AVX; later
AVX-512 according to CPUID), a per-thread state area sized from CPUID (leaf
0xD) instead of a fixed 512 B, and lazy or always-full saving. Until then C
code is compiled with SSE2 only (x86_64) and NEON (aarch64). See
[Kernel](kernel.md).

## FFmpeg and licences

FFmpeg will run as a **separate isolated process** (the playback and
decoding service, see [Architecture](architecture.md)); applications do not
link it, they talk to the service. Licensing:

- It is built only under **LGPL-2.1-or-later** - never `--enable-gpl` or
  `--enable-nonfree` (GPL and non-free parts must not enter the system).
- The LGPL obligation (the ability to relink a modified library) applies
  only to the FFmpeg process binary, not to the rest of the system: we ship
  the sources of the FFmpeg version used, our changes and the build script
  with which the user can rebuild the process with their own version of the
  library.
- Later dynamic linking (a shared FFmpeg library), once there is a shared
  library loader; then replacing the library is enough.
- The `THIRD-PARTY.md` entry is created with the first vendored code (the
  FFmpeg section with the rules is already there).
