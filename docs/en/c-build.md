# Building C code

The system is in Rust, but vendored C code is compiled and linked into
userspace programs: the first one is ACPICA ([ACPI](acpi.md)), next will be
DRM drivers from Linux (through a compatibility layer) and our own libc.
This page describes how C plugs into the build, what it stands on (the
`cstd` library) and how its safety is guarded.

> **Status:** done: compilation (clang, `libs/cbuild`), the libc base
> (`libs/cstd`: strings, memory, ctype, `printf` to the log, stack
> protection) and the first user, ACPICA. Files (`fopen`), threads and time
> are not in the libc yet.

## How C plugs into the build

<figure class="diagram">
<svg viewBox="0 0 780 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Building C code">
<defs><marker id="pe1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="200" height="80" rx="6"/><text class="t b" x="110" y="45" text-anchor="middle">C sources</text><text class="m" x="110" y="64" text-anchor="middle">third_party/acpica</text><text class="m" x="110" y="80" text-anchor="middle">libs/acpica/c/glue.c</text>
<path class="ln" d="M210 60 L268 60" marker-end="url(#pe1)"/>
<rect class="acc" x="270" y="20" width="220" height="80" rx="6"/><text class="t b" x="380" y="45" text-anchor="middle">build.rs + cbuild</text><text class="m" x="380" y="64" text-anchor="middle">clang --target=..-none-elf</text><text class="m" x="380" y="80" text-anchor="middle">common flags and protections</text>
<path class="ln" d="M490 60 L548 60" marker-end="url(#pe1)"/>
<rect class="box" x="550" y="20" width="220" height="80" rx="6"/><text class="t b" x="660" y="45" text-anchor="middle">libacpica.a</text><text class="m" x="660" y="64" text-anchor="middle">llvm-ar, in OUT_DIR</text><text class="m" x="660" y="80" text-anchor="middle">cargo:rustc-link-lib</text>
<path class="ln" d="M660 100 L660 150" marker-end="url(#pe1)"/>
<rect class="ok" x="430" y="152" width="340" height="80" rx="6"/><text class="t b" x="600" y="177" text-anchor="middle">program (rust-lld)</text><text class="m" x="600" y="196" text-anchor="middle">Rust code + C library + cstd</text><text class="m" x="600" y="212" text-anchor="middle">+ compiler_builtins (memcpy, memset)</text>
<rect class="box" x="10" y="152" width="380" height="80" rx="6"/><text class="t b" x="200" y="177" text-anchor="middle">libs/cstd (Rust, C ABI)</text><text class="m" x="200" y="196" text-anchor="middle">include/*.h: string, stdlib, ctype, stdio, assert</text><text class="m" x="200" y="212" text-anchor="middle">strlen, malloc, printf, __stack_chk_*</text>
<path class="ln" d="M390 192 L428 192" marker-end="url(#pe1)"/>
</svg>
<figcaption>C is compiled in the build.rs of the crate that contains it; the resulting static library is linked with the program together with cstd.</figcaption>
</figure>

1. A crate with C code (e.g. `libs/acpica`) has a `build.rs` that calls
   `cbuild::Build` with the list of files and headers.
2. `cbuild` finds clang (`CLANG`, `PATH`, `C:\Program Files\LLVM\bin`,
   `/usr/bin`), compiles the files in parallel with flags for the Rust
   target (`TARGET`) and archives them with `llvm-ar` into `lib<name>.a`
   in `OUT_DIR`.
3. Cargo links the library (`cargo:rustc-link-lib=static=...`) into the
   program using the crate. rust-lld links it together with the Rust code.
4. Functions the C code needs from the system are provided by Rust with the
   C ABI (`#[unsafe(no_mangle)] extern "C"`): the C library is
   `libs/cstd`, OS services come from the particular program (ACPICA:
   `user/acpid/src/osl`).

Only what changed is recompiled (file time against the `.o`); changing the
flags, the compiler or any header = everything again. ACPICA (164 files)
compiles in a few seconds on an ordinary PC.

### Tools

| Tool | From | Note |
|---|---|---|
| clang 17+ | LLVM (Windows: `winget install LLVM.LLVM`, Linux: the `clang` package) | one clang handles both targets (`x86_64`, `aarch64`) |
| llvm-ar | the same LLVM installation | looked up next to clang |

A missing clang = a build error with instructions on how to install it.

### Adding C code (example)

```rust
// build.rs of crate libs/foo
fn main() {
    cbuild::Build::new("foo")
        .trust(cbuild::Trust::Own)          // own code: warnings = errors
        .include("include")
        .include("../cstd/include")         // the system libc
        .file("c/foo.c")
        .compile();
}
```

```toml
[dependencies]
cstd = { path = "../cstd" }                # the C library gets linked in

[build-dependencies]
cbuild = { path = "../cbuild" }
```

Vendored code: `Trust::Vendored` (warnings are not shown, the code is not
changed), and vendored headers included from own code via
`include_system` (`-isystem`) so their warnings do not count as errors.

## Compiler flags

| Flag | Why |
|---|---|
| `--target=x86_64-unknown-none-elf` / `aarch64-unknown-none-elf` | OS-less ELF in LLVM, the ABI of the Rust targets `*-unknown-magnumopus` |
| `-march=x86-64-v2` (x86) / `-march=armv8-a+simd` (aarch64) | hard-float like Rust: `float`/`double` in FPU registers, the CPU baseline ([hardware-requirements.md](hardware-requirements.md)) |
| `-mcmodel=small` (x86) | the same model as the Rust target |
| `-mstrict-align` (aarch64) | as the Rust target; an unaligned device register access = exception |
| `-ffreestanding -nostdlibinc` | no host headers (Windows SDK, glibc), only clang's and cstd |
| `-fno-pic -fno-common -ffunction-sections -fdata-sections` | static programs, the linker drops unused functions |
| `-O2` | an AML interpreter and similar code is many times slower unoptimised |
| `-std=c11` | the language of ACPICA and Linux |

## How safety is guarded

C code has none of Rust's memory checks. Therefore:

| Measure | What it protects |
|---|---|
| `-fstack-protector-strong`, `-mstack-protector-guard=global` | a canary in every function with an array on the stack; overflowing the array = `__stack_chk_fail` (cstd) ends the process before it jumps to a forged return address. The canary is random (`cstd::init_stack_guard`), its lowest byte zero (string functions cannot overwrite it "correctly") |
| `-ftrivial-auto-var-init=zero` | uninitialised local variables are zero (no leak of old data, predictable behaviour) |
| `-fno-strict-aliasing -fwrapv -fno-delete-null-pointer-checks` | the compiler does not drop pointer and overflow checks based on "undefined behaviour" |
| `-Werror=implicit-function-declaration`, `incompatible-pointer-types`, `int-conversion`, `return-type` | calling a function without a prototype (truncated pointer) and similar bugs do not compile, even in vendored code |
| own code: `-Wall -Wextra -Werror` | warnings = build errors |
| cstd without dangerous functions | no `sprintf`, `gets`, `strtok`; `%n` in printf writes nothing; huge format widths are capped |
| `_Static_assert` on structure layouts | a new version of vendored code cannot silently break the Rust interface |
| a process, not the kernel | C runs in userspace with a restricted resource; a crash = a service crash, the kernel and other processes' RAM stay protected |

To consider next: `-fsanitize=bounds -fsanitize-trap` (array bounds without
a runtime library; ACPICA has variable-length arrays declared as `[1]`,
needs checking), CFI (`-fsanitize=cfi` needs LTO).

## The `cstd` library

The base of our own libc, written in Rust (tested on the host:
`cargo test -p cstd`). The headers are in `libs/cstd/include`; the types
`stddef.h`, `stdint.h`, `stdarg.h`, `stdbool.h` and `limits.h` come from
clang.

| Header | Functions |
|---|---|
| `string.h` | `strlen`, `strnlen`, `strcmp`, `strncmp`, `strcpy`, `strncpy`, `strcat`, `strncat`, `strchr`, `strrchr`, `strstr`, `memchr`; `memcpy`, `memmove`, `memset`, `memcmp` come from `compiler_builtins` |
| `ctype.h` | `isdigit`, `isxdigit`, `isalpha`, `isalnum`, `isupper`, `islower`, `isspace`, `isprint`, `isgraph`, `iscntrl`, `ispunct`, `toupper`, `tolower` (ASCII) |
| `stdlib.h` | `malloc`, `calloc` (overflow check), `realloc`, `free`, `strtoul`, `strtoull`, `strtol`, `strtoll`, `atoi`, `abs`, `abort` |
| `stdio.h` | `printf`, `vprintf`, `snprintf`, `vsnprintf`, `puts`, `putchar`; output = the program log, line by line |
| `assert.h` | `assert` (failure ends the process with the file name and line) |

At the start of `main` the program sets where the output goes and a random
canary – **before** the first call into C code:

```rust
cstd::set_output(|line| libos::println!("acpid: {}", core::str::from_utf8(line).unwrap_or("?")));
cstd::init_stack_guard(libos::random::u64().0);
```

`printf` supports the flags `-+ #0`, width and precision (also `*`), the
lengths `hh h l ll z j t` and the conversions `d i u o x X c s p %`.
Floating point is not in `cstd` yet (`?` is printed); programs already have an FPU.

### Plan: our own libc and porting more C

- **libc over libos** (the languages milestone): files (`FILE`, `fopen`
  over fsd), `errno`, time, threads (`pthread` over `libos::thread`),
  `qsort`, `bsearch`, floating point. Built on the same headers and the
  same approach (Rust with the C ABI, headers in `include`).
- **DRM drivers from Linux**: the compatibility layer (`linux/*.h` onto
  system primitives) will be another crate with a `build.rs` via `cbuild`;
  the driver will run as a userspace process like other drivers.

## Licences of vendored code

Vendored code has its own directory in `third_party/` with a README
(version, origin, what was left out, edits) and its licence; the overview
is in `THIRD-PARTY.md` at the repository root.
