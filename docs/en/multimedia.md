# Multimedia: FFmpeg and the player

Design of step 7 of the order in [architecture.md](architecture.md):
decoding audio and video with the third-party FFmpeg library in an
**isolated process**, with a player on top. The state of each part is in
the table below; the service, the protocol and the test decoder already
run, FFmpeg and the player do not yet. The user's decisions (2026-11-14) are
in [Decisions](#decisions). Parts depending on C ports (F1, building
FFmpeg and the subtitle libraries) wait until the C port mechanism from the
SDL thread is on `main`; the service, the protocol and the player with a
test decoder do not depend on it.

## Status

| Part | What | Status |
|---|---|---|
| F1 | FFmpeg 9.0.2 and dav1d 1.5.4 as C ports (LGPL, shared libraries over `libc.so`, dav1d inside `libavcodec.so`), licence check in `config.h`, switch without patent-encumbered codecs | **[verified in QEMU]** x86_64 and aarch64 (suite `media ffmpeg`) |
| F2a | `libs/mediaproto` protocol (host tests), `mediad` service, `mediatest` test decoder (NV12 frames into the pool with a fence, tone into the `audiod` ring), VMO and channel sources, `SEEK`, decoder crash and recovery | **[verified in QEMU]** x86_64 and aarch64 (suite `media`) |
| F2b | `ffdec` process (C, PIE over `libc.so` and `libav*.so`): AV1 and VP9 bit-exact against test vector MD5s, WAV audio over a source channel into `audiod`, seek, corrupted input | **[verified in QEMU]** x86_64 and aarch64 (suite `media ffmpeg`) |
| F3 | video into shared buffers (`bufdesc` NV12/I420/P010) with fences, A/V sync | **[designed]** |
| F4a | video area in `libs/ui` (`node::video`, a layer from buffers with fences), Player application with a built-in sample: play/pause, slider (seek), time, A/V sync by audio | **[verified in QEMU]** x86_64 and aarch64 (suite `gui`, player test) |
| F4b | file choice, libass subtitles, volume and track selection, fullscreen, system media session | **[designed]** |
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
- **mediad** (service, trusted, under the init supervisor) holds sessions
  (one thread per session), reads input on behalf of the application and
  starts the decoder directly (`spawn_with`) in its own job with limits
  (64 MiB user, 4 MiB kernel memory) **without any handles** except the
  control channel. It checks decoder events before forwarding them (buffer
  number within the pool, the decoder may not announce its own loss). It
  decides which codecs are allowed (decision 5).
- **ffdec** is the only process running FFmpeg code: libavformat (demux),
  libavcodec (decoding), libswresample (sample format conversion), libswscale
  when needed. It has no capabilities except the channel to `mediad`, buffer
  VMOs and fences.

### The `mediaproto` protocol [verified in QEMU]

Crate `libs/mediaproto` (`wire` encoding; reading an invalid message
returns `INVALID_ARGS`, never a panic). **One client channel = one
session**; another session = another channel (`CONNECT` on the service
control channel held by init). `mediad` sends the decoder the same
requests (plus `PROBE`) and forwards its events to the application.

Messages from the service start with a u32 tag: `0` = reply to the last
request (status i32 + data), otherwise an event number. Events arrive at
any time, also while waiting for a reply (the `MediaClient` client queues
them).

| Operation | Number | Parameters -> reply | Handles |
|---|---|---|---|
| `CONNECT` | 1 | -> new session channel | reply: channel |
| `OPEN` | 2 | source kind u8 (`1` VMO, `2` channel), size u64 -> `Info` | read-only VMO / source channel |
| `SELECT` | 3 | video, audio, subtitles i32 (`-1` = none) | - |
| `POOL` | 4 | count u8 + `BufDesc` of each buffer | buffer VMOs, then the frame fence |
| `AUDIO` | 5 | rate u32, channels u8, sample u8, capacity u32 | ring VMO, period event |
| `PLAY` / `PAUSE` | 6 / 7 | - | - |
| `SEEK` | 8 | time us u64, exact u8 -> actual time u64 | - |
| `RELEASE` | 9 | buffer u8 | - |
| `CLOSE` | 10 | end of session (the decoder exits) | - |
| `REOPEN` | 11 | after decoder loss: a new one with the same source, pool, ring and selection -> `Info` | - |
| `PROBE` | 50 | `mediad` -> decoder only: size u64 -> `Info` | source channel |

