# Unified device and service interfaces

**Status: [designed]** (2026-11-08). Nothing on this page is implemented
yet unless explicitly marked. It builds on the existing
[driverkit](drivers.md) and the device protocols.

Goal: drivers are separated from the rest of the system by a uniform
interface per device class, and applications never reach hardware
directly, only through system services with a standard API. The user gets
the same behaviour regardless of the hardware vendor and the application
(a call is a call wherever it comes from; the camera always has the same
permission and indicator).

## Three layers

<figure class="diagram">
<svg viewBox="0 0 780 330" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Three interface layers">
<defs><marker id="di1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="10" width="760" height="60" rx="6"/>
<text class="t b" x="390" y="34" text-anchor="middle">applications</text>
<text class="m" x="390" y="54" text-anchor="middle">only service APIs + capabilities from the manifest; no access to drivers</text>
<rect class="acc" x="10" y="110" width="760" height="70" rx="6"/>
<text class="t b" x="390" y="134" text-anchor="middle">system services (layer 2)</text>
<text class="m" x="390" y="154" text-anchor="middle">camera · microphone · calls · location · playback · notifications · file picker · screen sharing</text>
<text class="m" x="390" y="170" text-anchor="middle">permission, indicator, concurrency, providers (apps register as a source of calls, media...)</text>
<rect class="ok" x="10" y="220" width="760" height="50" rx="6"/>
<text class="t b" x="390" y="242" text-anchor="middle">device class interfaces (layer 1)</text>
<text class="m" x="390" y="260" text-anchor="middle">display · block storage · network · audio · input · camera · ...: the same for every vendor</text>
<rect class="box" x="10" y="290" width="240" height="34" rx="6"/><text class="m" x="130" y="312" text-anchor="middle">our Rust drivers</text>
<rect class="box" x="270" y="290" width="240" height="34" rx="6"/><text class="m" x="390" y="312" text-anchor="middle">adopted C (LinuxKPI, isolated)</text>
<rect class="box" x="530" y="290" width="240" height="34" rx="6"/><text class="m" x="650" y="312" text-anchor="middle">virtual (QEMU, VMware)</text>
<line class="ln" x1="390" y1="70" x2="390" y2="106" marker-end="url(#di1)"/>
<line class="ln" x1="390" y1="180" x2="390" y2="216" marker-end="url(#di1)"/>
</svg>
<figcaption>The application talks to a service, the service to a class interface, the class to a driver.</figcaption>
</figure>

## Layer 1: a unified driver model

Builds on `driverkit`; no new framework, the existing one is extended.

1. **Common life cycle**: discovery, start, client attach, stop, detach,
   reset, crash recovery. The framework always releases resources (DMA,
   interrupts, mappings), even after an error. Today:
   `probe -> start -> events -> stop` and restart through devmgr
   **[implemented]**; only `virtio-rng`, `virtio-input` and `edu` are
   converted to driverkit.
2. **Per-class interfaces**: display, block storage, network, audio, input
   (later camera, sensors, printer). Services talk to the class, not the
   vendor. Today: the protocols `blockproto`, `netproto`, `audioproto`,
   `inputproto` and the display protocol exist **[implemented]**, but each
   is different.
3. **Asynchronous requests and events** with a common header: request id,
   **connection generation** (incremented with every driver restart),
   class, version, result. The result distinguishes **completed**, **not
   performed** and **outcome unknown** (the driver may have performed the
   operation and crashed before acknowledging). Every operation in the
   class protocol states whether it is **safely retryable** (idempotent,
   e.g. reading a block) and whether **its outcome can be looked up**
   (e.g. the state of a write by ID). After a restart (new generation) the
   client retries only retryable operations, looks up the others, and what
   cannot be looked up is returned to the caller as "outcome unknown" -
   never a silent retry risking a double effect.
4. **Capability and version negotiation**: the driver announces features,
   formats and limits. Extensions have their own id and version; an
   unsupported extension has a clear result (`NOT_SUPPORTED`), not silent
   ignoring. Today: only the display capability bits (`VSYNC`, `HOTPLUG`,
   `MULTI`).
