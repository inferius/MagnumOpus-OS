# C ports

Third-party libraries and programs in C (zlib, sqlite, SDL, later FFmpeg)
are built by one mechanism in xtask (`xtask/src/cports/`) against the
[C sysroot](cstd.md#c-sysroot-and-ports-part-2i). Every port is a pinned
version from its official site, verified by a hash, with our patches in
the repository.

## Status

| Part | What | Status |
|---|---|---|
| port source | HTTPS (`curl`), SHA-256, `tar` (zip too), patches with `git apply`, own files (overlay) | **[verified in QEMU]** zlib and sqlite (suite `cstd porty`, no patches); patches and overlay **[implemented]**, verified with SDL |
| `Sources` | a list of sources compiled directly with clang | **[verified in QEMU]** zlib 1.3.1, sqlite 3.46.1 |
| `CMake` | CMake + Ninja, toolchain file and the `MagnumOpus` platform module from the sysroot | **[implemented]** (SDL, step 6) |
| `Configure` | a `configure` script (autotools, FFmpeg) + `make` | **[implemented]**, verified with FFmpeg |
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
| `build` | the build method (below) |

Own files of a port (for example the SDL driver for our compositor) are in
`third_party/c-ports/<port>/overlay/` and are copied over the source after
the patches. The prepared source (`target/c-ports/src/<port>-<hash>`) is
made again only when the archive, a patch or an overlay file changes.

## Build methods

| Method | When | Needs |
|---|---|---|
| `Sources` | small libraries with a simple source list (zlib, sqlite) | clang only |
| `CMake` | projects with CMake (SDL) | CMake, Ninja |
| `Configure` | projects with a `configure` script (autotools, FFmpeg) | `sh`, `make` |

- **`CMake`:** xtask generates `target/c-ports/<arch>/cmake/toolchain.cmake`
  and the platform module `Platform/MagnumOpus.cmake` (`UNIX`, `MAGNUMOPUS`,
  static libraries only). The flags are the same as in the sysroot
  `clang.cfg`; `-lc` is in `CMAKE_C_STANDARD_LIBRARIES` (after the objects).
  Libraries and headers are searched only in the sysroot and the port
  prefix; the host `pkg-config` is switched off (it must not find Windows
  or Linux libraries). Configuration is repeated only when the options, the
  toolchain file or the source change.
- **`Configure`:** the arguments have placeholders `{prefix}`, `{host}`,
  `{arch}`, `{cc}`, `{ar}`, `{cflags}`, `{ldflags}`, because every script
  wants the target differently (autotools `--host=`, FFmpeg `--target-os=
  --arch= --cc=`). The build runs outside the source tree.
- **Output:** libraries and headers into the prefix
  `target/c-ports/<arch>/prefix` (the next port finds them), programs into
  the initfs.

## Host tools

When a tool is missing, the build ends with one error listing all missing
tools and advice for the host:

| Tool | Windows | Linux / macOS |
|---|---|---|
| CMake | `winget install Kitware.CMake` | `apt install cmake` / `brew install cmake` |
| Ninja | `winget install Ninja-build.Ninja` | `apt install ninja-build` / `brew install ninja` |
| `sh` | Git for Windows | part of the system |
| `make` | MSYS2 (`pacman -S make`) or WSL | `apt install make` |

The path can also be given in a variable (`CMAKE`, `NINJA`, `SH`, `MAKE`).
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

Port tests and limitations (sqlite without WAL and mmap I/O) are described
in [cstd](cstd.md#c-sysroot-and-ports-part-2i).