| Event | Number | Content |
|---|---|---|
| `FRAME` | 100 | buffer u8, pts us u64, duration us u32, serial u64 (= frame fence value) |
| `AUDIO_BASE` | 101 | ring position u64, pts us u64 (audio clock anchor after start and `SEEK`) |
| `END` | 102 | end of all selected streams |
| `ERROR` | 103 | error code i32, decoder lost u8 (the session can be recovered with `REOPEN`) |

`Info` = duration us u64 and streams (kind, codec, size, fps as a
fraction, rate, channels, language, codec available). Subtitles
(`SUBTITLE_SIZE`, `SUBTITLE`) come with libass in F4 and are not in the
protocol yet.

**Byte source** (`mediaproto::source`, a deviation from the design): fsd
cannot yet hand over one open file as a transferable handle (file handles
are valid only within an fsd session). The source is therefore either a
**read-only VMO** (small files, tests) or a **source channel** on which the
application answers `READ_AT { offset u64, length u32 }` (at most 60 KiB so the reply fits a 64 KiB channel message)
and `SIZE`. The decoder always reads through its source channel from
`mediad` and never gets the VMO handle or the application channel. A
narrow single-file handle in fsd is a separate step.

**Clock** (`mediaproto::clock`, host tests): audio time = anchor pts +
(played frames - latency * rate - anchor position) / rate; without audio
`WallClock`. Frame choice `decide`: a frame belongs to a swap when the swap
time falls into `[pts - duration/2, pts + duration)`; earlier = hold,
later = drop, a deviation over 200 ms = resync by a jump.

### Test decoder `mediatest` [verified in QEMU]

A process with the same interface as the future `ffdec`, without FFmpeg.
It reads the `MTM1` test format (`mediaproto::testmedia`: a header + a
record per frame with pts, seed and keyframe flag) through the source
channel, draws a deterministic NV12 picture (luma gradient, moving bar,
colour from the seed) directly into the pool VMO, advances the frame fence
and sends `FRAME`; it writes an s16 stereo tone into the `audiod` ring. The
`crash_at` header field makes it exit on purpose at a given frame (session
recovery test). It decodes ahead while it has free buffers; the pace is set
by the application returning buffers (`RELEASE`) and by the audio ring.

Test decoder limits: NV12 and s16 stereo only; after `SEEK` the audio
already in the ring plays out (a producer cannot empty the ring itself -
the new `AUDIO_BASE` anchor tells the application).

Suite `media` (`user/init/src/tests/media.rs`) checks: stream description,
frame content and order against a checksum and the fence, keyframe and
exact seek, a source channel from the application, audio into `audiod`
with the audio clock advancing and `END`, decoder crash (`ERROR` with
loss, `REOPEN`, `SEEK`, a frame after recovery), invalid requests and a
corrupted file.

### Video: buffers and fences

Video output is buffers described by `bufdesc` (fourcc `NV12` / `YU12`
(I420) / `P010` for 10 bit, planes, strides, alignment). The decoder writes
a frame and advances the **acquire fence**; the compositor shows the frame
once the fence is reached and after the swap advances the **release
fence**; only then may the decoder reuse the buffer. FFmpeg software
decoders write into their own frames (I420, 10-bit formats); for now `ffdec`
**copies** them into an NV12 pool buffer with libswscale (at the same size a
lossless chroma interleave, verified by MD5). Decoding **directly into pool
VMOs** (`get_buffer2` with an I420 pool and 64 B alignment, no copy) is the
next step together with F3.

Note the dependency state:

- **Kernel fences (6a-1) are on `main`** ([fences.md](fences.md)).
- **`bufdesc`, `BUFFER_CREATE_DESC` and window sync points (6a-3, 6a-4) are
  on `main`**; compd converts YUV on the CPU (`libs/yuv`) or in `vkcomp`.
  `libs/ui` has a video area (`node::video`, F4a).
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

### Player (application)

**Video area in `libs/ui`** [verified in QEMU]: the `video(id, w, h, alt)`
node only reserves space in the layout (natural size = video size, less
space scales it keeping the aspect ratio, image role in accessibility). The
client shows the picture (`ui::client::run_video` with `VideoSurface`:
`bufdesc` descriptions, VMOs, acquire and release fences) in its own layer
above the window (`900 + id`): `LAYER_SET` with the buffer and rectangle,
`SYNC_POINTS` and a commit, **without redrawing the window**. While
`App::wants_frames`, the client ticks with compositor frames (`FRAME` with
the swap deadline) and asks `App::video_frame` what to show;
`App::video_shown` returns the frame's release fence value. After a
compositor crash the client recreates the surface on the next frame.

