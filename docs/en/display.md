# Display configuration (native display drivers)

An extension on top of the firmware framebuffer (UEFI GOP, `fbdisplay`
driver): enumeration of connectors and monitors, modes from EDID, runtime
resolution changes, monitor hotplug and page flips at vsync. **No
acceleration** – the `compd` compositor still composes the image on the CPU.

| Part | What it does |
|---|---|
| `libs/display` | vendor-independent part: EDID, DisplayID, timings (CVT, DMT), DisplayPort (AUX, DPCD, link training, MST – groundwork only), HDMI (infoframes, TMDS, SCDC) |
| `guiproto::modes` + `guiproto::display` | protocol between driver and compositor (one for all drivers) |
| `libs/displaydrv` | native driver skeleton: protocol handling, double buffering with vsync, firmware state record |
| `user/bochs-display` | sample driver for QEMU (`bochs-display`, `secondary-vga`) |
| `libs/intelgfx` + `user/intel-display` | Intel Gen9 (Skylake) to Xe2 (Lunar Lake, Battlemage) |
| `user/devmgr` (`native.rs`), `user/init` (`display_native.rs`) | start-up, display selection, recovery after a crash |

## Overview

<figure class="diagram">
<svg viewBox="0 0 760 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Display configuration layers">
<defs><marker id="de1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="acc" x="270" y="10" width="220" height="56" rx="6"/><text class="t b" x="380" y="33" text-anchor="middle">compd (compositor)</text><text class="m" x="380" y="52" text-anchor="middle">frame in a VMO, mode / outputs commands</text>
<path class="ln" d="M380 66 L380 96" marker-end="url(#de1)"/>
<text class="m" x="392" y="86">guiproto::display + modes: CONNECTORS, SET_MODE, ATTACH, PRESENT, HOTPLUG, RESET</text>
<rect class="box" x="10" y="100" width="160" height="56" rx="6"/><text class="t b" x="90" y="123" text-anchor="middle">fbdisplay</text><text class="m" x="90" y="142" text-anchor="middle">GOP, single mode</text>
<rect class="box" x="190" y="100" width="160" height="56" rx="6"/><text class="t b" x="270" y="123" text-anchor="middle">virtio-gpu</text><text class="m" x="270" y="142" text-anchor="middle">mode from the host</text>
<rect class="ok" x="370" y="100" width="180" height="56" rx="6"/><text class="t b" x="460" y="123" text-anchor="middle">bochs-display</text><text class="m" x="460" y="142" text-anchor="middle">QEMU, flips, modes</text>
<rect class="ok" x="570" y="100" width="180" height="56" rx="6"/><text class="t b" x="660" y="123" text-anchor="middle">intel-display</text><text class="m" x="660" y="142" text-anchor="middle">Gen9 to Xe2</text>
<path class="ln" d="M460 156 L560 196" marker-end="url(#de1)"/>
<path class="ln" d="M660 156 L600 196" marker-end="url(#de1)"/>
<rect class="ok" x="440" y="200" width="310" height="50" rx="6"/><text class="t b" x="595" y="221" text-anchor="middle">libs/displaydrv</text><text class="m" x="595" y="239" text-anchor="middle">protocol, double buffer, vsync, firmware record</text>
<path class="ln" d="M595 250 L595 268" marker-end="url(#de1)"/>
<rect class="ok" x="440" y="272" width="310" height="50" rx="6"/><text class="t b" x="595" y="293" text-anchor="middle">libs/display</text><text class="m" x="595" y="311" text-anchor="middle">EDID, DisplayID, CVT/DMT, DP, HDMI</text>
<rect class="box" x="10" y="200" width="400" height="122" rx="6"/><text class="t b" x="210" y="223" text-anchor="middle">devmgr + init</text><text class="m" x="210" y="244" text-anchor="middle">devmgr starts the driver by PCI id, sends init a restart kit</text><text class="m" x="210" y="262" text-anchor="middle">init: picks virtio-gpu, then native, else fbdisplay;</text><text class="m" x="210" y="280" text-anchor="middle">holds the driver's channel end and the firmware record;</text><text class="m" x="210" y="298" text-anchor="middle">after a crash: a new instance restores the firmware image</text>
</svg>
<figcaption>The compositor talks to every driver with the same protocol; native drivers share a skeleton and the format library.</figcaption>
</figure>

