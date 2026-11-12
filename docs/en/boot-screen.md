# Boot screen

What the user sees from power-on to the desktop: the logo with a progress
ring on a warm background, without firmware, loader or kernel text, and a
smooth hand-over to the desktop. The design is `design/start.html`
(variant A, log from variant B, hand-over into the avatar); the original
variants are in `design/start-varianty.html`.

| Code | What it does |
|---|---|
| `libs/bootsplash` | layout, logo and wordmark from `design/assets`, ring, glow, background; the same picture in every phase |
| `loader/src/splash.rs`, `loader/src/logger.rs` | first frame right after taking the GOP, loader output only to the serial line |
| `kernel/src/console.rs` | with `SPLASH` the kernel neither draws into nor clears the framebuffer |
| `user/splash` | separate process: fade-in, breathing, ring by boot steps, "waiting for", failure screen |
| `user/init/src/splash.rs` | starting the `splash` process, boot steps, ending it before the display |
| `user/compd/src/splash.rs` | the compositor's first frame is the boot picture, hand-over to the desktop |

## Sequence

<figure class="diagram">
<svg viewBox="0 0 780 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Boot screen sequence">
<defs><marker id="st1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="140" height="80" rx="6"/><text class="t b" x="80" y="44" text-anchor="middle">loader</text><text class="m" x="80" y="63" text-anchor="middle">black + mark</text><text class="m" x="80" y="80" text-anchor="middle">empty ring</text>
<rect class="box" x="165" y="20" width="140" height="80" rx="6"/><text class="t b" x="235" y="44" text-anchor="middle">kernel</text><text class="m" x="235" y="63" text-anchor="middle">keeps the picture</text><text class="m" x="235" y="80" text-anchor="middle">log only to serial</text>
<rect class="acc" x="320" y="20" width="200" height="80" rx="6"/><text class="t b" x="420" y="44" text-anchor="middle">splash process</text><text class="m" x="420" y="63" text-anchor="middle">fade-in, breathing,</text><text class="m" x="420" y="80" text-anchor="middle">ring by steps</text>
<rect class="ok" x="535" y="20" width="235" height="80" rx="6"/><text class="t b" x="652" y="44" text-anchor="middle">compd</text><text class="m" x="652" y="63" text-anchor="middle">same frame, ring completes,</text><text class="m" x="652" y="80" text-anchor="middle">mark shrinks and fades</text>
<path class="ln" d="M150 60 L163 60" marker-end="url(#st1)"/><path class="ln" d="M305 60 L318 60" marker-end="url(#st1)"/><path class="ln" d="M520 60 L533 60" marker-end="url(#st1)"/>
<text class="m" x="157" y="125">SPLASH</text><text class="m" x="300" y="125">CLAIM</text><text class="m" x="470" y="125">RELEASE without clearing,</text><text class="m" x="470" y="141">display ... splash=permille</text>
<line class="ln" x1="10" y1="170" x2="770" y2="170"/>
<text class="m" x="10" y="192">kernel</text><text class="m" x="200" y="192">devices</text><text class="m" x="360" y="192">files</text><text class="m" x="500" y="192">network</text><text class="m" x="640" y="192">desktop</text>
<text class="m" x="10" y="225">ring steps (5): init sends `krok n`; a step taking over 2 s shows "waiting for: ..." under the wordmark</text>
</svg>
<figcaption>Who draws the boot picture and when. The picture is never cleared: each next owner takes it over as it is.</figcaption>
</figure>

1. **Loader** **[implemented]** picks the GOP (as before) as the very
   first thing after taking control and draws black with the mark and an
   empty ring. Only then it loads the kernel and the initfs. From then on
   its output goes only to the serial line (UEFI Serial I/O protocol); the
   firmware text console would write over the picture. It sets `SPLASH`
   in `BootInfo::boot_flags`.
2. **Kernel** **[implemented]** with `SPLASH` does not create the text
   console over the framebuffer (it would clear the screen); the log goes
   to the serial line and the log buffer. Neither `FB_CONTROL RELEASE` nor
   the owner's exit clears the screen. It passes
   `bootstrap::flags::SPLASH` to init.