**Player** (`user/gui-player`) [verified in QEMU]: for now it plays a
built-in `MTM1` sample (10 s, 640x360, tone) through `mediad` and
`mediatest`. A pool of 4 NV12 buffers, an `audiod` stream (the ring goes to
the decoder); the clock follows audio (`AUDIO_BASE` + played frames and
ring latency), without a sound card the monotonic time. On each compositor
frame `clock::decide` picks the frame for the swap time, late ones are
dropped, a buffer goes back to the decoder only after the release fence.
Controls: play/pause, slider (seek to a keyframe), time. A decoder crash
is recovered (`REOPEN` + `SEEK`). Test in suite `gui`: empty area before
playing, a picture and its motion after "Přehrát" (Play), time advancing,
a still picture after pause.

Remaining (F4b) [designed]:

- file choice (source over a channel from the application until fsd has a
  narrow file handle), seek bar with keyframe previews, volume, track
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

The **xtask switch** for building without them is
`XTASK_BEZ_PATENTOVYCH_KODEKU=1` (environment variable of `cargo xtask
build` / `test`): configure gets `--disable-decoder=` for h264, hevc, aac,
aac_fixed, aac_latm, eac3, vvc and `--disable-parser=` for the matching
parsers, `xtask` checks in `config_components.h` after the build that they
are off (otherwise the build fails). `ffdec` then reports them in `INFO`
with `available = 0` (the application gets the stream description, not a
crash).

## Subtitles

> **Not shown yet.** HarfBuzz is written in C++ and needs the C++ standard
> library (libc++ headers); libass has required HarfBuzz since 0.17. The
> C++ runtime (step 7a: libc++, libunwind) is not in the system yet, so the
> libass, HarfBuzz, FriBidi and FreeType ports are built only after it
> (coordinator decision 2026-11-15). Until then the player only lists
> subtitle streams in `INFO` and does not select them.

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

## Building FFmpeg [verified in QEMU]

FFmpeg and dav1d are built by the generic C port mechanism in xtask
([C ports](c-ports.md)) - a tarball from the official site with SHA-256,
no source patches:

- **dav1d 1.5.4** with the `Meson` method: a static library with `-fPIC`
  (`b_staticpic`), x86_64 with hand-written SIMD in nasm, aarch64 NEON via
  clang. FFmpeg finds it through `dav1d.pc` in the port prefix and puts it
  into `libavcodec.so`.
