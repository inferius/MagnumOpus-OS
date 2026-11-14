# NVIDIA and AMD graphics

**Status: [proposed]** (2026-11-14). Nothing on this page is implemented.
It is research and a proposal: which reused path to take, what it gives
us, and **what should change in the system foundations now** so the
driver does not have to be bolted on later (section
[Impact on the foundations](#impact-on-the-foundations)).

Order: **NVIDIA first** (NVIDIA cards are available for testing): **our
own Rust driver for GSP** (decided 2026-11-14). AMD and Intel 3D later
through an isolated LinuxKPI layer ([strategy](#strategy-for-amd-and-intel-3d)).
Intel display is handled by our own driver ([Display configuration](display.md)).

## Summary

| Question | Decision / recommendation |
|---|---|
| NVIDIA driver | **decided**: our own Rust driver for GSP only, fixed firmware version 570.144, RPC definitions generated from nouveau's headers ([comparison](#comparison-nouveau-in-c-or-our-own-rust-driver-for-gsp)) |
| NVIDIA driver licence | **decided in the design**: GPL-2.0-only in the `gpl/` part – GSP boot, VBIOS, Falcon/FWSEC and queues taken from nova (Rust, GPL), display, channels, MMU and the NVK uAPI translated from nouveau (MIT) ([driver licence](#driver-licence-mit-or-gpl)) |
| NVIDIA generations | **Turing to Blackwell** (GSP: full clocks, Vulkan). Pascal and older firmware framebuffer for now; porting nouveau's non-GSP path only on request |
| NVIDIA Vulkan | **NVK** from Mesa (MIT) with unchanged logic; the render node imitates the nouveau uAPI |
| AMD driver | **amdgpu** + display core (MIT) over LinuxKPI, second step ([strategy](#strategy-for-amd-and-intel-3d)) |
| Compatibility layer | our own "LinuxKPI" in Rust on driverkit, only for AMD and Intel 3D, isolated inside the driver process; model: FreeBSD drm-kmod (BSD) |
| Without an IOMMU | **decided**: start with a warning (like disks and network), DMA quarantine after a crash, advice to enable VT-d / AMD-Vi |
| Foundations | **decided**: points 1, 2, 4 and 8 (fence, shared buffer, scatter-gather DMA, render class) as kernel work after dynamic linking and before FFmpeg ([architecture](architecture.md#adopting-existing-software-order)) |
| Firmware | **decided**: bundle it into the image; the NVIDIA and AMD licences allow it (conditions below) |

## Architecture

<figure class="diagram">
<svg viewBox="0 0 780 420" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="GPU driver architecture">
<defs><marker id="ge1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="10" width="230" height="74" rx="6"/><text class="t b" x="125" y="34" text-anchor="middle">application (Vulkan)</text><text class="m" x="125" y="53" text-anchor="middle">Mesa NVK in the app process,</text><text class="m" x="125" y="69" text-anchor="middle">winsys: nouveau uAPI = messages</text>
<rect class="acc" x="275" y="10" width="230" height="74" rx="6"/><text class="t b" x="390" y="34" text-anchor="middle">render service (gpud)</text><text class="m" x="390" y="53" text-anchor="middle">permissions, contexts, quotas,</text><text class="m" x="390" y="69" text-anchor="middle">hands out render nodes</text>
<rect class="acc" x="540" y="10" width="230" height="74" rx="6"/><text class="t b" x="655" y="34" text-anchor="middle">compd</text><text class="m" x="655" y="53" text-anchor="middle">display class</text><text class="m" x="655" y="69" text-anchor="middle">(protocol as intel-display)</text>
<path class="ln" d="M240 47 L273 47" marker-end="url(#ge1)"/>
<path class="ln dash" d="M125 84 L125 150 L218 150" marker-end="url(#ge1)"/><text class="m" x="132" y="120">render node (channel)</text>
<path class="ln" d="M390 84 L390 128" marker-end="url(#ge1)"/>
<path class="ln" d="M655 84 L655 128" marker-end="url(#ge1)"/>
<rect class="ok" x="220" y="130" width="550" height="200" rx="6"/><text class="t b" x="495" y="152" text-anchor="middle">gpu-nvidia process (Rust, isolated, restricted Resource, IOMMU)</text>
<rect class="box" x="235" y="165" width="250" height="70" rx="4"/><text class="t" x="360" y="187" text-anchor="middle">GSP core (from nouveau)</text><text class="m" x="360" y="205" text-anchor="middle">GSP boot, RPC r570, RM objects,</text><text class="m" x="360" y="221" text-anchor="middle">MMU, channels, display via RM</text>
<rect class="box" x="505" y="165" width="250" height="70" rx="4"/><text class="t" x="630" y="187" text-anchor="middle">device classes</text><text class="m" x="630" y="205" text-anchor="middle">display (like intel-display),</text><text class="m" x="630" y="221" text-anchor="middle">render (nouveau uAPI, VM_BIND)</text>
<rect class="warn" x="235" y="250" width="520" height="66" rx="4"/><text class="t" x="495" y="272" text-anchor="middle">driverkit: BAR (WC), MSI-X, scatter-gather DMA, IOMMU domain,</text><text class="m" x="495" y="290" text-anchor="middle">kernel fences and shared buffers, firmware service</text><text class="m" x="495" y="306" text-anchor="middle">(LinuxKPI only for AMD and Intel 3D, inside their process)</text>
<path class="ln" d="M495 330 L495 358" marker-end="url(#ge1)"/>
<rect class="box" x="10" y="360" width="760" height="50" rx="6"/><text class="t b" x="390" y="382" text-anchor="middle">kernel</text><text class="m" x="390" y="400" text-anchor="middle">Resource, BAR (WC), MSI-X, scatter-gather DMA, IOMMU domain, fences, buffers as VMO + bufdesc, firmware from FS</text>
<rect class="box" x="10" y="190" width="190" height="120" rx="6"/><text class="t b" x="105" y="212" text-anchor="middle">firmware</text><text class="m" x="105" y="232" text-anchor="middle">GSP-RM 570.144,</text><text class="m" x="105" y="248" text-anchor="middle">booter, FWSEC (VBIOS),</text><text class="m" x="105" y="264" text-anchor="middle">AMD: PSP, SMU,</text><text class="m" x="105" y="280" text-anchor="middle">DMCUB, VCN</text><text class="m" x="105" y="296" text-anchor="middle">firmware service</text>
<path class="ln" d="M200 250 L233 250" marker-end="url(#ge1)"/>
</svg>
<figcaption>An application never gets a capability to the driver: the render service hands it a render node, the image goes through the display class. The NVIDIA driver is Rust on driverkit; the GSP firmware does most of the work.</figcaption>
</figure>

- **One process per GPU**: `gpu-nvidia` in Rust on driverkit (GSP core
  translated from nouveau). Later `gpu-amd` with reused C on LinuxKPI in a
  process of its own – only our interfaces leave any of them.
- **Display class**: the same protocol as `intel-display` and
  `bochs-display` (connectors, modes, vsync flips, hotplug). On Turing+ the
  driver translates it into GSP-RM control calls and the display channel.
  The compositor cannot tell which driver is underneath.
- **Render class** (new): Vulkan in an application (Mesa NVK/RADV) needs
  DRM ioctls (buffer allocation, `VM_BIND`, `EXEC`, fences). They travel
  as messages over a "render node" channel. The channel is handed out by
  the **render service** (manifest permission, a context per application,
  GPU memory quotas), never by the application itself
  ([Device interfaces](device-interfaces.md)).
- **Crash**: after a mode set the GPU cannot return to the firmware
  framebuffer (GOP no longer scans out). Recovery = PCI function reset
  (FLR or bus reset) and a new instance; the compositor's frame is not
  shown meanwhile. See [Risks](#risks).

## NVIDIA: which path covers which generations

Verified in the Linux and Mesa sources (2026-11-14):

| Path | Licence | Generations | Display (KMS) | Acceleration | For us |
|---|---|---|---|---|---|
| **nouveau** (Linux) | MIT text in headers (`nouveau_drm.c`, `nvkm/...`, `gsp/...`), module "GPL and additional rights" | NV04 to Ada, chip tables also for Blackwell (GB100/GB102, GB202 to GB207) | yes, all | Kepler+; from Turing through GSP-RM, default (`NvGspRm` = true) | **translation source** (GSP parts) |
| **NVK** (Mesa) | MIT (Mesa core) | Kepler to Ada, consumer Blackwell (RTX 5000) | – | conformant Vulkan 1.4; needs nouveau from Linux 6.6+ (`VM_BIND`/`EXEC` uAPI) | **Vulkan** |
| **open-gpu-kernel-modules** (NVIDIA) | files MIT, MIT/GPLv2 as a module | Turing+ (only with GSP firmware of the same release) | `nvidia-modeset` | only with NVIDIA's closed userspace | facts only, maybe display only |
| **nova** (Linux, Rust) | GPL-2.0 | GSP: Turing to Blackwell (TU102 to GB207 enum) | no (nova-drm has only `INFO`, `GEM_CREATE`, `GEM_INFO`) | GSP boot, command queue, VBIOS, Falcon/FWSEC, booter, MMU partly; no channels | **reuse source** for GSP boot and queues (the driver is then GPL in `gpl/`) |

Therefore:

1. **Turing (RTX 20, GTX 16) to Blackwell (RTX 50)**: our driver
   (translation of nouveau's GSP parts) + GSP-RM firmware 570.144. GSP
   itself manages clocks and power, so the card runs at full speed. Vulkan
   through NVK. **Chosen target.**
   **Blackwell** (GB202 to GB207, RTX 50) is GSP-only in nouveau: even
   display goes through GSP-RM (`disp/gb202.c`: "GB20x is GSP-only").
   Without GSP firmware there is not even an image.
2. **Maxwell 2 (GTX 900), Pascal (GTX 10), Volta**: display yes,
   acceleration yes through NVIDIA-signed firmware loaded by ACR secure
   boot (`nvidia/gp108/acr`, `.../gr/*_sig.bin` in linux-firmware); NVK
   is enabled for these generations since April 2025. Clocks cannot be
   raised, though (NVIDIA has not released power-management firmware),
   so the card stays at boot clocks. Fine for the desktop, not for games.
   **For us the firmware framebuffer for now**; porting this path from
   nouveau only on request.
3. **Kepler (GTX 600/700), Maxwell 1 (GTX 750)**: display, acceleration,
   manual reclocking, NVK Vulkan. For us the firmware framebuffer.
4. **Fermi and older** (e.g. GTX 560 Ti, GF114/GF110): nouveau has display
   and 3D, but power management is unfinished ("it's slow") and NVK does
   not support Fermi. Proposal: not supported, keep the firmware
   framebuffer (`fbdisplay`).

`open-gpu-kernel-modules` supports new features better, but without
NVIDIA's closed userspace it has no Vulkan or OpenGL. Its display part
(`nvidia-modeset`) is MIT and could serve as a fallback for Turing+ if
nouveau fails on a particular card – at the cost of a second huge code
base. Not recommended.

### Test cards and what to verify on them

The user's cards (as of 2026-11-14):

| Card | Chip / generation | Path | What to test on it |
|---|---|---|---|
| **RTX 4090** (active, from USB) | AD102, Ada | our GSP driver, NVK | **main card**: stage (a) display (takeover, modes, flips, hotplug, multiple monitors), (b) copy, (c) Vulkan; recovery after a crash (GSP reset) |
| **RTX 5070 Ti** | GB203, Blackwell | our GSP driver (display through GSP too), NVK | second Turing+ card: verify it runs purely through GSP including display; newest GSP firmware; NVK Vulkan |
| RTX 2080 Ti (maybe) | TU102, Turing | our GSP driver, NVK | oldest GSP generation: firmware version compatibility, otherwise the same tests as the 4090 |
| **GT 1030** (active, from USB) | GP108, Pascal | firmware framebuffer (`fbdisplay`) | the driver rejects the non-GSP card and the display stays on `fbdisplay`; coexistence with a Turing+ card in one machine (two GPUs) |
| GTX 560 Ti | GF114/GF110, Fermi | not supported | only check that the driver rejects the card and the display stays on `fbdisplay` (safe fallback) |

Order: the **RTX 4090** for stages (a), (b) and (c); the **RTX 5070 Ti**
for Blackwell (display through GSP only); the **RTX 2080 Ti** (if
available) for Turing; the **GT 1030** and GTX 560 Ti only for safely
rejecting an unsupported card.

Blackwell in each path: nouveau has GB100/GB102 and GB202 to GB207 (GB203
= RTX 5070 Ti, display `gb202_disp` through GSP-RM), NVK supports consumer
Blackwell (Mesa 25.2), open-gpu-kernel-modules "Turing or later", nova
lists GB20x in its chip enum (`gpu.rs`) but has no display.

## Comparison: nouveau in C, or our own Rust driver for GSP

**Status: decided 2026-11-14 – variant B (our own Rust driver for GSP
only).** Turing to Blackwell, parts translated from nouveau (the MIT
licence allows a translation) and GSP boot from nova (GPL, so the driver lives in `gpl/`), fixed firmware
version 570.144, RPC definitions generated from nouveau's headers. Pascal
and older on the firmware framebuffer for now. LinuxKPI is postponed to
AMD and Intel 3D. Variant A (nouveau in C over LinuxKPI) stays recorded
as considered.

### What GSP does and what is left to the host

From Turing on, the GPU runs the GSP-RM firmware (NVIDIA's Resource
Manager). It takes over clocks, power, engine initialisation and channel
scheduling; the driver talks to it through RPC. Based on nouveau
(`nvkm/subdev/gsp`, `gsp/rm/r535`, `gsp/rm/r570`; 11 `.c` files in `gsp/`,
12 in `rm/`, 18 in `rm/r535/`, plus generated message definitions in
`nvrm/`):

| Part | What the driver must do | Where in nouveau | Rust estimate |
|---|---|---|---|
| GSP boot | find FWSEC in the VBIOS and run FWSEC-FRTS (WPR2 region), prepare WPR metadata, load `booter_load`, the `gsp` firmware and the bootloader, pass the libos buffer address | `gsp/fwsec.c`, `gsp/tu102.c`, `rm/r535/gsp.c` | 3–5 k lines |
| RPC queues | shared memory: command and message queues (`cmdq`/`msgq`, 256 KiB each), read and write pointers, message encoding, asynchronous events from GSP | `rm/r535/rpc.c`, `gsp.c` | 2–3 k |
| RM objects | object allocation (client, device, channel, ...) and control calls – messages with NVIDIA's numeric structures | `rm/r535/alloc.c`, `ctrl.c`, `client.c`, `device.c` + `nvrm/` | 2–4 k + definitions |
| GPU memory | GPU virtual address page tables, BAR1/BAR2, VRAM | `rm/r535/vmm.c`, `bar.c`, `subdev/mmu` | 4–6 k |
| channels | channel allocation through RM, push buffers, semaphores; GSP does the scheduling | `rm/r535/fifo.c`, `ce.c`, `gr.c` | 3–5 k |
| display | outputs, DP, HDMI, backlight through `NV0073_CTRL_*` control calls; the driver fills the display channel (methods) and reads its registers | `rm/r535/disp.c`, `engine/disp/tu102.c` to `gb202.c` | 6–10 k |
| suspend and recovery | saving VRAM on suspend (fbsr), reset after a GSP crash | `rm/r535/fbsr.c` | 1–2 k |
| uAPI for NVK | see below | `nouveau_drm.h`, `nouveau_uvmm.c`, `nouveau_exec.c` | 4–6 k |

Total estimate (not measured): **25–40 thousand lines of Rust** + generated
RPC message definitions.

### RPC and firmware versions

- nouveau has GSP-RM interfaces for two firmware versions: `r535`
  (535.113.01) and `r570` (570.144), selected by the firmware available
  (tables in `gsp/tu102.c`). Message structures differ between versions;
  NVIDIA keeps no stable ABI.
- For us: **one fixed firmware version** (the newest supported by
  nouveau, 570.144 today, covering Turing to Blackwell). Generate the
  message definitions from nouveau's headers (MIT) into Rust with an
  `xtask` script; an "rm" layer with an interface like nouveau's
  `rm/rm.h`, so the next version is a new definition set plus small
  differences. Firmware and driver are updated together.

### What NVK needs

NVK (Mesa) talks to the kernel through the nouveau uAPI
(`include/uapi/drm/nouveau_drm.h`, MIT licence): `GETPARAM`,
`CHANNEL_ALLOC`/`FREE`, `VM_INIT` (must come first), `VM_BIND`, `EXEC`,
`GEM_NEW`, `GEM_INFO` (returns a `map_handle` for mapping),
`GEM_CPU_PREP`/`FINI`. Synchronisation goes through `drm_nouveau_sync`
with syncobj or timeline syncobj (no fence ioctl of its own).

Proposal for both variants: the render node ([point 8](#impact-on-the-foundations))
offers **exactly these semantics** as messages over a channel and a
"winsys" in Mesa replaces `ioctl` with messages; NVK logic stays
unchanged. Syncobj = our fence object (point 1), `map_handle` = the
buffer's VMO (point 2), `VM_BIND` = mapping into the GPU context's
virtual addresses. The exact layer inside Mesa is not verified (the Mesa
source was unreachable during the research).

### Comparing the variants

| | A: nouveau in C over LinuxKPI (considered) | **B: our own Rust for GSP only (chosen)** |
|---|---|---|
| generations | Kepler to Blackwell (Maxwell 2 to Volta slow) | Turing to Blackwell; Pascal and older firmware framebuffer only |
| our code | layer 15–30 k + reused C (hundreds of thousands) | 25–40 k of our own Rust |
| safety | C in an isolated process | Rust, smaller attack surface |
| updates | LTS import, patches only in the layer | port nouveau changes by hand; a new chip = work |
| LinuxKPI | now | only for AMD |
| NVK | nouveau uAPI through the render node | imitate the same uAPI |
| risk | size and maintenance of the layer | we fix bugs the nouveau community fixes |
| user's cards | 4090, 5070 Ti, 2080 Ti fully; GT 1030 slowly | 4090, 5070 Ti, 2080 Ti fully; GT 1030 firmware framebuffer only |

B was chosen: smaller and cleaner for Turing+ because GSP does most of the
work. The price: Pascal and older without acceleration, and we port
nouveau changes ourselves. Foundation points 1, 2, 4 and 8 apply
unchanged.

### Driver licence: MIT or GPL

Project licence policy (approved 2026-11-14): GPL code may be reused or
translated, but **only in the separate `gpl/` part** as a standalone
driver process under GPL-2.0-only that speaks our public protocol; never
in the kernel, libraries or services.

What nova (GPL-2.0, Linux master 2026-11-14) contains:

| Part | nova-core | nouveau |
|---|---|---|
| VBIOS (finding FWSEC in the PMU table) | yes (`vbios.rs`) | yes |
| Falcon, FWSEC, booter, SEC2, FSP | yes (`falcon/`, `firmware/`) | yes (FSP new chips only) |
| GSP boot, sequencer | yes (`gsp/boot.rs`, `sequencer.rs`) | yes |
| command and message queue | yes (`gsp/cmdq`) | yes |
| message definitions per firmware version | in progress ("GSP firmware abstraction") | yes (`rm/r535`, `rm/r570`) |
| MMU, BAR, VRAM | partly (`mm/`, VRAM allocator open) | yes |
| channels, GR, CE | no (open items) | yes |
| display | no (nor nova-drm) | yes (through RM) |
| uAPI for Vulkan | no (nova-drm 3 ioctls) | yes (`VM_BIND`, `EXEC`, ... for NVK) |

| | MIT (nouveau only) | **GPL-2.0-only (nova + nouveau)** |
|---|---|---|
| location | anywhere (driver, shared libraries) | only `gpl/`, a separate process |
| GSP boot and queues | translate from C | take ready Rust from nova (less work, estimate 5–8 k fewer lines to translate) |
| display, channels, MMU, NVK uAPI | translate from nouveau | the same (nouveau, MIT – allowed inside a GPL work) |
| chips after Blackwell | wait for nouveau | nova is upstream's direction for GSP (aim: supersede nouveau for GSP); take from both |
| sharing code with the rest of the system | freely | nothing from the driver may flow back into MIT parts; shared things (RPC definitions from nouveau headers, the display class trait) stay in MIT libraries |
| dependencies | driverkit | driverkit + a thin replacement of the Linux Rust `kernel::` API (PCI, DMA, IO, firmware, devres) nova builds on |
| binary licence | MIT | GPL-2.0-only; the source is public (the project is open source), our MIT libraries stay MIT inside it |
| firmware | unchanged (NVIDIA licence for OSI systems) | unchanged |

**Decision in the design: GPL-2.0-only in `gpl/drivers/gpu-nvidia`.** Main
reasons: GSP boot and queues are done in Rust in nova (same language,
abstractions similar to driverkit), and nova is upstream's long-term
direction for GSP cards – new generations will appear there first. The
rest (display, channels, MMU, NVK uAPI) is translated from nouveau
anyway.

Consequences and rules:

- the driver is **one process in `gpl/`**, only our protocols leave it
  (display class, render class, fences, shared buffers) – the same as in
  the MIT variant, so the decision can be reversed later without changing
  the rest of the system;
- **nothing from `gpl/` is linked into MIT parts**; the build (`xtask`)
  enforces it (impact on the foundations, point 12);
- RPC message definitions are generated **from nouveau's headers (MIT)**
  into an MIT library – usable outside `gpl/` as well;
- verify which firmware version nova's GSP boot supports against the
  chosen 570.144 (VBIOS boot and booter are nearly version-independent,
  RPC messages are not);
- every reused file carries its origin and licence in its header and in
  `THIRD-PARTY.md`.

## AMD (second step)

| Part | Licence (verified in headers) |
|---|---|
| `amdgpu` (`amdgpu_device.c`) | MIT text |
| display core (`dc/core/dc.c`, `amdgpu_dm.c`) | MIT text, `amdgpu_dm.c` SPDX MIT |
| power management (`pm/swsmu/amdgpu_smu.c`) | MIT text |

amdgpu goes into the same compatibility layer as nouveau (same DRM core,
TTM, scheduler). It additionally needs firmware for almost everything
(PSP, SMU, DMCUB for display, VCN for video, graphics and SDMA microcode).
Vulkan: RADV from Mesa. The generation range (GCN 1.2+ and RDNA by
default, older experimentally) and the firmware licence are to be
verified when AMD work starts (the firmware sources were unreachable
during the research).

## Licences and what not to reuse

The reused DRM code is MIT or dual MIT/GPL with few exceptions. Verified
headers:

| File | Licence | Action |
|---|---|---|
| `drm_gem.c`, `drm_atomic.c`, `drm_syncobj.c`, `scheduler/sched_main.c` | MIT text | reuse |
| `ttm/ttm_bo.c`, `drm_exec.c` | GPL-2.0 OR MIT | reuse under MIT |
| `drm_gpuvm.c` | GPL-2.0-only OR MIT | reuse under MIT |
| `dma-buf/dma-resv.c` | MIT | reuse |
| **`dma-buf/dma-fence.c`** | **GPL-2.0-only** | not outside `gpl/`; allowed in a driver under `gpl/`, but kernel fences are ours anyway – `dma_fence_*` must still forward state to our fence object |
| `include/linux/*.h` (kernel headers) | mostly GPL | do not reuse: our own LinuxKPI headers (model FreeBSD, BSD-2) |

Rule: **check the licence of every file before reusing it**. GPL-only
files may go only into a driver process in `gpl/` (the whole process is
then GPL-2.0-only); outside it, our own implementation from the API and
documentation. The import script keeps the list automatically (SPDX
header / MIT text).

### Firmware licences

Verified in a copy of linux-firmware (GitHub mirror, 2026-11-14):

| Firmware | Licence | Redistribution | Conditions |
|---|---|---|---|
| NVIDIA (`LICENCE.nvidia`, "License For Customer Use of NVIDIA Software") | proprietary | only "for use on operating systems distributed under the terms of an OSI-approved open source license" – our system is MIT OR Apache-2.0, so yes | binaries unmodified, include the licence text; no reverse engineering, decompilation or translation; only for microprocessors designed by NVIDIA |
| AMD (`LICENSE.amdgpu`) | proprietary, free of fees | in binary form, yes | reproduce the copyright and licence text; no reverse engineering; US export regulations |

**Open:** the available copy of `WHENCE` has no entry for
`nvidia/<chip>/gsp/` (the copy is probably older). Before bundling, verify
in current linux-firmware that the GSP firmware falls under
`LICENCE.nvidia`. The image then carries the firmware unmodified in
`/system/firmware/` with the licences next to it; `THIRD-PARTY.md` records
the linux-firmware version.

## Strategy for AMD and Intel 3D

For AMD (and for Intel 3D acceleration, where we already have display in
Rust) an existing Linux DRM driver (amdgpu, possibly xe) is reused in C.
The LinuxKPI layer is **sealed inside the driver process**:

- only **our interfaces** leave the process: the display class, the render
  class (envelope: contexts, buffers, fences, quotas), fences and shared
  buffers (VMO + `libs/bufdesc`), the firmware service, the power protocol. No Linux API
  (kernel structures, ioctl numbers outside the render node payload,
  sysfs) crosses the process boundary;
- inside the render node the driver's uAPI travels as payload for Mesa
  (RADV, Intel ANV) – just like for NVK. A replacement driver must keep the
  same uAPI so Mesa stays unchanged;
- **the driver can therefore later be replaced by our own Rust driver
  without changing the rest of the system** (services, compositor,
  applications, Mesa).

**Licence (`gpl/` policy)**: with a reused C driver the AMD process is a
candidate for `gpl/` as well (amdgpu is MIT, but the layer could reuse
Linux's GPL infrastructure – `dma-fence`, helper libraries from `lib/` –
instead of our own implementation). Approach: the LinuxKPI base from
FreeBSD (BSD) and our own, Linux GPL code only where it saves real work;
then the whole AMD process lives in `gpl/` under GPL-2.0-only. Decide when
AMD work starts.

**The trend towards firmware**: like NVIDIA with GSP, AMD and Intel move
scheduling into firmware – AMD from RDNA 3 (GFX11) schedules queues with
the MES (Micro Engine Scheduler) microcode, Intel `xe` submits work only
through the GuC firmware (per Linux documentation and sources; not
verified in detail in this research). The host part for newer generations
shrinks, and our own Rust driver (as for NVIDIA) may become cheaper than
maintaining LinuxKPI. **Decision before AMD work starts**: compare our own
Rust for RDNA 3+ (MES) with amdgpu over LinuxKPI again.

**LinuxKPI maintenance cost**: Linux has no stable internal API – kernel
and DRM functions change with every version (recent years added e.g.
`drm_exec`, `drm_gpuvm`, changes to `dma-fence` and `dma-resv`). Therefore:

- track **one LTS version** of Linux and move to the next once a year in
  one big step (like FreeBSD drm-kmod, which follows Linux version by
  version with a patch set);
- import by a script: the list of Linux symbols used is generated and
  watched (a new or changed symbol = work in the layer, not in the
  driver), licence check of every file;
- host tests of the layer (locks, workqueues, DMA, fences) in the
  driverkit simulation;
- estimate: moving to a new LTS = weeks of layer work plus hardware tests
  (not measured); without it the driver ages and loses fixes and new chips.

## Compatibility layer (LinuxKPI)

Applies to AMD and Intel 3D ([strategy](#strategy-for-amd-and-intel-3d)),
not to NVIDIA.

A DRM driver expects Linux kernel services. The layer is ours (Rust +
thin C headers), modelled on FreeBSD `sys/compat/linuxkpi` (BSD), from
which code can also be reused where it fits.

| Group | What the driver calls | What it maps to here |
|---|---|---|
| memory | `kmalloc`, `vmalloc`, `alloc_pages`, `kref`, `idr`/`xarray`, lists, rb-tree | process allocator (`cstd`), VMO pages |
| DMA | `dma_alloc_coherent`, `dma_map_sg`, `dma_map_page`, masks | scatter-gather DMA + device IOMMU domain |
| MMIO | `ioremap`, `ioremap_wc`, `readl`/`writel` | `Platform::map_bar`, WC cache type |
| interrupts | `request_irq`, MSI/MSI-X, `tasklet` | `Platform::enable_irq`, interrupt thread |
| concurrency | spinlock, mutex, rwsem, RCU, atomics, `completion`, `wait_queue` | `cstd` locks, futex/event |
| work | `workqueue`, `kthread`, `timer`, `hrtimer`, `jiffies`, `msleep` | process threads, libos timers |
| PCI | configuration, BARs, `pci_enable_device`, D0/D3 power | configuration page + power service |
| firmware | `request_firmware` | **firmware service** |
| i2c/DDC | `i2c_adapter`, `i2c_transfer` | inside the driver (nouveau/amdgpu have their own) |
| synchronisation | `dma_fence`, `sync_file`, `dma_resv`, `dma_buf` | **our fences and buffers (VMO + `libs/bufdesc`)** |
| debugging | `printk`, `dev_*`, `WARN_ON`, debugfs | driverkit log; no debugfs |

Size estimate (not measured): layer 15–30 thousand lines (FreeBSD
linuxkpi is in that range), reused DRM core + TTM + scheduler tens of
thousands of lines, nouveau hundreds of thousands, amdgpu millions (mostly
generated register headers).

**Upstream updates**: track one Linux LTS version (like drm-kmod), import
by a script (`xtask`): fetch the tag, check licences, apply our patch set
(patches only in the layer and the build, not in driver logic), build,
host tests of the layer, hardware test. Move to a new LTS once a year.

## Stages

| Stage | Content | Prerequisites |
|---|---|---|
| (a) display | display class: GSP boot, takeover from GOP, modes from EDID, flips, hotplug, multiple monitors (through RM on Turing+) | GSP core (boot, RPC r570, RM objects), firmware service, IOMMU |
| (b) 2D/copy | buffers in GPU memory for the compositor, copy engine, zero-copy frames | shared buffers, fences |
| (c) Vulkan | NVK/RADV in applications, render class, render service, quotas | Mesa port (languages/libc thread), render node |
| (d) power and thermals | runtime PM (D3cold, Optimus), temperatures and fans into the power service | ACPI `_DSM`/`_PR3` via acpid |

Stage (a) gives a display on Turing to Blackwell cards. Compositor
acceleration (b) pays off even without Vulkan. The same stages for AMD
over LinuxKPI.

## IOMMU

The GPU is the most powerful DMA device in the computer and the driver is
hundreds of thousands of lines of reused C plus firmware (GSP is a
processor of its own with memory access). **Without an IOMMU a bug in the
driver or firmware can overwrite any memory, including the kernel.**

Decision (2026-11-14, consistent with disks and network):

- with an IOMMU: the device has its own domain (already today), DMA only
  into memory the driver maps for it; after a crash devmgr turns bus
  master off and drops the domain;
- **without an IOMMU the GPU driver starts with a warning**: a message in
  the log and in system notifications that DMA is unrestricted, with the
  advice to enable VT-d (Intel) or AMD-Vi / IOMMU (AMD) in the firmware.
  After a driver crash the kernel's **DMA quarantine** applies (turn bus
  master off, wait for transfers to finish, only then release the memory
  – [Kernel](kernel.md));
- turn interrupt remapping on (off today) before a GPU runs (a DMA device
  could otherwise forge an MSI).

## Testing

QEMU emulates neither NVIDIA nor AMD graphics. Options:

1. **Real machine** (main): image on a USB stick, log over a serial line
   or to a file ([Display configuration](display.md#testing-on-a-real-machine)).
2. **Passing the card to a VM (VFIO)**: needs Linux as the host with an
   IOMMU; on Windows (Hyper-V DDA) only Windows Server. Alternative: a
   second machine with Linux and QEMU + VFIO; faster cycle (no reboot).
3. **driverkit simulation**: host tests of the compatibility layer (locks,
   workqueues, DMA, fences) and replaying MMIO recordings from real runs
   (recorded from our own driver, not from foreign code).

Hardware: see [Test cards](#test-cards-and-what-to-verify-on-them) – an
RTX 4090 (Turing+ with GSP) and a GT 1030 (Pascal without GSP) are
available. For AMD later one RDNA 2 or newer card (the user has none).

## Impact on the foundations

What is worth changing **now** (independently of the driver itself) so
that the GPU does not require changes to finished parts. Size: S = days,
M = 1–2 weeks, L = more.

| # | Area | Change | Why now | Size |
|---|---|---|---|---|
| 1 | **fence object** | kernel object: state (pending, signalled, error), timeline (64-bit value), waitable in `wait_many`, passed by handle, signalled only by the holder of the `SIGNAL` right; **[verified in QEMU]** step 6a-1 ([fences.md](fences.md)) | `dma-fence.c` is GPL – we need our own; also the basis for Vulkan semaphores, `sync_file`, `drm_syncobj` and explicit sync in the display protocol; Venus has none today either (fences only inside the driver) | M |
| 2 | **shared buffer** | **decided (6a): a VMO + a description in the `libs/bufdesc` library, not a new kernel object.** Description: format (fourcc) + modifier (tiling, compression), planes `{offset, pitch}`, width and height, placement (system / device memory), cache type. Who may write is decided by the rights of the VMO handle (`READ`/`MAP`, the writer also `WRITE`). The receiver validates the description against the VMO size (untrusted input). Explicit synchronisation only: acquire and release fences in protocol messages (point 1), no implicit `dma_resv`. Revocation: device memory is a physical VMO which the driver revokes with `VMO_REVOKE`; system memory is held by handles. Linux's `dma-buf` is an object mainly because of implicit fencing and device attachment; we cover both with fences in messages and `DMA_MAP` over a VMO (point 4) **[designed]** | the `dma-buf` equivalent; device interfaces already require "the whole buffer lifecycle" (point 5) – define it once now for display, Venus and future GPUs | M |
| 3 | **display class v2** | optional: a client buffer as a frame without a copy (with acquire and release fences), formats and modifiers, multiple planes (cursor, overlay), atomic commit (mode + buffers at once), full timings in `SET_MODE` | today's protocol copies the shadow; GPU drivers and Intel can scan a buffer out directly – add it now as capabilities so messages do not change later | S–M |
| 4 | **scatter-gather DMA and IOMMU** | large pinned allocations page by page (not only contiguous 4 MiB), mapping a VMO range into the device domain at an IOVA chosen by the driver, unmapping with a synchronous IOMMU TLB flush, DMA quotas in GB | GPUs map hundreds of MB to GB (TTM, GTT); today's `dma_memory` is contiguous and small | M |
| 5 | **interrupt remapping** | enable in the kernel (VT-d IR, AMD-Vi; GICv3 ITS exists) | without it a DMA device can forge MSIs | M |
| 6 | **firmware service** | `request_firmware` as a service: files from `/system/firmware`, checksums, licence metadata, read-only VMO hand-over; image and installer bundle the firmware | GSP-RM (tens of MB), AMD, Wi-Fi, Realtek NICs (`rtl_fw`), future audio; nothing like it exists today | S–M |
| 7 | **GPU memory and budgets** | job budgets also for device memory (VRAM, GTT); objects the driver may evict (TTM) and pin again; VRAM mapped through the BAR as WC (cache type exists) | per-application Vulkan quotas; fairness between applications | M |
| 8 | **render class and render service** | a general render node protocol (from Venus: context, buffer, `VM_BIND`, submit with fences, queries) + a service that hands render nodes out by manifest and enforces quotas | applications must not get a capability to the driver; Venus hands its render node out through init today – unify before another driver appears | M |
| 9 | **runtime for reused C** | threads in the driver process (kthread, workqueue), precise timers, waits with timeouts; `cstd` with threads | the DRM core and amdgpu have their own threads and timers; Wi-Fi needs the same. The Rust NVIDIA driver does not need it – only with AMD | M |
| 10 | **device power** | a protocol for device power states (D0/D3hot/D3cold) through the power service; ACPI `_DSM`/`_PRx` calls from the driver through acpid | Optimus laptops turn the dGPU off; otherwise battery drain | S–M |
| 12 | **separate `gpl/` part** | build programs from `gpl/` separately (GPL-2.0-only licence in the manifest and the initfs), check that no MIT program or library depends on `gpl/`, licence in the `THIRD-PARTY.md` listing | policy approved 2026-11-14; the NVIDIA driver will be the first program in `gpl/` | S |
| 11 | **GPU recovery after a crash** | devmgr: PCI function reset (FLR / bus reset) before a new instance; the compositor survives without a display (it already does) | after a mode set there is no way back to GOP; recovery only by a reset | S |

I recommend starting points 1, 2, 4 and 8 right away: they touch the
kernel and protocols already used by the display and Venus, and later
they would change in more places.

## Risks

- **NVIDIA size**: 25–40 thousand lines of our own Rust; nouveau changes,
  fixes and new chips are ported by hand; GSP firmware and driver must be
  updated together (fixed version 570.144).
- **AMD size**: hundreds of thousands of lines of reused C and LinuxKPI
  maintenance (Linux has no stable internal API) – see
  [strategy](#strategy-for-amd-and-intel-3d).
- **Pascal and older** (GT 1030) without acceleration, firmware framebuffer only.
- **NVIDIA and AMD firmware is closed** (binary; redistribution allowed,
  see [Firmware licences](#firmware-licences)). Turing+ runs nothing
  without it, not even display on Blackwell. The NVIDIA licence applies
  only to open-source systems – if the system licence changed, the
  firmware could not be bundled.
- **Without an IOMMU** (decided: start with a warning) a bug in the driver
  or firmware can overwrite kernel memory; DMA quarantine protects only
  after a crash, not while running.
- **Optimus laptops**: the integrated Intel drives the display; NVIDIA only
  computes – requires shared buffers between drivers (point 2) and power
  (point 10).
- **Tests on real hardware only**: slower cycle, needs a serial line or a
  log on disk.
- **GSP firmware crashes**: recovery only by a device reset, briefly no image.
- **Licences can change** per file: licence check at every import.

## Decisions and questions

| # | Question | Decision (2026-11-14) |
|---|---|---|
| 1 | NVIDIA path | **B**: our own Rust for GSP only (Turing to Blackwell), firmware 570.144, RPC generated from nouveau's headers; Pascal on the firmware framebuffer; LinuxKPI only for AMD and Intel 3D ([comparison](#comparison-nouveau-in-c-or-our-own-rust-driver-for-gsp)) |
| 2 | without an IOMMU | **B**: start with a warning, DMA quarantine, advice to enable VT-d / AMD-Vi |
| 3 | firmware | **A**: bundle into the image after verifying the licence (verified; the GSP entry in `WHENCE` remains) |
| 4 | foundations | points 1, 2, 4, 8 as kernel work **after dynamic linking and before FFmpeg** (video uses fences and shared buffers too) |
| 5 | NVIDIA driver licence | **GPL-2.0-only in `gpl/`** (nova + nouveau), decided in the design under the `gpl/` policy ([driver licence](#driver-licence-mit-or-gpl)) |
