# Multimedia: FFmpeg and the player

Design of step 7 of the order in [architecture.md](architecture.md):
decoding audio and video with the third-party FFmpeg library in an
**isolated process**, with a player on top. The whole page is
**[designed]**; no code exists yet. The user's decisions (2026-11-14) are
in [Decisions](#decisions). Parts depending on C ports (F1, building
FFmpeg and the subtitle libraries) wait until the C port mechanism from the
SDL thread is on `main`; the service, the protocol and the player with a
test decoder do not depend on it.

## Status

| Part | What | Status |
|---|---|---|
| F1 | FFmpeg built as a C port (LGPL, shared libraries over `libc.so`), test of an `ffprobe`-like program in QEMU | **[designed]** |
| F2 | `mediad` service + `ffdec` decoder process, `mediaproto` protocol, audio into `audiod`; first with a test decoder without FFmpeg | **[designed]** |
| F3 | video into shared buffers (`bufdesc` NV12/I420/P010) with fences, A/V sync | **[designed]** |
| F4 | Player application over `libs/ui` (video layer, controls, libass subtitles, media session) | **[designed]** |
| F5 | hardware-accelerated decoding (VA-API/Vulkan Video through the GPU driver) | **[designed]** |

## Why an isolated process

Demuxers and decoders process untrusted data from the network and from
files; FFmpeg fixes dozens of memory bugs every year. Therefore:

- the decoder runs in a **separate process with no rights**: no access to
  the FS, the network, launchd or drivers; it gets only a channel to
  `mediad` and shared buffers,
- one decoder process **per playback session** (decision 1); a crash or
  exploit of one session does not affect another one or the application,
- input is **bytes only** (a stream or file blocks), output only decoded
  frames and samples in shared memory,
- job limits (memory, CPU, thread count) come from the service manifest;
  after a crash `mediad` reports an error for the session and the
  application can reopen it from the last position (recovery as in the GUI).

LGPL code thus stays in a separate binary and process; the kernel, `libs/`,
services and applications do not depend on it (rules in `THIRD-PARTY.md`).

## Architecture