- **FFmpeg 9.0.2** with the `Configure` method and shared libraries:
  `--enable-cross-compile --target-os=none --arch=<arch> --cc=clang` (LLVM
  tools by name), flags of the dynamic C sysroot (`-fPIC`, link
  `-Wl,-Bdynamic`, `-Wl,-shared` for libraries, `crt1.o` and the loader only
  for configure's test programs), `--pkg-config=pkgconf`. No `target-os` or
  CPU detection patch was needed: `none` is a generic POSIX target and the
  CPU count comes from `sysconf` in cstd.
- **Decoding only**: `--disable-encoders --disable-muxers
  --disable-filters --disable-avfilter --disable-avdevice --disable-devices
  --disable-hwaccels --disable-protocols --disable-network
  --disable-autodetect --disable-programs`; demuxers, parsers, decoders and
  bsfs stay (the `FFMPEG_ARGS` list in `xtask/src/cports/mod.rs`).
- **Licence hard-coded**: `--disable-gpl --disable-version3
  --disable-nonfree`; after the build xtask checks `config.h` for
  `CONFIG_GPL 0`, `CONFIG_NONFREE 0`, `CONFIG_VERSION3 0` and
  `CONFIG_SHARED 1`, otherwise the build fails (host test
  `ffmpeg_config_check`).
- **Output**: `lib/libavutil.so.61`, `lib/libswresample.so.7`,
  `lib/libswscale.so.10`, `lib/libavcodec.so.63` (with dav1d),
  `lib/libavformat.so.63` in the initfs (no TEXTREL, BIND_NOW, symbol
  versions `LIBAVCODEC_63` ...). They add about 20 MiB per architecture, so
  the disk image is now 256 MiB (`xtask/src/image.rs`).
- **Tools**: `sh`, `make`, nasm, Meson, Ninja and pkgconf from MSYS2
  (decision 8 = A); a missing one = an error with install advice
  ([Building](building.md#tools)).

What it builds on in the system:

- `libc.so` and the `ld.so` loader (libraries from `/system/lib` via libd),
- `cstd`: pthreads (FFmpeg decodes in threads), `clock_gettime`, `sysconf`,
  new `posix_memalign` / `aligned_alloc` / `memalign` (SIMD alignment) and
  the `static_assert` macro (C11),
- `magnumopus/sys.h` (`libs/mosys` in `libc.so`): channels, VMOs, fences
  and waiting for `ffdec` in C,
- W^X: neither FFmpeg nor dav1d needs a JIT,
- AVX/XSAVE: FFmpeg detects AVX2/AVX-512 at runtime ([cstd.md](cstd.md)).

### The `ffdec` process [verified in QEMU]

`user/ffdec` is a C program (PIE over `libc.so` and `libav*.so`) speaking
the `mediaproto` protocol over the bootstrap channel (message encoding in
`proto.c`, kept in sync with Rust by the host test `c_header_tests`).
`mediad` starts it once per session in its own job (256 MiB of user
memory) **without any handles** except the control channel; it picks the
decoder by the first bytes of the source (`MTM1` = `mediatest`, otherwise
`ffdec`).

- **Input**: libavformat with its own `AVIOContext` over the source channel
  (`READ_AT` of 60 KiB, `SIZE`); no files, network or FFmpeg protocols.
- **Video**: libavcodec (threads per CPU count) -> libswscale into an NV12
  pool buffer -> frame fence -> `FRAME`.
- **Audio**: libswresample to s16 stereo at the ring rate -> directly into
  the `audiod` ring; `AUDIO_BASE` with the position and pts of the first
  sample after start and seek.
- **Pace**: decodes ahead while it has a free buffer and room in the ring.
- **Seek**: `avformat_seek_file` backwards to a keyframe; an exact seek
  drops frames before the target.

Suite `media ffmpeg` (`user/init/src/tests/media_ffmpeg.rs`): stream
description of AV1 and VP9, **bit-exact picture** (I420 rebuilt from NV12
against the MD5 of every frame from the AOM and libvpx test vectors), seek
to the start after the end, WAV audio over a source channel into `audiod`,
random and truncated inputs (`mediad` survives).

## LGPL obligations

LGPL 2.1+ allows using FFmpeg in a closed system as long as the user can
replace the library with their own modified version:

- FFmpeg is **dynamically linked** (`.so` over `libc.so`), the `ffdec`
  process is only a thin layer; replacing the library = replacing files in
  `/system/lib` (decision 6),
- **sources**: the pinned tarball (SHA-256, signature by the FFmpeg key
  checked when pinned), configure switches in `xtask/src/cports/mod.rs`
  (`FFMPEG_ARGS`), no patches,
- **rebuild script**: `cargo xtask build` builds the same libraries from
  the same sources (described in [C ports](c-ports.md)),
- `THIRD-PARTY.md` has rows for FFmpeg (LGPL-2.1-or-later) and dav1d
  (BSD-2-Clause) with version and hash; libass (ISC), HarfBuzz (MIT),
  FriBidi (LGPL-2.1-or-later) and FreeType (FTL) come with subtitles,
- FriBidi will be under the LGPL like FFmpeg: a shared `.so`, sources and a
  rebuild script the same way,
- the player's About states that it uses FFmpeg under the LGPL, with a link
  to the sources (with file choice in F4b).

## Stages

1. **F1 port** (done): dav1d + FFmpeg through the port mechanism; the QEMU
   test decodes the AV1 and VP9 test vectors through `ffdec` bit-exact
   against their MD5s, both architectures. FreeType, FriBidi, HarfBuzz and
   libass only after step 7a (C++ runtime).
2. **F2 service and audio**: `mediaproto`, `mediad`, `ffdec`; playing
   Opus/FLAC into `audiod`; decoder crash test (the session reports an
   error, `mediad` survives). The protocol, service and isolation are built
   **first with a test decoder** (a process with the same interface as
   `ffdec` that generates a tone and test patterns without FFmpeg), so they
   do not wait for C ports.
3. **F3 video**: `bufdesc` buffers with fences (6a-3/6a-4 are on `main`),
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
