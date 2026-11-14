# SDL

A port of the **SDL 3** library (Simple DirectMedia Layer, zlib licence)
for C programs: window, drawing, keyboard, mouse, threads and timers. SDL
is step 6 of the software takeover order; it builds on the
[C library](cstd.md) and the [port mechanism](c-ports.md). Games and
players in C (and later FFmpeg `ffplay`) thus run in a compositor window
without changes to their code.

## Status

| Part | What | Status |
|---|---|---|
| build | SDL 3.4.20 (pinned version), CMake + Ninja, static `libSDL3.a` in the port prefix | **[verified in QEMU]** x86_64 and aarch64 (suite `sdl`) |
| `libs/mosdl` | the C interface `mosdl_*` over the window from `libs/ui`, part of `libc.a` | **[verified in QEMU]** x86_64 and aarch64; event translation by host tests |
| `magnumopus` video driver | one window, software framebuffer, keyboard (evdev codes), text, mouse, wheel, focus, resize, waiting for events with wake-up | **[verified in QEMU]** `testsprite` (pixels), `checkkeys` (injected input) |
| clipboard, cursor, relative mouse, fullscreen, display scale | stage 6-2: text in the clipboard, system shapes and an own 64 × 64 cursor, compositor pointer lock, fullscreen on request, scale as `SDL_GetDisplayContentScale` ([Cursor and pointer lock](gui.md#cursor-and-pointer-lock)) | **[verified in QEMU]** x86_64 and aarch64 (suite `sdl`, physical input through QMP) |
| audio (playback, capture with permission) | stage 6-2 | **[designed]** |
| zero-copy YUV (`BUFFER_CREATE_DESC`) | postponed to the FFmpeg step (a patch of SDL internals, estimated ~300 lines) | **[designed]** |
| SDL example games (`snake`, `woodeneye-008`), a test with a screenshot | stage 6-3 | **[designed]** |
| gamepads | a separate step with its own design (decision Q6 B) | **[designed]** |

## How it works

<figure class="diagram">
<svg viewBox="0 0 780 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="An SDL program in a compositor window">
<defs><marker id="sd1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="10" width="240" height="60" rx="6"/><text class="t b" x="130" y="34" text-anchor="middle">C program</text><text class="m" x="130" y="52" text-anchor="middle">testsprite, game, player</text>
<rect class="acc" x="10" y="100" width="240" height="60" rx="6"/><text class="t b" x="130" y="124" text-anchor="middle">libSDL3.a</text><text class="m" x="130" y="142" text-anchor="middle">magnumopus video driver</text>
<rect class="acc" x="300" y="100" width="220" height="60" rx="6"/><text class="t b" x="410" y="124" text-anchor="middle">mosdl (in libc.a)</text><text class="m" x="410" y="142" text-anchor="middle">mosdl_* over ui::client::Window</text>
<rect class="ok" x="570" y="100" width="200" height="60" rx="6"/><text class="t b" x="670" y="124" text-anchor="middle">compd</text><text class="m" x="670" y="142" text-anchor="middle">window, buffers, input</text>
<rect class="box" x="300" y="190" width="220" height="50" rx="6"/><text class="t b" x="410" y="212" text-anchor="middle">osrt</text><text class="m" x="410" y="228" text-anchor="middle">window handles from startup</text>
<path class="ln" d="M130 70 L130 96" marker-end="url(#sd1)"/>
<path class="ln" d="M250 130 L296 130" marker-end="url(#sd1)"/>
<path class="ln" d="M520 130 L566 130" marker-end="url(#sd1)"/>
<path class="ln" d="M410 186 L410 164" marker-end="url(#sd1)"/>
</svg>
<figcaption>The program calls SDL; the magnumopus driver calls mosdl in libc.a, which talks to the compositor over the window protocol. The process gets the window channel at startup.</figcaption>
</figure>

- **The video driver** (`third_party/c-ports/sdl3/overlay/src/video/magnumopus/`)
  is our C code inside SDL. It calls only the `mosdl_*` functions from the
  `mosdl.h` header in the C sysroot. The patch `0001-video-magnumopus.patch`
  registers it in SDL and adds the `SDL_PLATFORM_MAGNUMOPUS` macro.
- **`libs/mosdl`** is in Rust and is part of `libc.a`. Two static Rust
  libraries in one program would each have their own copy of std
  (allocator, TLS, threads). A program without SDL links nothing from
  `mosdl` (`--gc-sections`).
- **The window**: `mosdl` holds a `ui::client::Window` (the same layer as
  the terminal window), so an SDL window can do what other windows do:
  double buffers, `CONFIGURE` on resize, recovery after a compositor crash
  (SDL gets `SDL_EVENT_WINDOW_EXPOSED` and redraws).
- **Startup**: an application with a window gets untagged handles in the
  startup message, 0 = the window channel from the compositor, 1 = the
  recovery lease. The `osrt` runtime keeps them for `mosdl`
  (`take_handles`); other handles (stdout, FS) are tagged as for every
  application with std.

### The `mosdl.h` interface

| Function | What |
|---|---|
| `mosdl_available` | does the process have a window channel? (otherwise SDL picks another driver) |
| `mosdl_window_open`, `_close`, `_set_title`, `_set_size` | the window (one per process) |
| `mosdl_window_present` | a B G R A frame (`XRGB8888`) into the window buffer |
| `mosdl_next_event` | the next event with a timeout |
| `mosdl_wake` | wakes the wait from another thread (`SDL_WaitEvent`) |
| `mosdl_display_info` | screen, work area, scale, refresh rate |

The functions return 0 or a negative kernel error code ([errors](errors.md)).
A host test checks the `mosdl_event` layout and the constants against the
header.

### Input

| Window event | SDL |
|---|---|
| key (evdev code) | `SDL_EVENT_KEY_DOWN`/`UP`, scancode from the Linux table (`SDL_GetScancodeFromTable`) |
| Caps Lock, Num Lock | from the system state (`SDL_ToggleModState`) |
| text | `SDL_EVENT_TEXT_INPUT` (only with text input enabled, as elsewhere) |
| motion, button, wheel | `SDL_EVENT_MOUSE_*` (evdev buttons `BTN_LEFT` ... `BTN_EXTRA` = left ... X2) |
| focus, pointer leaving | SDL keyboard and mouse focus |
| `CONFIGURE`, close | `SDL_EVENT_WINDOW_RESIZED`, `SDL_EVENT_WINDOW_CLOSE_REQUESTED` |

The event time (ns since system start) is converted to SDL time.

## Limitations of stage 6-1

- **One window per process.** A second window returns the SDL error
  "Unsupported". Several windows over one channel come with the window
  protocol for multiple application windows.
- **The whole frame is copied** on `SDL_UpdateWindowSurface` /
  `SDL_RenderPresent` (the window buffers are double, a partial copy would
  leave older content). 1920 × 1080 = 8 MiB per frame; zero-copy video
  (`BUFFER_CREATE_DESC`) is stage 6-2.
- **Software renderer.** Vulkan for SDL (render and the GPU API) comes with
  GPU drivers for applications ([render](render.md)).
- **No audio** (SDL audio driver `dummy`): stage 6-2.
- **HiDPI only as a scale**: window buffers are 1:1, the compositor has no
  scale other than 100 % and no buffer scaling yet; SDL gets the scale as
  `SDL_GetDisplayContentScale` (pixel density 1).
- **Clipboard text only** (`text/plain;charset=utf-8`); copy, paste,
  pointer lock and entering fullscreen only in response to a physical
  press (compositor rule). There is no `SDL_WarpMouse` (relative mode does
  not need it with the lock).
- **No gamepads** (decision Q6 B), file dialogs (Q7 A: SDL returns
  unsupported), tray (`SDL_TRAY`) or `SDL_OpenURL` (will go through the
  "open path" service).
- **Thread priority** only `SCHED_OTHER` (the scheduler does not give
  applications real time yet): `SDL_SetCurrentThreadPriority` with the time
  critical priority returns an error and SDL continues.
- **Static library** (decision Q8 A). `libSDL3.so` comes with `libc.so`.

## Building

The `sdl3` port in `xtask/src/cports/mod.rs` (method `CMake`, [C ports](c-ports.md)).
Main options: `SDL_STATIC=ON`, `SDL_SHARED=OFF`, `SDL_UNIX_CONSOLE_BUILD=ON`
(windows come from our driver, not X11 or Wayland), X11, Wayland, KMSDRM,
OpenGL, Vulkan, GPU, ALSA, PulseAudio, PipeWire, D-Bus, HIDAPI, joysticks,
camera, dialogs and tray switched off. The library and headers go to
`target/c-ports/<arch>/prefix`, the SDL tests `testsprite` and `checkkeys`
to the initfs together with manifests and an icon.

What SDL needed from the C library and was added in this step
([cstd](cstd.md#sdl-step-6)): `environ`, `fenv.h`, `nl_langinfo`, `sched_param` and
`pthread_*schedparam`, `SIGVTALRM`/`SIGPROF`, the headers `wchar.h` (types
only so far) and `inttypes.h`, empty `libm.a`, `libpthread.a`, `librt.a`.

## Tests

| Test | Where | What it verifies |
|---|---|---|
| `cargo test -p mosdl` | host | event translation, frame copy (cropping, padding, hostile sizes), match with `mosdl.h` |
| `sdl: testsprite ...` | QEMU, suite `sdl` | an SDL program window in the compositor; a deterministic frame (`--iterations 0`): red corner point, green top, blue left and red bottom line on screen |
| `sdl: checkkeys ...` | QEMU, suite `sdl` | injected input through inputd and the compositor: key A = scancode 4, text "a", right mouse button, Esc twice ends the program with code 0 |
| `sdl: kurzor, zamek ukazatele, schranka a cela obrazovka (QMP)` | QEMU, suite `sdl` | the `testmagnumopus` program (our SDL test): display and scale as the compositor, crosshair cursor on screen; physical click = pointer lock, movement = relative motion, Super = lock cancelled, second click = copy, Ctrl+V = paste, F = fullscreen |
| `sdl: mereni cesty videa 1080p` | QEMU, suite `sdl` | `testmagnumopus bench`: I420 1920 × 1080 -> conversion in SDL -> window ([Performance](#performance)) |

```text
cargo xtask test --only sdl
cargo xtask test --arch aarch64 --only sdl
```

## Performance

Today's video path through SDL (input for the zero-copy YUV decision): an
I420 1920 × 1080 texture -> `SDL_UpdateYUVTexture` (conversion to RGB in
SDL) -> `SDL_RenderTexture` into a 960 × 540 window -> `SDL_RenderPresent`
(copy into the window buffer through `mosdl`). Mean of 60 frames after 10
warm-up frames (`testmagnumopus bench`, test `sdl: mereni cesty videa
1080p`).

| Architecture | System build | ms per frame |
|---|---|---|
| x86_64 | debug (`cargo xtask test`) | 139.6 |
| x86_64 | release (`--release`) | 102.0 |
| aarch64 | debug | 150.4 |

Conditions: revision `5ab30f2`, QEMU 11.1.0 without acceleration (TCG),
`-cpu max` (aarch64 `cortex-a76`), `-smp 4`, `-m 1G`, host AMD Ryzen 9
9950X3D, Windows 11. SDL built as Release (CMake), on 1 thread. Numbers
from TCG emulation are for comparing paths, not for a real machine;
measuring on hardware with KVM/WHPX is **[designed]**.

**Zero-copy YUV** (the frame as NV12/I420 through `BUFFER_CREATE_DESC`,
SIMD conversion in the compositor, later on the GPU) is a follow-up step
together with FFmpeg: a patch of `SDL_render.c` (an unconverted YUV texture
for the magnumopus window, detecting a single YUV texture over the whole
window in `RenderPresent`), `ui::Window::present_yuv` and a test; estimated
~300 lines of SDL patch **[designed]**. The system player goes through
`mediad` and windows with `bufdesc` directly, SDL and `ffplay` are a side
path.

## Decisions (2026-11-14)

| Question | Decision |
|---|---|
| version | SDL 3.4.x, pinned (3.4.20) |
| build | a generic port mechanism in xtask (archive, SHA-256, patches, CMake/Ninja and configure) |
| interface | `libs/mosdl` in Rust with a C interface over `libs/ui`, `audioproto`, `clipproto` |
| drawing | software renderer (6-1), zero-copy YUV through `BUFFER_CREATE_DESC` (6-2), Vulkan later |
| cursor and mouse | `SET_CURSOR` and `POINTER_LOCK` in the window protocol (6-2) |
| gamepads | a separate step |
| file dialogs | unsupported |
| library | static `libSDL3.a` |
| proof | SDL examples `snake`, `woodeneye-008` and the tests `testsprite`, `checkkeys`, `testaudio` |
| audio capture | `[audio] capture = true` in the manifest and a prompt on first use (6-2) |