3. **The `splash` process** **[implemented]** is started by init from
   the initfs (handles: framebuffer memory object, control channel; QoS
   class Background, nobody waits for the picture). It takes the picture
   (`CLAIM`), fades in the background (900 ms), shows the wordmark, lets
   the glow breathe (2.4 s) and fills the ring by the steps init sends.
   Before the compositor's display (or the fallback console) starts, init
   sends `konec`: the process completes the ring (200 ms), replies
   `hotovo 1000` and exits; the kernel does not clear the picture when the
   process ends. If it does not reply within 1.5 s, init kills it. A
   separate process instead of an init thread: a drawing bug brings down
   only the process, not the critical init.

   | Message (init -> splash) | Meaning | Reply |
   |---|---|---|
   | `krok <n>` | `n` steps of `bootsplash::STEPS` done | - |
   | `konec` | complete the ring and exit | `hotovo <permille>` |
   | `chyba <text>` | boot failure screen, then exit | `ok` |
4. **Compositor** **[implemented]** gets the ring value the picture
   stopped at (`display <zone> splash=<permille>`); its first frame is the
   same picture, the ring completes (500 ms), holds for 200 ms, the mark
   shrinks to the avatar size (600 ms) and fades into the desktop together
   with the boot background (300 ms). The desktop has the same background
   (`gfx::backdrop`), so the hand-over is just the mark disappearing.

The glow breathing is computed from the time since system start, so `splash`
and the compositor continue without a jump. The desktop keeps being
composed during the hand-over (into a RAM buffer), so clients get frames
and application start-up is not delayed.

## Picture

Sizes and timings come from the prototype (logical px, scale = height /
800, 1280 x 800 = 1:1):

| Element | Value |
|---|---|
| mark | 132 px, logo 84 px, ring radius 62 px and stroke 3 px (track: accent 22 %) |
| glow | accent 42 % to 65 % of the ray of a 184 px square; breathing 0.25-0.7 and 0.92-1.05 |
| wordmark | 44 px high, 16 px under the mark, colour `ink` |
| "waiting for" | 8 px under the wordmark, style `small`, colour `ink_muted` |
| background | as the desktop: `bg` and two radial gradients `wall_a`, `wall_b` (light theme) |

Colours always come from the light theme: start-up happens before the
user's settings are loaded. The logo and wordmark are drawn directly from
the vector data `design/assets/logo/symbol-fialovy.svg` and
`design/assets/logo-wordmark.js` (even-odd rule,
`font::Rasterizer::finish_even_odd`).

## Debug boot

The on-screen text log remains for debugging **[implemented]**:

- key **D** in the loader menu (2 s, together with **L** for pulling the
  logs),
- the file `\os\bootlog` on the ESP (then every boot is a debug boot).

The loader then sets `VERBOSE` instead of `SPLASH` and the kernel draws
the log as before. The serial line always has the full output.

## Failures

