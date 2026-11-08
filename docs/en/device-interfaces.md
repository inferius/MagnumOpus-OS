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
   class, version, result. The protocol defines the behaviour on timeout,
   device detach and driver restart (pending requests get
   `SERVICE_RESTARTED` or are retried, depending on the kind).
4. **Capability and version negotiation**: the driver announces features,
   formats and limits. Extensions have their own id and version; an
   unsupported extension has a clear result (`NOT_SUPPORTED`), not silent
   ignoring. Today: only the display capability bits (`VSYNC`, `HOTPLUG`,
   `MULTI`).
5. **A data path without needless copies**: large data through shared
   buffers and handles. The interface defines buffer ownership,
   synchronization (fences, ordering) and when a buffer may be reused.
   Today: disks copy data in channel messages (a recorded deviation from
   the design).
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
| video memory (bochs-display, Intel DG2, `fbdisplay`) | compose in RAM, copy the damaged rectangles | one; the driver reports the buffer age, removing the copy of the previous frame |
| GPU compositing (`libs/vkcomp`) | the GPU draws straight into the scanout image | none |

The driver offers the compositor a list of its buffers with their memory
kind and the compositor picks the path. The display interface stays one
for CPU and GPU compositing.

## Layer 2: system services for applications

An application never gets a capability to a driver. It only gets a
channel to a service, according to the permission in its manifest and the
user's consent. The service:

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

**The cost**: applications have less freedom than on Android. So every
service needs versioned extensions (layer 1, item 4), otherwise developers
would route around it.

## Next step

A detailed protocol design for layer 1 (header, capabilities, buffers,
conformance tests) and converting block storage; layer 2 services
gradually as applications need them (first: file picker, notifications,
media playback).