## Display protocol

Extends the original messages (`INFO`, `ATTACH`, `PRESENT`, `SYNC`) with
display configuration. Numbers 6–15 / 106–115 and capability bits 2–7
belong to display configuration (5/105 and bit 1 are Venus).

| Message | Direction | Content |
|---|---|---|
| `CONNECTORS` (6) → `CONNECTORS` (106) | client → driver | connectors: output number, kind (DP, eDP, HDMI, DVI, VGA, virtual, firmware), connected, monitor name from EDID, size in mm, modes (w × h, refresh in mHz, pixel clock, `PREFERRED`, `CURRENT` flags) |
| `SET_MODE` (7) → `MODE_SET` (107) | client → driver | sets the output mode; refresh 0 = highest. The client then sends a new `ATTACH` with the new size. On error the previous mode stays |
| `HOTPLUG` (108) | driver → client | unsolicited event: a connector changed, the client sends `CONNECTORS` again |
| `RESET` (109) | driver → client | new driver instance after a crash: the previous `ATTACH` is gone, the client picks the output again |

| Capability | Bit | Meaning |
|---|---|---|
| `MODESET` | 2 | the driver supports more than one mode |
| `HOTPLUG` | 3 | the driver sends `HOTPLUG` |
| `VSYNC` | 4 | `PRESENT` is shown as a whole frame at vertical blank, `PRESENTED` carries the vsync time |
| `MULTI` | 5 | several outputs |

**Page flip.** The compositor still draws into its VMO (the shadow) and
sends damaged rectangles in `PRESENT`. The driver copies them into the back
buffer and flips it at vsync. The back buffer is one frame behind, so the
previous frame's damage is copied as well. After `ATTACH` and a mode
change both buffers are copied in full. The compositor does not need to
know how many buffers the driver has, and the interface does not overlap
with the layer list for the Vulkan backend (`libs/vkcomp`).

**The compositor** picks the main connected connector (the built-in panel
first) and its current mode. If no mode is set (monitor just plugged in)
it sets the preferred one. Control commands: `mode <w> <h> [mHz]`
(resolution change; windows move onto the screen, fullscreen and
maximized windows get the new size) and `outputs` (connectors as text).

```text
compd                              driver
  SYNC / INFO / CONNECTORS  ───►
                            ◄───   MODES, CONNECTORS (1920x1080 CURRENT, ...)
  mode 1024 768 (from the user)
  SET_MODE 0 1024 768 0     ───►   sets the mode, drops the shadow
                            ◄───   MODE_SET 1024x768 @ 60004 mHz
  ATTACH (new VMO)          ───►   copy into both buffers, flip
                            ◄───   ATTACHED
  PRESENT seq=1 [rects]     ───►   copy (rects + previous) into the back buffer, flip
                            ◄───   PRESENTED seq=1 vsync time
```

## libs/display

| Module | Content |
|---|---|
| `edid` | EDID 1.3/1.4 base block: vendor, product, size, input (bits per colour, interface), established, standard and detailed timings, name and serial, range limits, block-wise reading with a retry |
| `cta` | CTA-861 extension: VIC codes (+ timing table), audio, speakers, HDMI VSDB (physical address, deep colour, max TMDS), HDMI Forum VSDB and HF-SCDB (600 MHz, SCDC) |
| `displayid` | DisplayID 1.3 and 2.0: type I (10 kHz) and VII (kHz) timings, product name |
| `cvt`, `dmt` | CVT 1.2 (standard, RB v1, RB v2) in integers; the DMT table |
| `modes` | mode list from all sources: duplicates merged (detailed > VIC > DMT > CVT), source and monitor limits, ordering, picking |
| `dp` | DPCD, capabilities (incl. the eDP 1.4 rate table), link configuration and fallback, M/N; `auxch` (AUX encoding, DEFER, I2C over AUX for EDID), `train` (clock recovery + channel equalization, TPS1–4), `mst` (sideband messages with CRC – groundwork only) |
| `hdmi` | AVI, vendor (HDMI VIC for 4K), SPD and audio infoframes; TMDS clock by colour depth and format; scrambling decision, SCDC registers |