| Situation | What is shown |
|---|---|
| kernel panic | the crash screen (`kernel/src/crashscreen.rs`, design `design/pad.html`) - always drawn **[implemented]** |
| init exits | the kernel restores the text console and shows the log with init's exit code **[implemented]** |
| neither GUI nor fallback console starts | the `splash` process (on init's `chyba`) replaces the mark with "Systém se nespustil" (the system did not start) with the reason and error code **[implemented]** |
| GUI fails, console starts | the picture is handed to the fallback console (as before) **[implemented]** |
| the `splash` process crashes | the picture stays (the kernel does not clear it), boot continues, the display takes it over **[implemented]** |

## Without GOP and other displays

| Case | What is shown |
|---|---|
| no framebuffer (headless, firmware without GOP) | the loader draws nothing and does not set `SPLASH`; kernel, init and compositor as before **[implemented]** |
| x86_64 `--gpu 2d` (virtio-gpu) | the picture is on the firmware card (VGA); the compositor on virtio-gpu starts with the same picture at its resolution (640 x 480, scale 0.75) and fades into the desktop **[verified in QEMU]** (manual run, `9e865be`) |
| native display (bochs-display, Intel) | the `splash` process exits, the native driver takes the framebuffer (`CLAIM`), the compositor continues as with fbdisplay **[verified in QEMU]** for bochs-display (default `xtask run`) |
| aarch64 in QEMU (ramfb) | picture on ramfb (800 x 600, scale 0.75), the `splash` process animates (~170 ms per frame under TCG), the compositor on bochs-display (1280 x 800) starts with the boot picture and fades into the desktop **[verified in QEMU]** (manual TCG run, `9e865be`) |

The test run (`TEST_RUN`) and the log-pulling mode do not start the
`splash` process (loader and kernel keep the picture, nobody animates it).

## Performance

Measured: release, `cargo xtask run --accel whpx --res 1920x1080`, QEMU
11.1 for Windows with WHPX. Times from the `os loader` line on the serial
line (host), average of 3 runs; they include the 2 s loader menu (same
before and after).

| Stage (ms from `os loader`) | before (`77ac8ee`) | after (`9e865be`) |
|---|---|---|
| first picture | loader text | boot picture after 33 ms (drawing 46-59 Mcycles) |
| kernel starts | 2511 | 2491 |
| init runs | 5489 | 5310 |
| compositor's first frame (`kompozitor bezi`) | 7401 | 7627 |
| first window (`klient 1`) | 8061 | 7961 |

Boot did not get slower: the first window is within 100 ms either way
(run-to-run spread is ~+/-250 ms). The compositor's first frame is ~230 ms
later: it computes the desktop background already on attach (its first
frame is the boot picture on that background, ~430 ms under WHPX);
before, it did so in the first tick, and init waits ~190 ms for `splash`
to complete the ring. Windows are not delayed. The `splash` process draws
a frame in 12-14 ms (breathing at 25 fps, class Background).

**Loader size** (release, x86_64): 86.5 KB before, 271.9 KB after (logo
and wordmark vector data, theme from `design/tokens.toml`, rasterizer).
Loading it from the ESP takes tenths of a ms longer.

Drawing itself (`cargo test --release -p bootsplash -- --ignored
perf_numbers`, host, 1920 x 1080): loader first frame 1.4 ms, full
fade-in frame 2.0 ms, fade into the desktop 3.2 ms, mark area
(244 x 244 px) 0.27 ms, background computation 9.2 ms (once).

⚠️ **Programs on x86_64 run without SSE** (target `x86_64-unknown-none`,
software floating point, ~30-40x slower). That is why the picture is
drawn with integers: the background per row (copy, dimming, blending)
and the resting mark from a table (`libs/bootsplash/src/cache.rs`:
distance, angle, ring and logo coverage per pixel). Floating point
remains only for the arc ends and for the moved mark during the
compositor's hand-over. A `splash` frame dropped from ~50 ms to 12-14 ms
(WHPX). The desktop background (~400 ms in floating point) is computed
once: in the `splash` process, and in the compositor the hand-over takes it
from the desktop (`Comp::backdrop`).

## Determinism

The loader draws with an FPU, the `splash` process and the compositor
with software floating point. The picture has to be bit-for-bit
identical, otherwise the hand-over between them would jump. The library
therefore uses only basic IEEE 754 operations (add, multiply, divide,
conversions; its own square root, sine and arctangent), which are
correctly rounded in hardware and software alike, and no `mul_add`. Two
tests guard it (below). The only known difference: the resting mark is
drawn from an integer table, the moved one (compositor hand-over) in
floating point; they differ by at most 2 colour levels (test
`integer_mark_matches_float`), invisible to the eye.

## Tests

- `cargo test -p bootsplash` **[implemented]**: layout, even-odd fill,
  first frame, clockwise ring, background equal to the desktop, RGB
  framebuffer, moved mark, integer vs floating-point path,
  `reference_output_is_stable` (frame hashes for 1280 x 800, 1920 x 1080
  and 1024 x 768) and `self_check_matches`.
- QEMU, suite `displej` **[verified in QEMU]**:
  - `startovni obraz: kresba bez FPU = reference hostitele` - the hash of
    `bootsplash::self_check()` in init (software floating point) equals
    the host value,
  - `bochs-display: compd navaze na startovni obraz, nastavi rezim, zmena
    rozliseni za behu` - the compositor with `splash=600`, replies during
    the hand-over and the mode change after it.
- Not verified automatically: the "waiting for" line and the failure
  screen (in QEMU no step stalls and boot does not fail).

## Follow-ups

- **Esc = detailed log** on the boot screen **[designed]**: init has no
  input until `inputd` starts.
- **Hand-over target = login avatar**: init adds the word `login` to
  `display` when no session is open **[implemented]**; the compositor then
  takes the target from `login_target` (`user/compd/src/splash.rs`), which
  after merging the accounts branch returns `Comp::greeter_avatar(false)`
  **[designed]**. Until then the mark shrinks in place.
- **Reduce motion**: the hand-over supports a 90 ms cross-fade only
  (`reduced`); there is no setting yet **[designed]**.
- **Resolution**: the loader does not change the GOP mode (firmware or
  EDID from `--res`); the picture only scales. Choosing a mode in the
  loader would cause a blink and hundreds of ms on real hardware.