5. **A data path without needless copies**: large data through shared
   buffers and handles. The interface defines **the whole buffer life
   cycle**: who allocates it (and from which memory), who may write to it
   and when, how completion of writing is signalled (a fence), when the
   device (display, disk) may take it over, when it is safe to reuse
   (release fence), and its format, memory layout (stride, alignment,
   tiling) and compatibility between the rendering and the displaying
   device. Today: disks copy data in channel messages (a recorded
   deviation from the design).
6. **Attach by manifest**: an ordinary driver = code + manifest + tests,
   without kernel changes or special branches in services. Today: the
   manifest **[implemented]**, but devmgr has a built-in driver table and
   init a special display choice.
7. **Common conformance tests** per class (protocol, life cycle, error
   states, resource limits, recovery), runnable in the hardware simulation
   on the host. The driver author only adds tests for their hardware.

Adopted large drivers (AMD/NVIDIA graphics, Wi-Fi) must provide the same
class interface as ours; services cannot tell the difference.

**Conversion order**: block storage (biggest gain: no more copying in
messages), display, network, audio, input. One class at a time, the old
protocol through an adapter until all drivers are converted.

## Display without a needless copy

Today **[implemented]**: the compositor composes into its own memory (a
shadow) and on `PRESENT` the driver copies the damaged rectangles into the
back buffer, plus the previous frame's rectangles (the back buffer is one
frame behind).

Removing the copy entirely is not possible everywhere: software
compositing **reads** destination pixels (transparency, shadows) and
reading video memory through a BAR (write-combining or uncached) is one to
two orders of magnitude slower than RAM. So by memory kind:

| Where the image lives | Path | Copies |
|---|---|---|
| system RAM the card can scan out (Intel integrated through the GGTT, virtio-gpu `SET_SCANOUT_BLOB`) | the compositor draws directly into the driver's buffer, `PRESENT` = flip | none |
| video memory (bochs-display, Intel DG2, `fbdisplay`) | compose in RAM, copy the damaged rectangles | one copy; see buffer age below |
| GPU compositing (`libs/vkcomp`) | the GPU draws into a scanout-capable buffer the display is not currently reading; it is flipped after the completion fence | none |

**Buffer age** alone does not remove the need to repair older content:
when alternating two buffers, the back buffer just acquired holds an
image two frames old. **All regions changed since its last use** must be
brought into it (the union of the damage of the last N frames, N being
the age) - by redrawing or copying. Example: frame 1 changes the left
part, frame 2 the right; the buffer for frame 2 must get both left and
right, otherwise the left reverts to the old state. The age only says
how many frames of damage to unite (unknown age = the whole buffer); the
saving is that nothing unchanged is copied.

The driver offers the compositor a list of its buffers with their memory
kind and the compositor picks the path. The display interface stays one
for CPU and GPU compositing.

### Video plane (designed)

**[designed]** Video should not be composited as an ordinary layer: a
decoded 4K frame would be copied and composited by the CPU every frame.
The compositor therefore supports a **video plane**: the buffer from the
hardware decoder goes straight to a display plane (overlay) if the
display can show it, otherwise to GPU compositing. Without either path
the video is scaled down to a resolution the CPU can keep up with, and
the user sees that in the playback information.

| What | Rule |
|---|---|
| source | decoder buffer in YUV (NV12, P010), no RGB conversion on the CPU |
| HDR | metadata (PQ, HLG, luminance) travels with the buffer; the compositor switches the output to HDR only while the video is in front and the display supports it |
| refresh rate | the display may switch to a multiple of the video frame rate (24, 25, 50 Hz), only in full screen |
| subtitles and controls | drawn as an ordinary layer above the video plane |
| content protection | a secure path (DRM) is not designed: a new system will not get certification |

