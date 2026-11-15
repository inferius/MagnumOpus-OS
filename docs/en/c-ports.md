# C ports

Third-party libraries and programs in C (zlib, sqlite, SDL, later FFmpeg)
are built by one mechanism in xtask (`xtask/src/cports/`) against the
[C sysroot](cstd.md#c-sysroot-and-ports-part-2i). Every port is a pinned
version from its official site, verified by a hash, with our patches in
the repository.

## Status

| Part | What | Status |
|---|---|---|
| tool versions | CMake 3.24+, Ninja 1.10+ (`--version`), advice on an old version | **[implemented]** host test; [Building](building.md#tools) |
| port source | HTTPS (`curl`), SHA-256, `tar` (zip too), patches with `git apply`, own files (overlay) | **[verified in QEMU]** zlib and sqlite (suite `cstd porty`), patches and overlay with SDL (suite `sdl`) |
| `Sources` | a list of sources compiled directly with clang | **[verified in QEMU]** zlib 1.3.1, sqlite 3.46.1 |
| `CMake` | CMake + Ninja, toolchain file and the `MagnumOpus` platform module from the sysroot | **[verified in QEMU]** SDL 3.4.20 ([SDL](sdl.md), suite `sdl`) |
| `Configure` | a `configure` script (autotools, FFmpeg) + `make`, also with shared libraries over `libc.so` | **[verified in QEMU]** FFmpeg 9.0.2 ([Multimedia](multimedia.md), suite `media ffmpeg`) |
| `Meson` | Meson + Ninja, cross file from the sysroot, static library with `-fPIC` | **[verified in QEMU]** dav1d 1.5.4 (inside `libavcodec.so`, suite `media ffmpeg`) |
| missing tool | an error with the tool name and install advice for the host | **[implemented]** host test `cargo test -p xtask cports` |

## A port

<figure class="diagram">
<svg viewBox="0 0 780 200" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Building a C port">
<defs><marker id="pc1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="10" width="230" height="70" rx="6"/><text class="t b" x="125" y="34" text-anchor="middle">archive</text><text class="m" x="125" y="52" text-anchor="middle">HTTPS, SHA-256</text><text class="m" x="125" y="68" text-anchor="middle">target/c-ports/dl</text>
<rect class="acc" x="275" y="10" width="230" height="70" rx="6"/><text class="t b" x="390" y="34" text-anchor="middle">port source</text><text class="m" x="390" y="52" text-anchor="middle">tar + patches + overlay</text><text class="m" x="390" y="68" text-anchor="middle">third_party/c-ports/&lt;port&gt;</text>
<rect class="acc" x="540" y="10" width="230" height="70" rx="6"/><text class="t b" x="655" y="34" text-anchor="middle">build</text><text class="m" x="655" y="52" text-anchor="middle">Sources / CMake / Configure</text><text class="m" x="655" y="68" text-anchor="middle">against the C sysroot</text>
<path class="ln" d="M240 45 L271 45" marker-end="url(#pc1)"/>
<path class="ln" d="M505 45 L536 45" marker-end="url(#pc1)"/>
<rect class="ok" x="275" y="120" width="230" height="64" rx="6"/><text class="t b" x="390" y="146" text-anchor="middle">port prefix</text><text class="m" x="390" y="164" text-anchor="middle">include/, lib/ (further ports)</text>
<rect class="ok" x="540" y="120" width="230" height="64" rx="6"/><text class="t b" x="655" y="146" text-anchor="middle">initfs</text><text class="m" x="655" y="164" text-anchor="middle">programs (static ET_EXEC)</text>
<path class="ln" d="M600 80 L440 116" marker-end="url(#pc1)"/>
<path class="ln" d="M655 80 L655 116" marker-end="url(#pc1)"/>
</svg>
<figcaption>The archive is verified and the source is prepared with patches; the port is built in one of three ways. Libraries go to the prefix, programs to the initfs.</figcaption>
</figure>

A port is an entry in the `PORTS` table (`xtask/src/cports/mod.rs`):

| Field | What |
|---|---|
| `url`, `file`, `sha256` | the pinned archive; HTTPS only, the hash is checked on every use |
| `top` | the directory inside the archive |
| `patches` | patches in `third_party/c-ports/<port>/` (order of application, `-p1`) |
| `data` | more files for the initfs; a `port:` path = in the port folder, `ws:` = in the repository, otherwise in the port source |
| `build` | the build method (below) |

Own files of a port (for example the SDL driver for our compositor) are in
`third_party/c-ports/<port>/overlay/` and are copied over the source after
the patches. The prepared source (`target/c-ports/src/<port>-<hash>`) is
made again only when the archive, a patch or an overlay file changes. Only
a part of a large archive can be extracted (`fetch::prepare_subset`:
selected subtrees and `--exclude` patterns); the [C++ runtime](cpp.md)
does that with `llvm-project`.

## Build methods

| Method | When | Needs |
|---|---|---|
| `Sources` | small libraries with a simple source list (zlib, sqlite, HarfBuzz) | clang only (C++ via clang++ and libc++) |
| `CMake` | projects with CMake (SDL) | CMake, Ninja |
| `Configure` | projects with a `configure` script (autotools, FFmpeg) | `sh`, `make`, pkgconf (nasm for FFmpeg on x86_64) |
| `Meson` | Meson projects (dav1d) | `sh`, Meson, Ninja, pkgconf (nasm on x86_64) |

- **`Sources`:** `.cc`, `.cpp` and `.cxx` sources are compiled by clang++
  against the libc++ headers ([C++ runtime](cpp.md)); a program with such
  an object is linked with `libc++.a`, `libc++abi.a` and `libunwind.a`.
- **`CMake`:** xtask generates `target/c-ports/<arch>/cmake/toolchain.cmake`
  and the platform module `Platform/MagnumOpus.cmake` (`UNIX`, `MAGNUMOPUS`,
  static libraries only). The flags are the same as in the sysroot
  `clang.cfg` (C, C++ and `.S` assembler); `-lc` is in
  `CMAKE_C_STANDARD_LIBRARIES` (after the objects). A second file
  `toolchain-dyn.cmake` (`MAGNUMOPUS_DYNAMIC`) builds PIC over `libc.so`
  including shared libraries without a version in the SONAME (the loader
  looks for them in `/system/lib` as `libfoo.so`); every `.so` gets
  `crtdso.o` with its own `__dso_handle`. CMake try-compiles only into a
  static library, so library existence checks (`check_library_exists`)
  always succeed: a port must switch off the libraries that do not exist
  explicitly.
  Libraries and headers are searched only in the sysroot and the port
  prefix; the host `pkg-config` is switched off (it must not find Windows
  or Linux libraries). Configuration is repeated only when the options, the
  toolchain file or the source change.
- **`Configure`:** the arguments have placeholders `{prefix}`, `{host}`,
  `{arch}`, `{cc}`, `{ar}`, `{nm}`, `{ranlib}`, `{strip}`, `{cflags}`,
  `{ldflags}`, because every script wants the target differently (autotools
  `--host=`, FFmpeg `--target-os= --arch= --cc=`). LLVM tools are passed by
  name only (the clang directory is in `PATH`; FFmpeg's configure cannot
  handle paths with a space). A port with **shared libraries**
  (`libraries`, FFmpeg) takes the flags of the dynamic sysroot plus
  `{ldexeflags}` (PIE programs only: `crt1.o`, loader), `{ldsoflags}`
  (`-Wl,-shared`) and `{libs}` (`libc.so`, `libbuiltins.a`); the clang
  baremetal driver links `-Bstatic` by itself and does not pass `-shared`,
  hence `-Wl,-Bdynamic` and `-Wl,-shared` explicitly. The build runs outside
  the source tree.
- **`Meson`:** xtask generates `target/c-ports/<arch>/<port>/cross.txt`
  (`system = 'magnumopus'`, shared-library flags from the sysroot) and builds
  a static library with `-fPIC` (`b_staticpic`) for another port's shared
  library. Meson from MSYS2 is a Python script: it runs through `sh` and gets
  paths as `/j/...`; after install xtask rewrites the prefix in `.pc` files
  to `J:/...` (clang does not know MSYS2 paths).
- **Ports over `libc.so`** (`dynamic`: dav1d, FFmpeg) are built only after
  the dynamic part of the C sysroot (`libc.so`, `crt1.o`); their test
  programs are linked as PIE.
- **Output:** libraries and headers into the prefix
  `target/c-ports/<arch>/prefix` (the next port finds them), programs into
  the initfs, shared libraries into initfs `lib/<soname>`.

## Host tools

When a tool is missing, the build ends with one error listing all missing
tools and advice for the host:

| Tool | Windows | Linux / macOS |
|---|---|---|
| CMake (3.24+) | `winget install Kitware.CMake` | `apt install cmake` / `brew install cmake` |
| Ninja | `winget install Ninja-build.Ninja` | `apt install ninja-build` / `brew install ninja` |
| `sh` | Git for Windows | part of the system |
| `make` | MSYS2 (`pacman -S make`) or WSL | `apt install make` |
| nasm (2.14+, x86_64 only) | MSYS2 (`pacman -S nasm`) | `apt install nasm` / `brew install nasm` |
| Meson | MSYS2 (`pacman -S meson`) | `apt install meson` / `brew install meson` |
| pkgconf | MSYS2 (with Meson) | `apt install pkgconf` / `brew install pkgconf` |

On Windows `sh`, `make`, nasm, Meson and pkgconf are taken from MSYS2 first
(`C:\msys64\usr\bin`, the `MSYS2_ROOT` variable): one environment instead
of a mix of Git and MSYS2, run in `MSYSTEM=MSYS` mode (Meson from MSYS2
refuses to run under MINGW64 from Git Bash). The path can also be given in
a variable (`CMAKE`, `NINJA`, `SH`, `MAKE`, `NASM`, `MESON`, `PKG_CONFIG`).
On Windows the usual install locations outside `PATH` are searched too (the
installer may not have refreshed `PATH` in an open terminal).

> **Pitfall:** `target/` lies inside the repository. `git apply` would find
> our repository there and silently skip a patch outside its index; xtask
> therefore sets `GIT_CEILING_DIRECTORIES` to the port source directory.

## Ports

| Port | Version | Licence | Method | Programs |
|---|---|---|---|---|
| zlib | 1.3.1 | zlib | `Sources` | `zlib-example`, `minigzip` |
| sqlite | 3.46.1 | public domain | `Sources` | `sqlite3`, `sqltest` |
| HarfBuzz | 14.2.0 | Old MIT | `Sources` (C++) | `hbtest` ([C++ runtime](cpp.md)) |
| SDL | 3.4.20 | zlib | `CMake` | `snake`, `woodeneye`, `testsprite`, `testaudio`, `testkeys`, `checkkeys`, `testmagnumopus` ([SDL](sdl.md)) |
| dav1d | 1.5.4 | BSD-2-Clause | `Meson` | - (inside `libavcodec.so`) |
| FFmpeg | 9.0.2 | LGPL-2.1-or-later | `Configure` (shared libraries) | `lib/libav*.so`, `libsw*.so`; the `ffdec` process ([Multimedia](multimedia.md)) |
| LLVM runtimes | 23.1.5 | Apache-2.0 WITH LLVM-exception | `CMake` (own, `cxx.rs`) | `cpptest`, `cpptest-dyn`, `/system/lib/libc++.so` ([C++ runtime](cpp.md)) |

Port tests and limitations (sqlite without WAL and mmap I/O) are described
in [cstd](cstd.md#c-sysroot-and-ports-part-2i).