<svg viewBox="0 0 760 360" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Media path">
  <style>
    .b{fill:#f4f6fa;stroke:#4a5a78;stroke-width:1.5}
    .i{fill:#fdf1e6;stroke:#b4692a;stroke-width:1.5}
    .t{font:13px sans-serif;fill:#1d2433}
    .s{font:11px sans-serif;fill:#4a5a78}
    .a{stroke:#1d2433;stroke-width:1.3;fill:none;marker-end:url(#m)}
    .d{stroke:#b4692a;stroke-width:1.3;fill:none;stroke-dasharray:5 3;marker-end:url(#m)}
  </style>
  <defs><marker id="m" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path d="M0,0L10,5L0,10z" fill="#1d2433"/></marker></defs>
  <rect class="b" x="20" y="20" width="200" height="70" rx="6"/>
  <text class="t" x="35" y="45">Player (application)</text>
  <text class="s" x="35" y="65">libs/ui, video layer,</text>
  <text class="s" x="35" y="80">controls, subtitles</text>
  <rect class="b" x="290" y="20" width="180" height="70" rx="6"/>
  <text class="t" x="305" y="45">mediad (service)</text>
  <text class="s" x="305" y="65">sessions, reads input,</text>
  <text class="s" x="305" y="80">starts decoders</text>
  <rect class="i" x="290" y="150" width="180" height="80" rx="6"/>
  <text class="t" x="305" y="175">ffdec (1 per session)</text>
  <text class="s" x="305" y="195">FFmpeg .so over libc.so</text>
  <text class="s" x="305" y="210">no FS, network, drivers</text>
  <rect class="b" x="560" y="20" width="180" height="60" rx="6"/>
  <text class="t" x="575" y="45">fsd / netd</text>
  <text class="s" x="575" y="65">source of bytes</text>
  <rect class="b" x="560" y="150" width="180" height="60" rx="6"/>
  <text class="t" x="575" y="175">audiod</text>
  <text class="s" x="575" y="195">sample ring + latency</text>
  <rect class="b" x="20" y="270" width="200" height="70" rx="6"/>
  <text class="t" x="35" y="295">compd</text>
  <text class="s" x="35" y="315">FRAME / PRESENTED,</text>
  <text class="s" x="35" y="330">window video layer</text>
  <rect class="b" x="290" y="270" width="180" height="70" rx="6"/>
  <text class="t" x="305" y="295">shared buffers</text>
  <text class="s" x="305" y="315">bufdesc NV12/I420/P010</text>
  <text class="s" x="305" y="330">+ acquire/release fences</text>
  <path class="a" d="M220,55 L288,55"/>
  <text class="s" x="225" y="48">mediaproto</text>
  <path class="a" d="M558,50 L472,50"/>
  <path class="a" d="M380,92 L380,148"/>
  <text class="s" x="386" y="125">input bytes</text>
  <path class="d" d="M472,185 L558,180"/>
  <text class="s" x="478" y="172">samples</text>
  <path class="d" d="M380,232 L380,268"/>
  <text class="s" x="386" y="255">frames</text>
  <path class="a" d="M288,305 L222,305"/>
  <path class="a" d="M120,92 L120,268"/>
  <text class="s" x="126" y="190">buffer commit</text>
</svg>

Solid arrows are control and trusted data, dashed ones the decoder
process output (only into shared memory someone handed to it).

### Roles

- **Player** (application) opens a session at `mediad`, hands it the input
  (decision 2), allocates or receives a buffer pool (decision 3), controls
  playback and composes the video into its window.
- **mediad** (service, trusted) holds sessions, reads input on behalf of the
  application (from a file or stream handle, not by path), starts `ffdec`
  through launchd with an empty namespace and passes it bytes only. It
  decides which codecs are allowed (decision 5).
- **ffdec** is the only process running FFmpeg code: libavformat (demux),
  libavcodec (decoding), libswresample (sample format conversion), libswscale
  when needed. It has no capabilities except the channel to `mediad`, buffer
  VMOs and fences.

### The `mediaproto` protocol [designed]

Messages over the application <-> `mediad` channel; `mediad` <-> `ffdec`
uses the same protocol extended with byte delivery. Operation numbers and
layout will live in the `libs/mediaproto` crate with host tests (like
`audioproto`).

| Operation | Who | Meaning |
|---|---|---|
| `OPEN { source, flags }` | application | new session; source = file / stream handle (decision 2) |
| `INFO -> { duration, streams[] }` | application | streams: type, codec, size, fps, channels, rate, language |
| `SELECT { video, audio, subtitles }` | application | stream selection (-1 = none) |
| `POOL { buffers[], fences }` | application | hands over the output buffer pool (VMO + `bufdesc`) |
| `AUDIO { ring }` | application | hands over the `audiod` stream ring (decision 4) |
| `PLAY` / `PAUSE` / `SEEK { time, exact }` | application | control; `SEEK` returns the actual keyframe time |
| `FRAME { buffer, pts, duration, fence }` | `mediad` -> application | the frame is in the buffer once the fence reaches the value |
| `RELEASE { buffer, fence }` | application | the buffer is free once compd signals the release fence |
| `CLOCK { audio pts, monotonic time }` | `mediad` -> application | clock for A/V sync |
| `SUBTITLE_SIZE { width, height, pixel aspect }` + `SUBTITLE { buffer, pts, duration, fence }` | application / `mediad` -> application | subtitle layer size; finished subtitle bitmap (RGBA, premultiplied alpha) |
| `EVENT { end, error, format change }` | `mediad` -> application | including a decoder crash (the session can be recovered) |

### Video: buffers and fences

Video output is buffers described by `bufdesc` (fourcc `NV12` / `YU12`
(I420) / `P010` for 10 bit, planes, strides, alignment). The decoder writes
a frame and advances the **acquire fence**; the compositor shows the frame
once the fence is reached and after the swap advances the **release
fence**; only then may the decoder reuse the buffer. FFmpeg software
decoders write into their own frames: `ffdec` uses `get_buffer2` so that
dav1d/libvpx/h264 decode **directly into pool VMOs** (no copy) with the
alignment FFmpeg requires (64 B stride and borders).

Note the dependency state:

- **Kernel fences (6a-1) are on `main`** ([fences.md](fences.md)).
- **`bufdesc`, `BUFFER_CREATE_DESC` and window sync points (6a-3, 6a-4) are
  not on `main` yet**, only on the branches `inferius-dev/6a-3-render` and
  `inferius-dev/6a-4-okna-video`. F3 waits for them; F1 and F2 (audio) do
  not.
- Showing YUV without CPU conversion needs a video layer in compd
  ([device-interfaces.md](device-interfaces.md), video plane [designed]);
  until then compd converts YUV -> RGB with a shader or on the CPU (cost:
  one extra copy of every frame, measured in F3).

### Audio

Decoded samples go to `audiod` through its **shared ring**; `audiod` mixes
and converts the rate. `ffdec` writes into the ring directly (libswresample
to the stream format, typically f32 / 48 kHz); neither the application nor
`mediad` copies samples. The ring header carries the played frame count
and the output latency, which is the clock source for sync.

### A/V synchronisation

- **The master clock is audio**: time = pts of the last written sample -
  (written - played frames + latency) / rate. Without an audio track the
  monotonic time is the clock.
- Video is scheduled from compd feedback: `FRAME { seq, deadline, period }`
  says when the next swap is, `PRESENTED { txn, time }` when the frame was
  actually lit. The player picks the frame whose pts best matches the swap
  time and commits it with a target time.
- Drift above ~1 frame: drop a frame (behind) or hold it (ahead); audio is
  never stretched, only a large jump (> 200 ms) resets the ring.
- Different rates (24 fps on 60 Hz): the 3:2 cadence follows naturally from
  compd deadlines; with VRR compd may adapt the period (later).

### Player (application) [designed]

An application over `libs/ui`. It needs `libs/ui` extensions:

- a **video layer widget**: a window area into which buffers from `mediad`
  are committed directly (not through a CPU image); today `libs/ui` only
  has a CPU image,
- controls: play/pause, seek bar with keyframe previews, volume, track
  selection, fullscreen, keyboard shortcuts and media keys,
- subtitles: full ASS/SSA, SRT and WebVTT through libass (see
  [Subtitles](#subtitles)); the player only composes finished bitmaps over
  the video,
- a **media session** for the system (now playing, controls from the panel,
  pause when headphones are unplugged) per
  [device-interfaces.md](device-interfaces.md).

## Codecs

Ordered by licensing risk. All decoders are software, inside FFmpeg or its
external libraries under compatible licences.

| Group | Codecs / containers | Source | Default |
|---|---|---|---|
| royalty-free | AV1 (dav1d, BSD-2), VP8/VP9 (built into FFmpeg), Opus, Vorbis, FLAC, Theora; WebM/Matroska, Ogg, MP4/ISOBMFF (container), WAV | FFmpeg + dav1d | on |
| patents expired | MP3, AC-3, MPEG-1/2 video | FFmpeg | always on |
| patent-encumbered | H.264, HEVC, AAC, E-AC-3 | native FFmpeg decoders only (LGPL) | **on in the default image**; an xtask switch builds an image without them |
| never | anything requiring `--enable-gpl`, `--enable-nonfree` (x264, x265, fdk-aac, GPL libpostproc) | - | hard off |

dav1d is faster than FFmpeg's built-in AV1 decoder and is BSD, so FFmpeg
uses it as an external library (its own C port via the same mechanism).
Encoders are not needed in F1-F4; screen recording is a separate later
step.

**Patent-encumbered codecs (decision 5 = B).** H.264, HEVC, AAC and
E-AC-3 are on in the default image, but **only FFmpeg's native decoders
under the LGPL**; no GPL libraries (x264, x265) and no nonfree ones
(fdk-aac). Encoders for these formats are not built.

> **Risk (knowingly accepted):** distributing the system with H.264, HEVC
> and AAC decoders may lead to **licence claims by patent pools** (MPEG LA
> / Via LA for H.264 and AAC, Access Advance and Via LA for HEVC), mainly
> in the USA and other countries where software patents are enforceable.
> The FFmpeg licence (LGPL) does not cover patents. Whoever distributes the
> system must assess this themselves; the text is also in
> `THIRD-PARTY.md`.

An **xtask switch** stays for building without them (working name
`--bez-patentovych-kodeku`): configure gets `--disable-decoder=` for h264,
hevc, aac, aac_latm, eac3 (and the matching parsers), `xtask` checks in
`config.h` after the build that they are off, and `mediad` then reports
them as unavailable (the application gets `INFO` with the codec and a
"codec unavailable" error, not a crash).

## Subtitles

Decision 9 = B: **libass** (ISC) with **HarfBuzz** (MIT, text shaping),
**FriBidi** (LGPL-2.1-or-later, bidirectional text) and **FreeType** (under
the FTL, not the GPL - FreeType is dual FTL/GPL-2 and we choose the FTL).

- **Why FreeType, not `libs/font`**: libass calls the FreeType API directly
  (`FT_Face`, `FT_Outline`, ...) and has no layer for another rasteriser.
  Replacing it would mean patching a large part of libass or wrapping the
  FreeType API over `libs/font`; both are costlier and more fragile than
  taking FreeType. `libs/font` stays the system font engine; FreeType lives
  only in the decoder process.
- **The FTL** requires a mention in the product documentation ("Portions of
  this software are copyright The FreeType Project"); it goes into the
  player's About and `THIRD-PARTY.md`.
- **Where it runs**: ASS subtitles are untrusted data (scripts with
  thousands of tags, embedded fonts), so libass runs **in the `ffdec`
  process** with the same isolation as the decoder. The output is bitmaps
  (`ASS_Image`: alpha mask + colour + position) that `ffdec` writes into a
  shared subtitle buffer (RGBA with premultiplied alpha) with a fence; the
  player only composes them over the video.
- **Fonts**: `ffdec` has no FS, so fontconfig is not built
  (`--disable-fontconfig`). `mediad` supplies fonts: embedded fonts from
  the container (Matroska attachments) and system fonts the script asks
  for are passed as bytes (`ass_add_font`); the default font is the system
  font from `design/tokens.toml`.
- **Size and time**: the player sends the video layer size and pixel
  aspect (`SUBTITLE_SIZE`), `ffdec` renders for the pts of the frame it is
  delivering; a window resize = a new render.
- SRT and WebVTT take the same path (FFmpeg converts them to ASS), so there
  is one renderer.

Subtitle library licences: ISC, MIT and the FTL are permissive; FriBidi
under the LGPL is linked dynamically like FFmpeg (same obligations, see
[LGPL obligations](#lgpl-obligations)).

## Hardware acceleration (later, F5)

GPU decoding (Intel Quick Sync, AMD VCN, NVIDIA NVDEC) comes after the GPU
drivers ([gpu-amd-nvidia.md](gpu-amd-nvidia.md)). The interface shape is
fixed now: `ffdec` gets a **video decode queue** from the GPU service (not
the device), the output is the same `bufdesc` buffers with fences, so
neither the player nor compd changes. An application never gets a
capability to a driver.

## Building FFmpeg

### C port in xtask

The generic C port mechanism (tarball + SHA-256 + patches + CMake/Ninja/
autotools) is being designed by the SDL thread (`navrh-sdl.md`, its question
2 = C). FFmpeg **uses it rather than duplicating it**. What it needs beyond
CMake/autotools:

1. **A custom `configure`** (shell script, not autoconf): run under `sh`
   (Git Bash / MSYS2 on a Windows host), `make` (GNU make) for the build.
   The mechanism must support a "different kind" of build: a configure
   command with switches, then `make -jN install`.
2. **Cross compilation**: `--enable-cross-compile --target-os=none` (or a
   patch adding `magnumopus`), `--arch=x86_64|aarch64`,
   `--cc=clang --cfg=...` with `clang-dyn.cfg` from the C sysroot,
   `--ar=llvm-ar`, `--nm=llvm-nm`, `--ranlib=llvm-ranlib`, `--pkg-config`
   pointing at `.pc` files in the sysroot (for dav1d).
3. **Assembler**: on x86_64 **nasm** (>= 2.13) for hand-written SIMD; the
   mechanism finds it and, if missing, gives install advice (decision 7).
   On aarch64 clang as the assembler is enough (GNU `.S` syntax, NEON).
4. **Patches**: `target-os` for our system, CPU count detection through
   `sysconf(_SC_NPROCESSORS_ONLN)` (on the cstd-posix branch), switches
   `--disable-network`, `--disable-autodetect`; FFmpeg must not reach for
   `fork` (cstd does not have it, only `posix_spawn`).
5. **Out-of-tree and per-arch**: build into `target/cports/<arch>/ffmpeg/`,
   install headers, `.so` and `.pc` into the C sysroot, fingerprint
   (version + patches + switches) to skip repeated builds.
6. **Dependencies between ports**: dav1d (Meson + Ninja) before FFmpeg;
   for subtitles FreeType (CMake), FriBidi and HarfBuzz (Meson) and libass
   (autotools) before `ffdec`. Meson is another build system the SDL
   mechanism does not cover yet; the mechanism must support it (or at
   least "own configure command + build", as for FFmpeg).
7. **Tool check** (decision 8 = A): before the build xtask checks `sh`,
   `make`, `nasm` (x86_64), `meson`, `ninja`, `pkg-config` from Git Bash /
   MSYS2 and, if a tool is missing, stops with exact advice on how to
   install it (no silent skipping of the port).

Licence switches hard-coded: `--disable-gpl --disable-nonfree
--disable-version3` (LGPL 2.1+), `--enable-shared --disable-static`,
`--disable-programs` (own `ffdec` instead of `ffmpeg`), `--disable-doc`.
After configure `xtask` checks `config.h` for `CONFIG_GPL 0` and
`CONFIG_NONFREE 0`, otherwise the build fails.

### What it builds on in the system

- `libc.so` (the libc.so step on the `inferius-dev/libc-so` branch, waiting
  for merge) and the dynamic loader `ld.so`: FFmpeg is a set of `.so` files
  (`libavformat.so.61`, `libavcodec.so.61`, `libavutil.so.59`,
  `libswresample.so.5`, `libswscale.so.8`) linked against `libc.so`.
- `cstd` with the POSIX layer (branch `inferius-dev/cstd-posix`): pthreads
  (FFmpeg decodes in threads), `clock_gettime`, `sysconf`,
  `posix_memalign` for aligned allocations.
- W^X: neither FFmpeg nor dav1d needs a JIT; hand-written SIMD is
  precompiled.
- AVX/XSAVE: the requirement is already described in [cstd.md](cstd.md)
  (FFmpeg detects AVX2/AVX-512 at runtime through `cpuid` + `xgetbv`; the
  kernel must save the state).

## LGPL obligations

LGPL 2.1+ allows using FFmpeg in a closed system as long as the user can
replace the library with their own modified version. The design:

- FFmpeg is **dynamically linked** (`.so` over `libc.so`), the `ffdec`
  process is only a thin layer; replacing the library = replacing files in
  `/lib` (decision 6),
- **sources**: the exact tarball (SHA-256), all patches and configure
  switches are in the repository (`third_party/ffmpeg/`); the system image
  carries a file pointing to them and the licence text,
- **rebuild script**: `cargo xtask cport ffmpeg` builds the same library
  from the same sources; described in the user manual,
- `THIRD-PARTY.md` gets rows for FFmpeg (LGPL-2.1-or-later), dav1d
  (BSD-2-Clause), libass (ISC), HarfBuzz (MIT), FriBidi
  (LGPL-2.1-or-later) and FreeType (FTL) with version and hash,
- FriBidi is under the LGPL like FFmpeg: a shared `.so`, sources and a
  rebuild script the same way,
- the player's About states that it uses FFmpeg under the LGPL, with a link
  to the sources.

## Stages

1. **F1 port**: dav1d + FFmpeg (and FreeType, FriBidi, HarfBuzz, libass)
   through the port mechanism, QEMU test: a
   program over `libavformat` lists the streams of a test WebM file
   (Opus + VP9) and decodes 10 frames with a checksum, both architectures.
2. **F2 service and audio**: `mediaproto`, `mediad`, `ffdec`; playing
   Opus/FLAC into `audiod`; decoder crash test (the session reports an
   error, `mediad` survives). The protocol, service and isolation are built
   **first with a test decoder** (a process with the same interface as
   `ffdec` that generates a tone and test patterns without FFmpeg), so they
   do not wait for C ports.
3. **F3 video**: `bufdesc` buffers with fences (after 6a-3/6a-4 are merged),
   A/V sync, measuring latency and dropped frames.
4. **F4 player**: application, video layer widget in `libs/ui`, subtitles,
   media session.
5. **F5 HW acceleration**: after the GPU drivers.

## Decisions

Decided by the user on 2026-11-14 (original A/B/C questions; the
recommendation was always A).

| # | Question | Decision |
|---|---|---|
| 1 | decoder isolation | **A** one `ffdec` process per session |
| 2 | input to the decoder | **A** the application gives a handle to `mediad`, which sends bytes; network streams are read by `mediad` through `netd`, FFmpeg has `--disable-network` |
| 3 | video path | **A** the application allocates the buffer pool, `ffdec` decodes into it, the application commits into its window |
| 4 | audio ring | **A** the application opens the `audiod` stream and passes the ring through `mediad` |
| 5 | patent codecs | **B** H.264, HEVC, AAC on in the default image (native LGPL FFmpeg decoders only); MP3, AC-3, MPEG-2 always; an xtask switch for an image without them; the patent pool risk stated explicitly in the docs and `THIRD-PARTY.md` |
| 6 | linking | **A** shared `.so` over `libc.so` |
| 7 | nasm | **A** required on x86_64 |
| 8 | host | **A** Git Bash / MSYS2, tool check in xtask |
| 9 | subtitles | **B** libass + HarfBuzz + FriBidi + FreeType (FTL) right away, inside the `ffdec` process |
| 10 | order | **A** F1 -> F2 -> F3 -> F4; parts without C ports (protocol, `mediad`, player with a test decoder) may start earlier |