Related to the [TV design](appearance.md#tablet-and-tv-design), but every
video playback on desktop and phone needs it.

## Layer 2: system services for applications

An application never gets a capability to a driver (registers,
unrestricted DMA). It gets a channel to a service according to the
permission in its manifest and the user's consent. **The service must not
become a bottleneck**: it checks the permission and then hands the app a
**restricted session** - shared buffers and a command queue with fixed
limits (format, size, rate) - and the data path does not copy through the
service (graphics, video, audio). The service keeps only control (open,
parameter changes, revoking permission = invalidating the session). The
service:

- **decides on permission** (once, always, only now) and shows an
  **indicator** (camera on, microphone active, location),
- **handles concurrency** (who may play to the speaker, a call ducks
  music),
- **provides a uniform UI** (call window, playback controls in the bar,
  file picker),
- **accepts providers**: an application registers as a source (of calls,
  media, location, a printer) and the system treats it like a built-in
  one.

| Service | What the application gets | Providers |
|---|---|---|
| camera | a frame stream in shared buffers, camera choice, resolution | camera drivers (UVC), an app's virtual camera |
| microphone and audio | a sample stream through `audiod`, ducking by priority | audio drivers |
| calls | the system call window, history, audio precedence, decline | phone, VoIP apps (like CallKit) |
| location | location at the permitted accuracy (approximate / precise) | GPS, Wi-Fi, manual |
| media playback | controls in the bar and media keys, track info | players |
| notifications | system notification, snooze, do not disturb | every app |
| file picker (portal) | only the files the user picked, not the whole disk | FS, cloud storage |
| screen sharing | only the window or screen the user picked, with an indicator | the compositor |
| clipboard, contacts, printing | data only on a user action (paste, pick a contact) | services and apps |

### Media actions and playback session (designed)

**[designed]** The media playback service keeps a **playback session**
for each player: what is playing (title, artwork, duration), position,
state and the list of actions the player supports. Bar controls, media
keys, headphones, the lock screen, a remote control, a companion phone
and voice are all just clients of the same session.

Control goes through **semantic actions**, not keys or clicks:

| Action | Parameters |
|---|---|
| `media.play`, `media.pause`, `media.toggle`, `media.stop` | - |
| `media.seek` | position or offset in ms |
| `media.next`, `media.previous` | - |
| `media.subtitles` | language or off |
| `media.audio_track` | track |
| `media.info` | - (shows details) |

The system handles volume and power; the app does not receive them. A
player lists the actions it supports in its manifest; the system does
not send unknown actions. With several players, an action goes to the
session that started playing last (the same rule as audio contention
above).

**The cost**: applications have less freedom than on Android. So every
service needs versioned extensions (layer 1, item 4), otherwise developers
would route around it.

### Window session for children: risk

**[verified in QEMU]** x86_64. An app may hand its children a window
session at the compositor (the `Gui` role,
[GUI](gui.md#window-session-for-child-processes-the-gui-role)). The app
still gets no capability to the display or input driver: only a session to
the `compd` service with its own identity.

- ⚠️ **What a child can do with it:** open windows, read input in its own
  windows and use the clipboard **under the parent's identity** (the trust
  badge shows the parent app). The same risk as a process inheriting
  `DISPLAY` or `WAYLAND_DISPLAY` on Linux.
- **What it cannot do:** get a wider identity or rights (the narrowing is
  only empty, the supervisor checks the recipe), get around the limits on
  windows and buffers (they are counted per identity) or survive revocation
  (the whole tree of leases is cancelled and the compositor closes all
  connections of the identity).
- **A child without GUI:** a child environment without `MO_GUI`.

### Sound session for children: risk

**[verified in QEMU]** x86_64. Like a window, an application may pass its
children a sound session at `audiod` (role `Audio`,
[Audio](audio.md#sound-session-for-children-role-audio)); it gets no
capability to the sound card driver.

- ⚠️ **What a child can do with it:** play **under the parent's
  identity**. Record only if the parent recipe allows it (`Capture::Ask`
  from the manifest), and always only after the user's decision in the
  microphone permission dialog; the bar shows an indicator while recording.
- **What it cannot do:** widen the recipe (the narrowing is empty only),
  get around the session and stream limits (per identity) or survive
  revocation (`audio::revoke`).
- **A child without sound:** a child environment without `MO_AUDIO`.

## Next step

A detailed protocol design for layer 1 (header, capabilities, buffers,
conformance tests) and converting block storage; layer 2 services
gradually as applications need them (first: file picker, notifications,
media playback).