Tests (`cargo test -p display`, part of `cargo xtask test`) run on real
EDID dumps from the [linuxhw/EDID](https://github.com/linuxhw/EDID)
database (an AOC 2243W HDMI monitor, an LG LP133WH2 laptop panel) and on
blocks built in the tests (DisplayID, HF-VSDB). CVT values are compared
with the VESA calculator. DP link training and EDID over AUX are tested
against a simulated sink (DEFER, short I2C replies, a bad cable, falling
back to a lower rate).

## Display selection in init

1. virtio-gpu from the device manager (virtual machine),
2. a native driver (`display-native` from devmgr) that answers `SYNC` and
   reports a connected monitor with a mode; on x86 the card with the lower
   PCI address (the one the firmware uses) wins,
3. otherwise `fbdisplay` on the firmware framebuffer.

devmgr starts `bochs-display` for `1234:1111` (bochs-display, stdvga,
secondary-vga) and `intel-display` for Intel class 03. It sends init a
`display-native <program> <b> <d> <f>` message with the channel ends
(client, driver), the device resource, the PCI configuration, the
firmware record and the process. Intel also gets the stolen memory range
(BDSM/GGC), no DMA and no interrupts.

## Display safety

Goal: **never end up with a black screen.**

<figure class="diagram">
<svg viewBox="0 0 760 210" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Returning to the firmware image">
<defs><marker id="de2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="170" height="70" rx="6"/><text class="t b" x="95" y="45" text-anchor="middle">driver start</text><text class="m" x="95" y="64" text-anchor="middle">only reads registers,</text><text class="m" x="95" y="80" text-anchor="middle">saves the firmware record</text>
<path class="ln" d="M180 55 L218 55" marker-end="url(#de2)"/>
<rect class="ok" x="220" y="20" width="170" height="70" rx="6"/><text class="t b" x="305" y="45" text-anchor="middle">ATTACH / SET_MODE</text><text class="m" x="305" y="64" text-anchor="middle">first register write</text><text class="m" x="305" y="80" text-anchor="middle">only now</text>
<path class="ln" d="M390 55 L428 55" marker-end="url(#de2)"/>
<rect class="warn" x="430" y="20" width="150" height="70" rx="6"/><text class="t b" x="505" y="45" text-anchor="middle">driver crash</text><text class="m" x="505" y="64" text-anchor="middle">init's supervisor</text><text class="m" x="505" y="80" text-anchor="middle">starts restore</text>
<path class="ln" d="M580 55 L618 55" marker-end="url(#de2)"/>
<rect class="acc" x="620" y="20" width="130" height="70" rx="6"/><text class="t b" x="685" y="45" text-anchor="middle">firmware image</text><text class="m" x="685" y="64" text-anchor="middle">RESET → compd</text><text class="m" x="685" y="80" text-anchor="middle">re-attaches</text>
<rect class="box" x="10" y="120" width="380" height="70" rx="6"/><text class="t b" x="200" y="145" text-anchor="middle">error before takeover</text><text class="m" x="200" y="164" text-anchor="middle">unknown generation, tiled framebuffer, no</text><text class="m" x="200" y="180" text-anchor="middle">output: the driver exits → init uses fbdisplay</text>
<rect class="box" x="430" y="120" width="320" height="70" rx="6"/><text class="t b" x="590" y="145" text-anchor="middle">SET_MODE error</text><text class="m" x="590" y="164" text-anchor="middle">the driver restores the previous setup,</text><text class="m" x="590" y="180" text-anchor="middle">compd keeps its frame</text>
</svg>
<figcaption>The driver changes display registers only after saving the firmware record; after a crash a new instance writes it back.</figcaption>
</figure>

- **Firmware record** (`displaydrv::snapshot`): a list of (register, value)
  for everything the driver changes, in restore order (surface address
  last so the change lands as a whole frame). It is written into a memory
  object held by init – it survives a driver crash. The magic is written
  last: a crash in the middle leaves the record invalid and the new
  instance overwrites nothing.
- **The driver touches no register** until the record is saved (the
  backend returns `BAD_STATE`) and until the compositor attaches a frame.
- **Crash**: init holds the driver's channel end (`Kept`), so the
  compositor keeps its channel. The supervisor starts a new instance with
  the `restore` flag: it writes the record back, sends `RESET` and then
  serves **only the firmware mode** (no `MODESET`, one buffer). The
  compositor re-attaches.
- **Clean exit** (shutdown) restores the firmware image as well.

Not covered (risks):

- A **hung** driver (endless loop without a crash) is not detected – there
  is no liveness watchdog.
- If the restore instance crashes too, the supervisor gives up after 5
  crashes per minute; the image stays as the last restore left it.
- The rescue console (`user/console`) draws into the firmware framebuffer;
  while a native driver runs on the same card at another resolution its
  image is offset until the firmware mode is back.

## bochs-display (QEMU)

DISPI (Bochs VBE) registers in MMIO BAR2 from 0x500, the image in BAR0,
EDID from offset 0 of BAR2 (generated by QEMU: "QEMU Monitor", 13 modes).
`secondary-vga` and `VGA` on x86 have the same registers, so `xtask` did
not need any new device (aarch64 already had `bochs-display`).

- **Resolution change**: `ENABLE` = 0, `BPP` 32, `XRES`, `YRES`,
  `VIRT_WIDTH`, `VIRT_HEIGHT` = 2 × height, `ENABLE` = on + LFB. Reading
  back checks that the device accepted the mode (otherwise back to the
  previous one).
- **Page flip**: two buffers in VRAM, switched with `Y_OFFSET`. The device
  has no vsync interrupt – vertical blanks are simulated by a clock with a
  period from the refresh rate (`VblankClock`); `PRESENTED` follows it.
- `-device bochs-display` lacks `GETCAPS` (maximum resolution); then only
  the VRAM size limits the modes.

Test (`cargo xtask test --only displej`, x86_64 and aarch64): a separate
compositor instance on the last `bochs-display` (on x86 the second card;
the first one carries the kernel log) – modes from EDID, the compositor
setting a mode, `mode 800 600`, `mode 1024 768 60000`, rejecting an
invalid mode, the presented frame count growing, then killing the driver,
a new instance with `restore`, `RESET` and checking that the firmware
mode applies (x86: OVMF enables the card at 1280x800; aarch64: output off).

## Intel (intel-display)

### Scope: display versions 9 to 20

| Display | Platforms | State |
|---|---|---|
| 9 | Skylake, Kaby Lake, Coffee/Whiskey/Comet Lake | takeover, flips, scaler, EDID (AUX, GMBUS), hotplug, DP link retraining (same rate) |
| 11 | Ice Lake, Elkhart/Jasper Lake | takeover, flips, scaler, EDID, hotplug |
| 12, 13 | Tiger Lake, Rocket Lake, Alder Lake, Raptor Lake, DG2 | takeover, flips (integrated), scaler, EDID, hotplug; DG2 single buffer (framebuffer in card memory) |
| 14, 20 | Meteor/Arrow Lake, Lunar Lake, Battlemage | takeover and scaler without EDID (AUX moved – XELPDP), single buffer |
| 30 | Panther Lake | recognised only, fbdisplay stays |

Why from Gen9: Skylake introduced universal planes, DDI with a uniform
`TRANS_DDI_FUNC_CTL`, pipe scalers and the gen8 GGTT format – these
registers change only in details up to Xe2. Older generations (Broadwell
and below) have different planes and ports and are rare today. Gen9 is
also the most widespread (2015–2020 machines).

### What the driver does

1. Recognises the generation by PCI ID (`intelgfx::platform`, lists from
   `include/drm/intel/pciids.h`).
2. Reads the GOP state: the enabled transcoder (EDP, A–D), timings, the
   DDI port (`TRANS_DDI_FUNC_CTL`), plane 1 (`PLANE_CTL`, `STRIDE`,
   `SURF`). Takes over only linear XRGB8888 (what GOP sets).
3. EDID: DP and eDP over AUX (`DDI_AUX_CTL`), HDMI and DVI over GMBUS
   (pins by PCH). Measures the refresh rate with the pipe frame counter,
   otherwise takes it from EDID.
4. Finds the GOP framebuffer through the GGTT entries (stolen memory or
   card memory) and maps it. The second flip buffer goes into stolen
   memory right after it, with its own GGTT entries at a free address
   (0xE0000000).
5. **Flip** = writing `PLANE_SURF` (latched at vsync); completion =
   `PLANE_SURFLIVE` has the new address and the frame counter moved.
6. **Resolution change** = smaller modes from EDID through the pipe scaler
   (`PIPESRC`, `PS_CTRL`, `PS_WIN_POS`, `PS_WIN_SZ`; aspect-preserving
   window). Monitor timings, PLL and link stay as the firmware set them.
7. Hotplug: periodic reads of `SDEISR`, `GEN8_DE_PORT_ISR`,
   `GEN11_DE_HPD_ISR` (HPD level) – a change = a `HOTPLUG` event.

### What the driver deliberately does not do

- **Another refresh rate or a higher resolution than the firmware's**:
  that needs PLL programming (DPLL / combo PHY / Type-C PHY by
  generation), re-enabling the DDI and training the link. The libraries
  prepare it (`libs/display::dp`: link choice, M/N, training;
  `intelgfx::phy` for Gen9), but the full i915 enable sequences are not
  translated.
- A second monitor (only the output the firmware enabled), MST, DSC, HDR, PSR.
- Interrupts (vsync is polled every 1 ms while a flip is pending).

### What is not verified on hardware

**None of the Intel part has run on a real machine yet** – QEMU does not
emulate Intel graphics. Host tests (`cargo test -p intelgfx`) run on
simulated registers (`intelgfx::sim`): "eDP on the EDP transcoder" and
"HDMI on pipe B" scenarios with real EDID dumps and recorded write
sequences (flip, scaler, record restore, Gen9 link training). The
simulation is only as faithful as i915 and the PRM describe the hardware.
Uncertain points:

- the EDP transcoder feeds pipe A (assumed from how GOP sets it up),
- AUX channel and GMBUS pin mapping for Type-C ports on ADL+ and discrete cards,
- changing `PIPESRC` at run time without disabling the pipe (i915 does it as a "fastset"),
- writing GGTT entries in the 0xE0000000 area (checked to be free),
- the stolen memory size from the GGC register (Gen9+ encoding).

### Testing on a real machine

1. `cargo xtask image` → `target/os-x86_64-debug.img`; write it to a USB
   stick (Linux `dd if=... of=/dev/sdX bs=4M`, Windows Rufus in DD mode).
   Disable Secure Boot.
2. **Serial line** (a board COM port or a USB-serial adapter on the port
   the kernel uses): 115200 8N1, record the whole log. Lines to look for:
   `devmgr: pci ... 8086:xxxx trida 03`, `intel-display ...: <platform>,
   vystup ..., <w>x<h> @ ... mHz, monitor ...`, `init: displej je
   nativni ovladac intel-display`, `compd: displej`.
3. Without a serial port: the kernel cannot log to disk yet (planned in
   the architecture) – photograph the screen; the rescue console after a
   GUI crash shows the last log lines.
4. Unplug and replug the monitor (HDMI/DP) and watch the log for
   `HOTPLUG`. Resolution changes are only available on the compositor's
   control channel so far (the `mode` command from init and tests); a
   user-facing display setting is the next step.
5. If the screen goes black: wait 5 s (driver restart in restore mode),
   or restart the machine – the firmware record is saved again at boot.

## Origin and licence

The code is written anew in Rust. From Linux we took procedures and facts
(register values, step order) only from MIT-licensed files (checked in
each file's header); no GPL code was used. The list is in `THIRD-PARTY.md`
at the repository root. EDID dumps in tests: linuxhw/EDID, CC BY 4.0.

## Tests

| Level | Command | What |
|---|---|---|
| host | `cargo test -p display` | EDID, CTA, DisplayID, CVT/DMT, AUX, link training, MST CRC, HDMI (35 tests) |
| host | `cargo test -p displaydrv` | double buffering, vsync, mode change, events, firmware record |
| host | `cargo test -p intelgfx` | platforms, GOP state, AUX, GMBUS, GGTT, flips, scaler, restore, hotplug, Gen9 training |
| host | `cargo test -p guiproto`, `-p comp` | protocol messages, `Comp::resize` |
| QEMU | `cargo xtask test --only displej` (x86_64, aarch64) | bochs-display + compd: modes, resolution change, crash and restore |
